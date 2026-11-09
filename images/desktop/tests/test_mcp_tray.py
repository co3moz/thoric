"""The MCP servers tray of the desktop images, without its window: the command line it runs for a
form is one thoric-mcp accepts and turns back into the same declaration, the rows it shows
follow which declaration Thoric uses, and the files the images install name each other."""

import contextlib
import importlib.machinery
import importlib.util
import io
import json
import os
from pathlib import Path
import shlex
import shutil
import tempfile
import types
import unittest

ROOT = Path(__file__).resolve().parents[3]
DESKTOP = ROOT / "images/desktop"


def load(name, path):
    loader = importlib.machinery.SourceFileLoader(name, str(path))
    spec = importlib.util.spec_from_loader(name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


tray = load("thoric_mcp_tray", DESKTOP / "thoric-mcp-tray.py")
command = load("thoric_mcp_for_tray", DESKTOP / "thoric-mcp.py")

PROGRAM = {"description": "Browser automation", "command": "npx",
           "args": ["-y", "@playwright/mcp@latest", "--title", "two words"],
           "env": {"PORT": "9", "TOKEN": "${API_TOKEN}", "EQUALS": "a=b"}, "cwd": "/home/desktop/work"}
HTTP = {"url": "http://127.0.0.1:${PORT}/mcp", "headers": {"Authorization": "Bearer ${API_TOKEN}"}}


class CommandLineTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp()).resolve()
        self.addCleanup(shutil.rmtree, self.root)
        self.target = self.root / "home/desktop/.config/thoric/mcp"

    def saved(self, argv):
        """What thoric-mcp writes for the command line the tray would run."""
        self.assertEqual(argv[0], "thoric-mcp")
        with contextlib.redirect_stdout(io.StringIO()):
            command.main(argv[1:], image=str(self.root / "image"), target=str(self.target), uid=1001,
                         status_path=str(self.root / "status.json"))
        return json.loads((self.target / (argv[2] + ".json")).read_text())

    def test_a_declaration_survives_the_form_and_the_command(self):
        for spec in (PROGRAM, HTTP, {"command": "node"}):
            argv = tray.add_command(tray.form_of("x", spec))
            self.assertEqual(self.saved(argv), spec)
            # The shown command line is the same command as a shell reads it.
            self.assertEqual(shlex.split(tray.command_text(argv)), argv)

    def test_a_shown_command_line_is_read_back_as_it_was_and_stays_readable(self):
        tricky = ["the desktop's Chrome", "Bearer ${API_TOKEN}", "it's $HOME", 'say "hi"', "it's!",
                  "a\\b's", "plain", "", "two words"]
        self.assertEqual(shlex.split(tray.command_text(tricky)), tricky)
        self.assertEqual(tray.quoted("the desktop's Chrome"), '"the desktop\'s Chrome"')
        self.assertEqual(tray.quoted("Bearer ${API_TOKEN}"), "'Bearer ${API_TOKEN}'")

    def test_a_form_reads_as_a_terminal_would(self):
        argv = tray.add_command({"name": "pw", "kind": "command", "description": " Browser ",
                                 "command": "npx -y '@playwright/mcp' --title \"two words\"",
                                 "env": "\n  PORT=9  \n\nTOKEN=${API_TOKEN}\n", "cwd": " /home/desktop "})
        self.assertEqual(argv, ["thoric-mcp", "add", "pw", "--description", "Browser",
                                "--env", "PORT=9", "--env", "TOKEN=${API_TOKEN}", "--cwd", "/home/desktop",
                                "--", "npx", "-y", "@playwright/mcp", "--title", "two words"])
        argv = tray.add_command({"name": "api", "kind": "http", "url": "http://127.0.0.1:8931/mcp",
                                 "headers": "X-Test:yes\nAuthorization: Bearer a:b", "command": "ignored"})
        self.assertEqual(argv, ["thoric-mcp", "add", "api", "--url", "http://127.0.0.1:8931/mcp",
                                "--header", "X-Test: yes", "--header", "Authorization: Bearer a:b"])

    def test_a_form_that_cannot_be_a_command_says_what_to_fix(self):
        base = {"name": "x", "kind": "command", "command": "node"}
        for change, words in (({"name": "Bad"}, "lowercase"), ({"name": ""}, "lowercase"),
                              ({"command": "  "}, "command"), ({"command": "node 'open"}, "quote"),
                              ({"env": "NOEQUALS"}, "KEY=value"), ({"kind": "http"}, "address"),
                              ({"kind": "http", "url": "http://127.0.0.1:1/mcp", "headers": "nocolon"},
                               "Name: value")):
            with self.assertRaises(ValueError, msg=change) as raised:
                tray.add_command(dict(base, **change))
            self.assertIn(words, str(raised.exception))


class RowsTest(unittest.TestCase):
    LISTING = {"watched": True, "servers": [
        {"name": "unity", "source": "image", "active": True, "spec": HTTP,
         "status": {"state": "error", "error": "refused"}},
        {"name": "chrome", "source": "image", "active": False, "spec": {"command": "playwright-mcp"}},
        {"name": "chrome", "source": "container", "active": True, "spec": PROGRAM,
         "status": {"state": "ready", "tools": [{"name": "a"}, {"name": "b"}]}},
        {"name": "broken", "source": "container", "active": True, "error": "Cannot read broken.json"},
        {"name": "new", "source": "container", "active": True, "spec": {"command": "node"}},
    ]}

    def test_rows_follow_the_declaration_thoric_uses(self):
        rows = tray.effective(self.LISTING)
        self.assertEqual([(row["name"], row["source"]) for row in rows],
                         [("broken", "container"), ("chrome", "container"), ("new", "container"),
                          ("unity", "image")])
        self.assertEqual(rows[1]["image"]["spec"], {"command": "playwright-mcp"})
        self.assertNotIn("image", rows[3])

    def test_states_and_the_summary_say_how_servers_are_doing(self):
        rows = {row["name"]: row for row in tray.effective(self.LISTING)}
        self.assertEqual(tray.state(rows["chrome"], True), ("ready", "Ready, 2 tools"))
        self.assertEqual(tray.state(rows["unity"], True), ("error", "Error"))
        self.assertEqual(tray.state(rows["broken"], True), ("error", "Unreadable"))
        self.assertEqual(tray.state(rows["new"], True), ("pending", "Waiting for Thoric"))
        self.assertEqual(tray.state(rows["new"], False), ("unknown", "Not checked"))
        # An instance of a scale set that runs the server in instance 1 alone never starts it.
        elsewhere = dict(rows["new"], status={"state": "elsewhere", "runs_in": 1, "tools": []})
        self.assertEqual(tray.state(elsewhere, True), ("elsewhere", "Runs in instance 1"))
        self.assertEqual(tray.summary(list(rows.values()), True), "4 MCP servers, 1 ready, 2 with errors")
        self.assertEqual(tray.summary([], False), "No MCP servers")
        self.assertEqual(tray.summary([rows["new"]], False), "1 MCP server")

    def test_an_endpoint_is_described_without_its_token(self):
        lines = tray.endpoint_lines({"scopes": ["view", "files_read"], "servers": ["chrome"],
                                     "excluded_tools": ["x"], "has_token": False})
        self.assertEqual(lines, ["Container tools: View, Read files", "Servers: chrome", "Left out: 1 tool",
                                 "No token yet: only the administrator's MCP token opens it"])
        self.assertIn("All desktop servers", tray.endpoint_lines({"servers": "all", "has_token": True}))
        # A tool profile is named unless it is the full one, which an older manager does not send.
        narrowed = tray.endpoint_lines({"scopes": ["view"], "profile": "headless", "servers": "all",
                                        "has_token": True})
        self.assertEqual(narrowed[:2], ["Container tools: View", "Tool profile: Headless"])
        self.assertFalse(any("profile" in line for line in tray.endpoint_lines(
            {"scopes": ["view"], "profile": "full", "servers": "all", "has_token": True})))

    def test_the_listing_comes_from_the_command(self):
        def run(argv, **_options):
            self.assertEqual(argv, ["thoric-mcp", "list", "--json"])
            return types.SimpleNamespace(returncode=0, stdout=json.dumps(self.LISTING), stderr="")
        self.assertEqual(tray.load(run), self.LISTING)
        failed = lambda *_a, **_o: types.SimpleNamespace(returncode=1, stdout="", stderr="no such file")
        with self.assertRaisesRegex(ValueError, "no such file"):
            tray.load(failed)

    def test_the_signature_changes_with_a_declaration_or_a_report(self):
        root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, root)
        paths = (str(root / "image"), str(root / "status"))
        before = tray.signature(paths)
        (root / "status").mkdir()
        (root / "status/mcp-status.json").write_text("{}")
        self.assertNotEqual(tray.signature(paths), before)


class InstalledFilesTest(unittest.TestCase):
    """The Dockerfile, the desktop entries and the icons name the same files."""

    def entry(self, name):
        text = (DESKTOP / "mcp-tray" / name).read_text()
        return dict(line.split("=", 1) for line in text.splitlines()[1:] if "=" in line)

    def test_entries_run_the_installed_tray_with_installed_icons(self):
        dockerfile = (DESKTOP / "Dockerfile").read_text()
        self.assertIn("COPY thoric-mcp-tray.py /usr/local/bin/thoric-mcp-tray", dockerfile)
        self.assertIn("gir1.2-ayatanaappindicator3-0.1", dockerfile)
        for stage in ("FROM xfce-packages AS xfce", "FROM gnome-packages AS gnome"):
            section = dockerfile.split(stage, 1)[1].split("\nFROM ", 1)[0]
            self.assertIn("/etc/xdg/autostart/thoric-mcp-tray.desktop", section)
            self.assertIn("/usr/local/share/icons/", section)
        menu, autostart = self.entry("thoric-mcp.desktop"), self.entry("thoric-mcp-tray.desktop")
        self.assertEqual(menu["Exec"], "thoric-mcp-tray --show")
        self.assertEqual(autostart["Exec"], "thoric-mcp-tray")
        icons = DESKTOP / "mcp-tray/icons"
        for icon in (menu["Icon"], autostart["Icon"], tray.ICON):
            self.assertTrue((icons / "scalable/apps" / (icon + ".svg")).is_file(), icon)
        for icon in (tray.TRAY_ICON, tray.TRAY_ICON_ERROR):
            self.assertTrue((icons / "symbolic/apps" / (icon + ".svg")).is_file(), icon)
        self.assertTrue(tray.TRAY_ICONS.endswith("/hicolor/symbolic/apps"))

    def test_the_icon_is_the_dashboards_mark(self):
        self.assertEqual((DESKTOP / "mcp-tray/icons/scalable/apps/thoric-mcp.svg").read_bytes(),
                         (ROOT / "core/web/public/mark.svg").read_bytes())

    def test_the_error_icon_marks_its_dot_for_the_panel_to_color(self):
        svg = (DESKTOP / "mcp-tray/icons/symbolic/apps/thoric-mcp-error-symbolic.svg").read_text()
        self.assertRegex(svg, r'<circle class="error"')

    def test_the_tray_watches_the_folders_the_command_reads(self):
        self.assertEqual(tray.WATCHED, (command.IMAGE, command.FOLDER, os.path.dirname(command.STATUS)))
        self.assertEqual(tray.NAME.pattern, command.NAME.pattern)


if __name__ == "__main__":
    unittest.main()
