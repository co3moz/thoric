"""The commands and variables the dashboard's script editor completes, held to what the desktop
image puts in a desktop. A command installed in /usr/local/bin, a variable the image sets or a
subcommand of thoric-mcp must be in core/web/src/lib/script-completions.json, offered or left
out on purpose, so a change here that the editor would not know about fails with the image's
own tests."""

import json
import re
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
DOCKERFILE = ROOT / "images/desktop/Dockerfile"
THORIC_MCP = ROOT / "images/desktop/thoric-mcp.py"
COMPLETIONS = json.loads((ROOT / "core/web/src/lib/script-completions.json").read_text())


def named(kind):
    """The names of a kind the editor offers or leaves out."""
    return {entry["name"] for entry in COMPLETIONS[kind]} | set(COMPLETIONS["unlisted"][kind])


class ScriptCompletionsTest(unittest.TestCase):
    def test_every_installed_command_is_known_to_the_editor(self):
        installed = set(re.findall(r"^COPY \S+ /usr/local/bin/(\S+)$", DOCKERFILE.read_text(), re.M))
        self.assertTrue(installed)
        self.assertEqual(installed - named("commands"), set())

    def test_every_variable_the_image_sets_is_known_to_the_editor(self):
        found = set()
        # An instruction continued on the next lines is one line.
        instructions = DOCKERFILE.read_text().replace("\\\n", " ")
        for line in re.findall(r"^ENV (.+)$", instructions, re.M):
            found |= set(re.findall(r"(?:^|\s)([A-Za-z_][A-Za-z0-9_]*)=", line))
        self.assertIn("DESKTOP_GPU", found)
        self.assertEqual(found - named("variables"), set())

    def test_every_subcommand_of_thoric_mcp_is_offered(self):
        parsers = set(re.findall(r'add_parser\("(\w+)"\)', THORIC_MCP.read_text()))
        command = next(entry for entry in COMPLETIONS["commands"] if entry["name"] == "thoric-mcp")
        offered = {subcommand["name"].split()[0] for subcommand in command["subcommands"]}
        self.assertEqual(parsers, offered)


if __name__ == "__main__":
    unittest.main()
