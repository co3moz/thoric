"""Read or set the desktop's clipboard as the desktop user, through GTK 3 on the desktop's display.

`get SELECTION LIMIT` prints the text of the CLIPBOARD or PRIMARY selection, cut at LIMIT bytes.
`set SELECTION LIMIT` reads UTF-8 text from stdin and leaves a process behind that owns the
selections and gives the text to every program that pastes it, until another program takes them:
an X11 selection lives only as long as its owner. It answers once that process owns them.

A refused request exits 1, a desktop image without GTK 3 exits 3, a selection's owner or the
owner process that does not answer in time exits 4, and a display or owner that fails exits 5.
"""
import json
import os
import select
import signal
import sys

# How long a read waits for the selection's owner to answer, and a set for its owner to take it.
WAIT = 10
# How often the owner checks whether another program took its selections.
CHECK_SECONDS = 2
SELECTIONS = {"clipboard": ("CLIPBOARD",), "primary": ("PRIMARY",), "both": ("CLIPBOARD", "PRIMARY")}
UNSUPPORTED = ("This desktop image has no GTK 3 for Python (python3-gi and gir1.2-gtk-3.0), which "
               "the clipboard needs; create the container from a current Thoric desktop image")


class Failure(Exception):
    """A failure that is not the request's, with the exit code that tells the manager whose."""
    code = 5


class Unsupported(Failure):
    code = 3


class TimedOut(Failure):
    code = 4


FAILURES = {kind.code: kind for kind in (Failure, Unsupported, TimedOut)}


def gtk():
    """GTK 3 on the desktop's display, which the MCP tray of both desktop images uses too."""
    os.environ["GDK_BACKEND"] = "x11"
    # The accessibility bridge would look for a session bus that a docker exec does not have.
    os.environ["NO_AT_BRIDGE"] = "1"
    try:
        import gi
        for name in ("Gdk", "GdkX11", "Gtk"):
            gi.require_version(name, "3.0")
        from gi.repository import Gdk, GdkX11, GLib, Gtk
    except (ImportError, ValueError) as error:
        raise Unsupported(UNSUPPORTED) from error
    if Gdk.Display.get_default() is None:
        raise Failure("Could not open the desktop's display %s" % os.environ.get("DISPLAY", ""))
    return Gdk, GdkX11, GLib, Gtk


def cut(text, limit):
    """The text as at most limit bytes of UTF-8, never splitting a character, with its whole size."""
    data = text.encode("utf-8")
    if len(data) <= limit:
        return text, len(data), False
    return data[:limit].decode("utf-8", "ignore"), len(data), True


def read_text(stream, limit):
    """The text to set, from stdin: UTF-8 of at most limit bytes without NUL characters."""
    data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError("Clipboard text is limited to %d bytes of UTF-8" % limit)
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        raise ValueError("Clipboard text must be UTF-8") from None
    if "\0" in text:
        # GTK copies the text as a C string, which would end it at the first NUL.
        raise ValueError("Clipboard text cannot contain NUL characters")
    return text


def get(selection, limit):
    Gdk, _, GLib, Gtk = gtk()
    clipboard = Gtk.Clipboard.get(Gdk.Atom.intern(SELECTIONS[selection][0], False))
    loop, answer = GLib.MainLoop(), {}

    def received(_clipboard, text):
        answer["text"] = text
        loop.quit()

    def expired():
        loop.quit()
        return False

    clipboard.request_text(received)
    GLib.timeout_add_seconds(WAIT, expired)
    loop.run()
    if "text" not in answer:
        raise TimedOut("The program that holds the %s selection did not answer within %d seconds"
                           % (SELECTIONS[selection][0], WAIT))
    if answer["text"] is None:
        return {"selection": selection, "text": None, "bytes": 0, "truncated": False}
    text, size, truncated = cut(answer["text"], limit)
    return {"selection": selection, "text": text, "bytes": size, "truncated": truncated}


def own(selection, text, ready):
    """In the owner process: take the selections, call ready, then serve them until another
    program owns every one of them."""
    Gdk, GdkX11, GLib, Gtk = gtk()
    display = Gdk.Display.get_default()
    atoms = [Gdk.Atom.intern(name, False) for name in SELECTIONS[selection]]
    clipboards = [Gtk.Clipboard.get(atom) for atom in atoms]

    def owner(atom):
        # GDK wraps another program's window too once it reported that program as the owner, so
        # the owner is told apart by its X window.
        window = Gdk.selection_owner_get_for_display(display, atom)
        return None if window is None else GdkX11.X11Window.get_xid(window)

    for clipboard in clipboards:
        clipboard.set_text(text, -1)
    ours = {owner(atom) for atom in atoms}
    if None in ours:
        raise Failure("Could not take the %s selection" % " and ".join(SELECTIONS[selection]))

    def held():
        return [atom for atom in atoms if owner(atom) in ours]

    ready()

    def check(*_):
        if not held():
            Gtk.main_quit()
        return True

    for clipboard in clipboards:
        clipboard.connect("owner-change", check)
    # XFixes reports a new owner at once; the timer covers a display that does not.
    GLib.timeout_add_seconds(CHECK_SECONDS, check)
    Gtk.main()


def set_(selection, limit):
    text = read_text(sys.stdin.buffer, limit)
    read_end, write_end = os.pipe()
    pid = os.fork()
    if pid == 0:
        os.close(read_end)
        # The owner outlives this docker exec: its own session, and nothing of the exec's streams,
        # which would keep the call open until the owner ends.
        os.setsid()
        null = os.open(os.devnull, os.O_RDWR)
        for descriptor in (0, 1, 2):
            os.dup2(null, descriptor)
        os.close(null)
        reported = []

        def ready():
            os.write(write_end, b"owned\n")
            os.close(write_end)
            reported.append(True)

        code = 0
        try:
            own(selection, text, ready)
        except BaseException as error:
            code = error.code if isinstance(error, Failure) else Failure.code
            if not reported:
                try:
                    os.write(write_end, ("error %d %s\n" % (code, error)).encode("utf-8", "replace"))
                except OSError:
                    pass
        os._exit(code)
    os.close(write_end)
    answered, _, _ = select.select([read_end], [], [], WAIT)
    line = os.read(read_end, 65536).decode("utf-8", "replace").strip() if answered else ""
    os.close(read_end)
    if line != "owned":
        if not answered:
            os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)
        if not answered:
            raise TimedOut("The clipboard owner did not take the selection within %d seconds" % WAIT)
        if line.startswith("error "):
            code, _, message = line[len("error "):].partition(" ")
            raise FAILURES.get(int(code) if code.isdigit() else 0, Failure)(message)
        raise Failure("The clipboard owner ended before it took the selection")
    return {"selection": selection, "bytes": len(text.encode("utf-8")), "characters": len(text)}


def main(arguments):
    command, selection, limit = arguments[0], arguments[1], int(arguments[2])
    if selection not in SELECTIONS or (command == "get" and selection == "both"):
        raise ValueError("Unknown selection: %s" % selection)
    if command == "get":
        return get(selection, limit)
    if command == "set":
        return set_(selection, limit)
    raise ValueError("Unknown command: %s" % command)


if __name__ == "__main__":
    try:
        print(json.dumps(main(sys.argv[1:])))
    except Unsupported as error:
        print(error, file=sys.stderr)
        sys.exit(error.code)
    except Failure as error:
        print("Clipboard operation failed: %s" % error, file=sys.stderr)
        sys.exit(error.code)
    except Exception as error:
        print("Clipboard operation failed: %s" % error, file=sys.stderr)
        sys.exit(1)
