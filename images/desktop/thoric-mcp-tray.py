#!/usr/bin/env python3
"""This desktop's MCP servers in the panel's tray, and a window to see and change them.

The tray shows what `thoric-mcp list --json` reports: every server the image or the desktop
user declares, its state and tools as Thoric last read them, and the endpoints that reach this
desktop. It changes servers only by running `thoric-mcp add` and `remove`, and shows that
command line before it runs it, so whatever is done here can be done from a terminal or a script.
"""
import json
import os
import re
import shlex
import subprocess
import sys

COMMAND = "thoric-mcp"
# The declarations and the manager's report; a change in any of these folders is read again.
WATCHED = ("/etc/thoric/mcp.d", "/home/desktop/.config/thoric/mcp", "/run/desktop/thoric")
NAME = re.compile(r"^[a-z0-9][a-z0-9-]{0,31}$")
APPLICATION_ID = "io.github.co3moz.Thoric.McpServers"
ICON = "thoric-mcp"
TRAY_ICON = "thoric-mcp-symbolic"
TRAY_ICON_ERROR = "thoric-mcp-error-symbolic"
TRAY_ICONS = "/usr/local/share/icons/hicolor/symbolic/apps"
POLL_SECONDS = 2
SCOPES = {"view": "View", "input": "Input", "applications": "Applications", "files_read": "Read files",
          "files_write": "Write files", "commands": "Commands", "media": "Media", "power": "Power"}
CSS = b"""
.mcp-name { font-weight: bold; }
.mcp-tag { font-size: smaller; }
.mcp-ready { color: #2ec27e; font-weight: bold; }
.mcp-pending { color: #e5a50a; font-weight: bold; }
.mcp-error { color: #e01b24; font-weight: bold; }
.mcp-mono { font-family: monospace; }
.mcp-heading { font-weight: bold; font-size: larger; }
"""

try:
    import gi
    gi.require_version("Gdk", "3.0")
    gi.require_version("Gtk", "3.0")
    from gi.repository import Gdk, Gio, GLib, Gtk, Pango
except (ImportError, ValueError):
    # The unit tests use the functions below without GTK.
    Gtk = None


def signature(paths=WATCHED):
    """What changes when a declaration or the manager's report is written or removed."""
    entries = []
    for path in paths:
        try:
            names = sorted(os.listdir(path))
        except OSError:
            entries.append((path, None))
            continue
        for name in names:
            try:
                info = os.lstat(os.path.join(path, name))
            except OSError:
                continue
            entries.append((path, name, info.st_mtime_ns, info.st_size, info.st_ino))
    return entries


def load(run=subprocess.run):
    """What `thoric-mcp list --json` reports."""
    result = run([COMMAND, "list", "--json"], capture_output=True, text=True, timeout=30)
    if result.returncode != 0:
        raise ValueError(result.stderr.strip() or COMMAND + " list failed")
    document = json.loads(result.stdout)
    if not isinstance(document, dict):
        raise ValueError(COMMAND + " list printed something else than a listing")
    return document


def effective(document):
    """One row per server, as Thoric uses it: the desktop user's declaration of a name
    replaces the image's, which the row keeps as `image`."""
    entries = [entry for entry in document.get("servers") or [] if isinstance(entry, dict)]
    images = {entry.get("name"): entry for entry in entries if entry.get("source") == "image"}
    rows = []
    for entry in entries:
        if not entry.get("active"):
            continue
        row = dict(entry)
        if entry.get("source") == "container" and entry.get("name") in images:
            row["image"] = images[entry["name"]]
        rows.append(row)
    rows.sort(key=lambda row: row.get("name", ""))
    return rows


def state(row, watched):
    """How a server is doing, as a kind (ready, pending, error, elsewhere or unknown) and a few
    words. A server is elsewhere in an instance of a scale set that runs it in another instance
    alone, for all of them."""
    status = row.get("status") or {}
    kind = status.get("state")
    if kind == "ready":
        count = len(status.get("tools") or [])
        return "ready", "Ready, %d tool%s" % (count, "" if count == 1 else "s")
    if kind == "pending":
        return "pending", "Reading its tools"
    if kind == "elsewhere":
        return "elsewhere", "Runs in instance %s" % (status.get("runs_in") or 1)
    if kind == "error":
        return "error", "Error"
    if "error" in row:
        return "error", "Unreadable"
    if watched:
        return "pending", "Waiting for Thoric"
    return "unknown", "Not checked"


def summary(rows, watched):
    """The tray's first line: how many servers there are and how they are doing."""
    if not rows:
        return "No MCP servers"
    kinds = [state(row, watched)[0] for row in rows]
    text = "%d MCP server%s" % (len(rows), "" if len(rows) == 1 else "s")
    if watched:
        text += ", %d ready" % kinds.count("ready")
    if "error" in kinds:
        text += ", %d with errors" % kinds.count("error")
    return text


def runs(spec):
    """What a server runs, in one line."""
    if spec.get("url"):
        return spec["url"]
    if spec.get("command"):
        return command_text([spec["command"]] + [str(arg) for arg in spec.get("args") or []])
    return ""


def lines(text):
    return [line.strip() for line in text.splitlines() if line.strip()]


def quoted(argument):
    """An argument as a shell reads it back: in double quotes when it holds a single quote and
    nothing a shell expands there, so a description such as the desktop's Chrome stays readable."""
    if "'" in argument and not re.search(r'[$`\\"!]', argument):
        return '"%s"' % argument
    return shlex.quote(argument)


def command_text(argv):
    """A command line to show, copy and paste into a terminal."""
    return " ".join(quoted(argument) for argument in argv)


def add_command(form):
    """The `thoric-mcp add` command line of a form, or a ValueError that says what to fix.
    The command checks the declaration again before it writes it."""
    name = form.get("name", "").strip()
    if not NAME.match(name):
        raise ValueError("A name is lowercase letters, digits and dashes, at most 32.")
    argv = [COMMAND, "add", name]
    description = form.get("description", "").strip()
    if description:
        argv += ["--description", description]
    if form.get("kind") == "http":
        url = form.get("url", "").strip()
        if not url:
            raise ValueError("Give the server's address, such as http://127.0.0.1:8931/mcp.")
        argv += ["--url", url]
        for line in lines(form.get("headers", "")):
            header, separator, value = line.partition(":")
            if not separator or not header.strip():
                raise ValueError("Write each header as Name: value, not " + line)
            argv += ["--header", "%s: %s" % (header.strip(), value.strip())]
        return argv
    try:
        program = shlex.split(form.get("command", ""))
    except ValueError:
        raise ValueError("The command has a quote that is not closed.")
    if not program:
        raise ValueError("Give the command that starts the server.")
    for line in lines(form.get("env", "")):
        key, separator, value = line.partition("=")
        if not separator or not key.strip():
            raise ValueError("Write each variable as KEY=value, not " + line)
        argv += ["--env", "%s=%s" % (key.strip(), value)]
    cwd = form.get("cwd", "").strip()
    if cwd:
        argv += ["--cwd", cwd]
    return argv + ["--"] + program


def form_of(name, spec):
    """A form filled in from a declaration, to edit it or to make one of the same name."""
    return {
        "name": name,
        "description": spec.get("description") or "",
        "kind": "http" if spec.get("url") else "command",
        "command": runs(spec) if spec.get("command") else "",
        "cwd": spec.get("cwd") or "",
        "env": "\n".join("%s=%s" % item for item in (spec.get("env") or {}).items()),
        "url": spec.get("url") or "",
        "headers": "\n".join("%s: %s" % item for item in (spec.get("headers") or {}).items()),
    }


def template_entry(name, spec):
    """A declaration as a template's or a build's `mcp` list holds it."""
    return json.dumps(dict({"name": name}, **spec), indent=2)


def endpoint_lines(endpoint):
    """What an endpoint gives an agent of this desktop, in short lines."""
    scopes = [SCOPES.get(scope, scope) for scope in endpoint.get("scopes") or []]
    servers = endpoint.get("servers")
    if servers == "all":
        servers = "All desktop servers"
    elif servers:
        servers = "Servers: " + ", ".join(servers)
    else:
        servers = "No desktop servers"
    result = ["Container tools: " + (", ".join(scopes) if scopes else "none"), servers]
    excluded = endpoint.get("excluded_tools") or []
    if excluded:
        result.append("Left out: %d tool%s" % (len(excluded), "" if len(excluded) == 1 else "s"))
    result.append("Has its own token" if endpoint.get("has_token") else
                  "No token yet: only the administrator's MCP token opens it")
    return result


def label(text="", css=(), wrap=False, selectable=False, markup=None):
    widget = Gtk.Label(xalign=0)
    if markup is not None:
        widget.set_markup(markup)
    else:
        widget.set_text(text)
    for name in css:
        widget.get_style_context().add_class(name)
    if wrap:
        widget.set_line_wrap(True)
        widget.set_line_wrap_mode(Pango.WrapMode.WORD_CHAR)
        # Without a width of its own a wrapping label asks for the height of one word a line,
        # which makes a window as tall as the screen.
        widget.set_width_chars(40)
        widget.set_max_width_chars(80)
    widget.set_selectable(selectable)
    return widget


def copy(text):
    Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).set_text(text, -1)


def headed():
    """Whether this desktop draws title bars inside windows, as GNOME does. XFCE does not, and
    without a compositor such a window gets black shadows."""
    return bool(Gtk.Settings.get_default().get_property("gtk-dialogs-use-header"))


class Editor:
    """The dialog that adds a server, edits the desktop user's or replaces the image's, by running
    the `thoric-mcp add` command line it shows."""

    def __init__(self, parent, form, title, notice="", fixed_name=False):
        # The desktop's gtk-dialogs-use-header setting places the buttons, as in its own dialogs.
        self.dialog = Gtk.Dialog(title=title, transient_for=parent, modal=True, use_header_bar=int(headed()))
        self.dialog.set_default_size(560, -1)
        self.dialog.add_button("Cancel", Gtk.ResponseType.CANCEL)
        self.save = self.dialog.add_button("Save", Gtk.ResponseType.OK)
        self.save.get_style_context().add_class("suggested-action")
        self.dialog.set_default_response(Gtk.ResponseType.OK)
        box = self.dialog.get_content_area()
        box.set_spacing(6)
        for side in ("start", "end", "top", "bottom"):
            getattr(box, "set_margin_" + side)(18)

        self.name = self.entry(form["name"], "playwright")
        self.name.set_sensitive(not fixed_name)
        self.description = self.entry(form["description"], "What its tools do, for the agent")
        box.pack_start(self.field("Name", self.name), False, False, 0)
        box.pack_start(self.field("Description", self.description), False, False, 0)

        self.stack = Gtk.Stack()
        self.stack.set_homogeneous(False)
        switcher = Gtk.StackSwitcher()
        switcher.set_stack(self.stack)
        switcher.set_halign(Gtk.Align.START)
        box.pack_start(switcher, False, False, 6)

        program = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        self.command = self.entry(form["command"], "npx -y @playwright/mcp@latest")
        program.pack_start(self.field("Command", self.command,
                                      "Type it as in a terminal; quotes keep a value with spaces together."),
                           False, False, 0)
        self.cwd = self.entry(form["cwd"], "/home/desktop")
        choose = Gtk.Button(label="Choose…")
        choose.connect("clicked", self.choose_folder)
        folder = Gtk.Box(spacing=6)
        folder.pack_start(self.cwd, True, True, 0)
        folder.pack_start(choose, False, False, 0)
        program.pack_start(self.field("Folder", folder, "Where it starts. Leave it empty for the home folder."),
                           False, False, 0)
        self.env, scrolled = self.text(form["env"])
        program.pack_start(self.field("Variables", scrolled,
                                      "One KEY=value a line. ${NAME} takes the container's compose value NAME."),
                           False, False, 0)
        self.stack.add_titled(program, "command", "Program")

        http = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        self.url = self.entry(form["url"], "http://127.0.0.1:8931/mcp")
        http.pack_start(self.field("Address", self.url,
                                   "A Streamable HTTP server listening inside this desktop."),
                        False, False, 0)
        self.headers, scrolled = self.text(form["headers"])
        http.pack_start(self.field("Headers", scrolled,
                                   "One Name: value a line. ${NAME} takes the container's compose value NAME."),
                        False, False, 0)
        self.stack.add_titled(http, "http", "HTTP server")
        box.pack_start(self.stack, False, False, 0)

        if notice:
            box.pack_start(label(notice, css=("dim-label",), wrap=True), False, False, 6)

        expander = Gtk.Expander(label="Command line")
        expander.set_expanded(True)
        shown = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        self.preview = label(css=("mcp-mono",), wrap=True, selectable=True)
        copy_button = Gtk.Button(label="Copy")
        copy_button.set_halign(Gtk.Align.START)
        copy_button.connect("clicked", lambda _button: copy(self.preview.get_text()))
        shown.pack_start(label("Saving runs this command. It does the same from a terminal or a script.",
                               css=("dim-label",), wrap=True), False, False, 0)
        shown.pack_start(self.preview, False, False, 0)
        shown.pack_start(copy_button, False, False, 0)
        expander.add(shown)
        box.pack_start(expander, False, False, 6)

        self.problem = label(css=("mcp-error",), wrap=True, selectable=True)
        self.problem.set_no_show_all(True)
        box.pack_start(self.problem, False, False, 0)

        for entry in (self.name, self.description, self.command, self.cwd, self.url):
            entry.connect("changed", self.update)
        for view in (self.env, self.headers):
            view.get_buffer().connect("changed", self.update)
        self.stack.connect("notify::visible-child-name", self.update)
        box.show_all()
        self.stack.set_visible_child_name(form["kind"])
        self.update()

    @staticmethod
    def entry(text, placeholder):
        entry = Gtk.Entry()
        entry.set_text(text)
        entry.set_placeholder_text(placeholder)
        entry.set_activates_default(True)
        return entry

    @staticmethod
    def text(text):
        """A text view in its scrolled window, both returned: a widget without a parent lives only
        as long as Python holds it."""
        view = Gtk.TextView()
        view.set_monospace(True)
        view.get_buffer().set_text(text)
        scrolled = Gtk.ScrolledWindow()
        scrolled.set_shadow_type(Gtk.ShadowType.IN)
        scrolled.set_min_content_height(64)
        scrolled.add(view)
        return view, scrolled

    @staticmethod
    def field(title, widget, hint=""):
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=3)
        box.pack_start(label(title, css=("mcp-name",)), False, False, 0)
        box.pack_start(widget, False, False, 0)
        if hint:
            box.pack_start(label(hint, css=("dim-label",), wrap=True), False, False, 0)
        return box

    def choose_folder(self, _button):
        chooser = Gtk.FileChooserNative.new("Choose the server's folder", self.dialog,
                                            Gtk.FileChooserAction.SELECT_FOLDER, "Choose", "Cancel")
        if self.cwd.get_text():
            chooser.set_current_folder(self.cwd.get_text())
        if chooser.run() == Gtk.ResponseType.ACCEPT:
            self.cwd.set_text(chooser.get_filename())
        chooser.destroy()

    def form(self):
        def value(view):
            buffer = view.get_buffer()
            return buffer.get_text(buffer.get_start_iter(), buffer.get_end_iter(), False)
        return {"name": self.name.get_text(), "description": self.description.get_text(),
                "kind": self.stack.get_visible_child_name() or "command", "command": self.command.get_text(),
                "cwd": self.cwd.get_text(), "env": value(self.env), "url": self.url.get_text(),
                "headers": value(self.headers)}

    def update(self, *_arguments):
        try:
            self.preview.set_text(command_text(add_command(self.form())))
            self.save.set_sensitive(True)
            self.show_problem("")
        except ValueError as error:
            self.preview.set_text(str(error))
            self.save.set_sensitive(False)

    def show_problem(self, text):
        self.problem.set_text(text)
        self.problem.set_visible(bool(text))

    def run(self):
        """Show the dialog until the command saves the server or the dialog is closed. Returns
        whether it saved."""
        try:
            while self.dialog.run() == Gtk.ResponseType.OK:
                try:
                    argv = add_command(self.form())
                except ValueError as error:
                    self.show_problem(str(error))
                    continue
                problem = execute(argv)
                if problem is None:
                    return True
                self.show_problem(problem)
            return False
        finally:
            self.dialog.destroy()


def execute(argv):
    """Run a thoric-mcp command line; None when it worked, or what it said went wrong."""
    try:
        result = subprocess.run(argv, capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.SubprocessError) as error:
        return str(error)
    if result.returncode == 0:
        return None
    return result.stderr.strip() or "%s ended with %d" % (argv[0], result.returncode)


class Window:
    """The servers of this desktop and the endpoints that reach it."""

    def __init__(self, tray):
        self.tray = tray
        self.expanded = set()
        self.window = Gtk.ApplicationWindow(application=tray.app)
        self.window.set_default_size(680, 640)
        self.window.set_icon_name(ICON)
        self.window.connect("delete-event", lambda window, _event: window.hide() or True)
        self.window.set_title("MCP servers")
        add = Gtk.Button(label="Add server")
        add.get_style_context().add_class("suggested-action")
        add.connect("clicked", lambda _button: tray.add())
        if headed():
            bar = Gtk.HeaderBar()
            bar.set_show_close_button(True)
            bar.set_title("MCP servers")
            bar.set_subtitle("This desktop's servers for Thoric")
            bar.pack_end(add)
            bar.show_all()
            self.window.set_titlebar(bar)

        page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        for side in ("start", "end", "top", "bottom"):
            getattr(page, "set_margin_" + side)(18)
        self.notice = Gtk.InfoBar()
        self.notice.set_message_type(Gtk.MessageType.WARNING)
        self.notice.get_content_area().add(label(
            "Thoric is not reading this desktop's servers right now, so their states are not "
            "known. MCP may be turned off, or the manager may be restarting. Changes are saved and "
            "read when it is back.", wrap=True))
        self.notice.set_no_show_all(True)
        page.pack_start(self.notice, False, False, 0)

        heading = Gtk.Box(spacing=12)
        heading.pack_start(label("Servers", css=("mcp-heading",)), False, False, 0)
        if not headed():
            heading.pack_end(add, False, False, 0)
        page.pack_start(heading, False, False, 0)
        page.pack_start(label("Thoric starts them as you when an agent needs their tools. Agents "
                              "reach them through the endpoints below.", css=("dim-label",), wrap=True),
                        False, False, 0)
        self.servers, frame = self.list_box("No MCP servers yet. Add one here, with thoric-mcp add "
                                            "in a terminal, or with a template that declares one.")
        self.servers.connect("row-activated", self.toggle)
        page.pack_start(frame, False, False, 0)

        page.pack_start(label("Endpoints that reach this desktop", css=("mcp-heading",)), False, False, 6)
        page.pack_start(label("An agent connects to /mcp/NAME at the address you open Thoric with. "
                              "Endpoints and their tokens are made on the dashboard's MCP page.",
                              css=("dim-label",), wrap=True), False, False, 0)
        self.endpoints, frame = self.list_box("")
        page.pack_start(frame, False, False, 0)

        scrolled = Gtk.ScrolledWindow()
        scrolled.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scrolled.add(page)
        self.window.add(scrolled)
        scrolled.show_all()

    @staticmethod
    def list_box(empty):
        box = Gtk.ListBox()
        box.set_selection_mode(Gtk.SelectionMode.NONE)
        placeholder = label(empty, css=("dim-label",), wrap=True)
        for side in ("start", "end", "top", "bottom"):
            getattr(placeholder, "set_margin_" + side)(12)
        placeholder.show()
        box.set_placeholder(placeholder)
        # GTK 3 has no getter for it.
        box.placeholder = placeholder
        frame = Gtk.Frame()
        frame.add(box)
        return box, frame

    def present(self, name=None):
        if name:
            self.expanded.add(name)
            self.fill()
        self.window.present()

    def toggle(self, _box, row):
        name = row.server_name
        if name in self.expanded:
            self.expanded.discard(name)
        else:
            self.expanded.add(name)
        row.details.set_reveal_child(name in self.expanded)

    def fill(self):
        document, rows, watched = self.tray.document, self.tray.rows, self.tray.watched
        self.notice.set_visible(not watched)
        for child in self.notice.get_content_area().get_children():
            child.show()
        for box in (self.servers, self.endpoints):
            for child in box.get_children():
                box.remove(child)
        for row in rows:
            self.servers.add(self.server_row(row, watched))
        endpoints = (document or {}).get("endpoints") or []
        self.endpoints.placeholder.set_text(
            "No endpoint reaches this desktop." if watched else
            "Not known while Thoric is not reading this desktop.")
        for endpoint in endpoints:
            self.endpoints.add(self.endpoint_row(endpoint))
        self.servers.show_all()
        self.endpoints.show_all()

    def server_row(self, row, watched):
        name = row.get("name", "")
        spec = row.get("spec") or {}
        status = row.get("status") or {}
        kind, text = state(row, watched)
        item = Gtk.ListBoxRow()
        item.server_name = name
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=3)
        for side in ("start", "end", "top", "bottom"):
            getattr(box, "set_margin_" + side)(10)
        head = Gtk.Box(spacing=8)
        head.pack_start(label(name, css=("mcp-name",)), False, False, 0)
        if row.get("image"):
            origin = "Image's, replaced here"
        elif row.get("source") == "image":
            origin = "Image"
        else:
            origin = "Added"
        head.pack_start(label(origin, css=("mcp-tag", "dim-label")), False, False, 0)
        head.pack_end(label(text, css=("mcp-" + kind,) if kind not in ("unknown", "elsewhere")
                            else ("dim-label",)),
                      False, False, 0)
        box.pack_start(head, False, False, 0)
        subtitle = label(spec.get("description") or runs(spec) or row.get("error", ""), css=("dim-label",))
        subtitle.set_ellipsize(Pango.EllipsizeMode.END)
        box.pack_start(subtitle, False, False, 0)

        details = Gtk.Grid(column_spacing=12, row_spacing=6)
        details.set_margin_top(8)
        line = 0

        def add(title, widget):
            nonlocal line
            heading = label(title, css=("dim-label",))
            heading.set_valign(Gtk.Align.START)
            details.attach(heading, 0, line, 1, 1)
            widget.set_hexpand(True)
            details.attach(widget, 1, line, 1, 1)
            line += 1

        if spec.get("url"):
            add("Address", label(spec["url"], css=("mcp-mono",), wrap=True, selectable=True))
            if spec.get("headers"):
                add("Headers", label(", ".join(spec["headers"]), wrap=True))
        elif spec.get("command"):
            add("Command", label(runs(spec), css=("mcp-mono",), wrap=True, selectable=True))
            if spec.get("cwd"):
                add("Folder", label(spec["cwd"], css=("mcp-mono",), selectable=True))
            if spec.get("env"):
                add("Variables", label(", ".join(spec["env"]), wrap=True))
        if row.get("source") == "image":
            where = "/etc/thoric/mcp.d/%s.json, from the image" % name
        else:
            where = "~/.config/thoric/mcp/%s.json" % name
        add("File", label(where, css=("mcp-mono",), wrap=True, selectable=True))
        if row.get("error"):
            add("Problem", label(row["error"], css=("mcp-error",), wrap=True, selectable=True))
        if status.get("error"):
            add("Error", label(status["error"], css=("mcp-error",), wrap=True, selectable=True))
        tools = status.get("tools") or []
        if tools:
            markup = "\n".join("<b>%s</b>  %s" % (GLib.markup_escape_text(tool.get("name", "")),
                                                  GLib.markup_escape_text(tool.get("description", "")))
                               for tool in tools)
            add("Tools", label(markup=markup, wrap=True, selectable=True))
        if status.get("log"):
            add("Output", label(status["log"], css=("mcp-mono",), wrap=True, selectable=True))

        actions = Gtk.Box(spacing=6)
        if row.get("source") == "container":
            edit = Gtk.Button(label="Edit")
            edit.connect("clicked", lambda _button: self.tray.edit(row))
            actions.pack_start(edit, False, False, 0)
            remove = Gtk.Button(label="Use the image's" if row.get("image") else "Remove")
            if not row.get("image"):
                remove.get_style_context().add_class("destructive-action")
            remove.connect("clicked", lambda _button: self.tray.remove(row))
            actions.pack_start(remove, False, False, 0)
        elif "spec" in row:
            replace = Gtk.Button(label="Change here")
            replace.set_tooltip_text("Save a copy of it in this desktop, which replaces the image's")
            replace.connect("clicked", lambda _button: self.tray.edit(row))
            actions.pack_start(replace, False, False, 0)
        if "spec" in row:
            as_command = Gtk.Button(label="Copy command")
            as_command.set_tooltip_text("The thoric-mcp add command that registers this server")
            as_command.connect("clicked", lambda _button: copy(command_text(add_command(form_of(name, spec)))))
            actions.pack_start(as_command, False, False, 0)
            as_template = Gtk.Button(label="Copy for a template")
            as_template.set_tooltip_text("The declaration as a template's mcp list holds it")
            as_template.connect("clicked", lambda _button: copy(template_entry(name, spec)))
            actions.pack_start(as_template, False, False, 0)
        details.attach(actions, 0, line, 2, 1)

        revealer = Gtk.Revealer()
        revealer.add(details)
        revealer.set_reveal_child(name in self.expanded)
        box.pack_start(revealer, False, False, 0)
        item.details = revealer
        item.add(box)
        return item

    @staticmethod
    def endpoint_row(endpoint):
        item = Gtk.ListBoxRow()
        item.set_activatable(False)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=3)
        for side in ("start", "end", "top", "bottom"):
            getattr(box, "set_margin_" + side)(10)
        head = Gtk.Box(spacing=8)
        head.pack_start(label(endpoint.get("name", ""), css=("mcp-name",)), False, False, 0)
        head.pack_start(label(endpoint.get("path", ""), css=("mcp-mono", "dim-label"), selectable=True),
                        False, False, 0)
        if endpoint.get("automatic"):
            head.pack_end(label("This container's own", css=("mcp-tag", "dim-label")), False, False, 0)
        box.pack_start(head, False, False, 0)
        for text in endpoint_lines(endpoint):
            box.pack_start(label(text, css=("dim-label",), wrap=True), False, False, 0)
        item.add(box)
        return item


class Tray:
    """The application: one per session, with the tray icon, its menu and the window."""

    def __init__(self):
        self.app = Gtk.Application(application_id=APPLICATION_ID,
                                   flags=Gio.ApplicationFlags.HANDLES_COMMAND_LINE)
        self.app.connect("startup", self.startup)
        self.app.connect("command-line", self.command_line)
        self.document = None
        self.rows = []
        self.watched = False
        self.failure = ""
        self.last = None
        self.indicator = None
        self.window = None

    def startup(self, app):
        app.hold()
        Gtk.Window.set_default_icon_name(ICON)
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), provider,
                                                 Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        self.menu = Gtk.Menu()
        try:
            gi.require_version("AyatanaAppIndicator3", "0.1")
            from gi.repository import AyatanaAppIndicator3 as indicator
        except (ImportError, ValueError) as error:
            sys.stderr.write("thoric-mcp-tray: no tray icon without AyatanaAppIndicator3: %s\n" % error)
        else:
            self.indicator = indicator.Indicator.new("thoric-mcp", TRAY_ICON,
                                                     indicator.IndicatorCategory.APPLICATION_STATUS)
            self.indicator.set_icon_theme_path(TRAY_ICONS)
            self.indicator.set_title("MCP servers")
            self.indicator.set_status(indicator.IndicatorStatus.ACTIVE)
            self.indicator.set_menu(self.menu)
        self.window = Window(self)
        self.poll()
        GLib.timeout_add_seconds(POLL_SECONDS, self.poll)

    def command_line(self, _app, command_line):
        arguments = command_line.get_arguments()[1:]
        if "--show" in arguments or "--add" in arguments:
            self.window.present()
        if "--add" in arguments:
            GLib.idle_add(self.add)
        return 0

    def poll(self):
        current = signature()
        if current != self.last:
            self.last = current
            self.refresh()
        return True

    def refresh(self):
        try:
            self.document = load()
            self.failure = ""
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            self.document = None
            self.failure = str(error)
        self.watched = bool(self.document and self.document.get("watched"))
        self.rows = effective(self.document or {})
        self.fill_menu()
        self.window.fill()

    def fill_menu(self):
        for child in self.menu.get_children():
            self.menu.remove(child)

        def item(text, action=None):
            entry = Gtk.MenuItem(label=text)
            if action is None:
                entry.set_sensitive(False)
            else:
                entry.connect("activate", lambda _entry: action())
            self.menu.append(entry)
            return entry

        item(self.failure[:80] or summary(self.rows, self.watched))
        for row in self.rows:
            name = row.get("name", "")
            item("%s: %s" % (name, state(row, self.watched)[1]),
                 lambda name=name: self.window.present(name))
        if not self.watched:
            item("Thoric is not reading them now")
        self.menu.append(Gtk.SeparatorMenuItem())
        manage = item("Manage MCP servers…", self.window.present)
        item("Add server…", self.add)
        self.menu.append(Gtk.SeparatorMenuItem())
        item("Quit", self.app.quit)
        self.menu.show_all()
        if self.indicator is not None:
            self.indicator.set_secondary_activate_target(manage)
            failing = any(state(row, self.watched)[0] == "error" for row in self.rows)
            self.indicator.set_icon_full(TRAY_ICON_ERROR if failing else TRAY_ICON,
                                         summary(self.rows, self.watched))

    def changed(self):
        self.last = None
        self.poll()

    def add(self):
        self.window.present()
        editor = Editor(self.window.window, form_of("", {}), "Add server")
        if editor.run():
            self.changed()
        return False

    def edit(self, row):
        name = row.get("name", "")
        if row.get("source") == "image":
            notice = ("Saving keeps a copy in this desktop, which Thoric uses instead of the "
                      "image's %s. Use the image's again removes the copy." % name)
            title = "Change %s here" % name
        else:
            notice = ""
            title = "Edit %s" % name
        editor = Editor(self.window.window, form_of(name, row.get("spec") or {}), title, notice,
                        fixed_name=True)
        if editor.run():
            self.changed()

    def remove(self, row):
        name = row.get("name", "")
        argv = [COMMAND, "remove", name]
        if row.get("image"):
            question = "Use the image's %s again?" % name
            detail = "Your copy is removed and the image's declaration is used again."
            button = "Use the image's"
        else:
            question = "Remove %s?" % name
            detail = "Agents lose its tools."
            button = "Remove"
        dialog = Gtk.MessageDialog(transient_for=self.window.window, modal=True,
                                   message_type=Gtk.MessageType.QUESTION, buttons=Gtk.ButtonsType.NONE,
                                   text=question)
        dialog.format_secondary_text("%s This runs: %s" % (detail, command_text(argv)))
        dialog.add_button("Cancel", Gtk.ResponseType.CANCEL)
        confirm = dialog.add_button(button, Gtk.ResponseType.OK)
        if not row.get("image"):
            confirm.get_style_context().add_class("destructive-action")
        answer = dialog.run()
        dialog.destroy()
        if answer != Gtk.ResponseType.OK:
            return
        problem = execute(argv)
        if problem is not None:
            error = Gtk.MessageDialog(transient_for=self.window.window, modal=True,
                                      message_type=Gtk.MessageType.ERROR, buttons=Gtk.ButtonsType.CLOSE,
                                      text="Could not remove %s" % name)
            error.format_secondary_text(problem)
            error.run()
            error.destroy()
        self.changed()


def main(argv):
    if Gtk is None:
        sys.stderr.write("thoric-mcp-tray needs GTK 3 and PyGObject; thoric-mcp does the same "
                         "from a terminal.\n")
        return 1
    GLib.set_prgname("thoric-mcp-tray")
    GLib.set_application_name("MCP servers")
    return Tray().app.run(argv)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
