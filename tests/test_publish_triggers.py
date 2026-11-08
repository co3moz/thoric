"""Which changes run .github/workflows/publish.yml. A push to master publishes images, so it runs only
for what they are built from, and a change to the documentation alone publishes nothing, except the
skills under docs/skills, which the manager builds in. A pull
request runs the checks, so it also runs for every file a test names: the Rust and interface tests
hold docs/API.md, docs/MCP.md and the README to the routes and tools they describe, and a change to
those alone could break them without any check running.

A test names a file with a string, from the root of the repository, as `read("docs/API.md")` and
`ROOT / "README.md"` do, or from its own folder when the string starts with ./ or ../, as
include_str! and `new URL(..., import.meta.url)` do. The scan reads every string of the crate, the
interface tests and the Python tests that way. A path a test joins from parts, such as a folder and
a file name, is not seen.
"""

import ast
import functools
import importlib.util
import os
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
# A quoted run without spaces, found at every quote, so a quote in a char literal such as '"' cannot
# swallow the string after it.
QUOTED = re.compile(r"""(?=(["'`])([^"'`\s]+)\1)""")
spec = importlib.util.spec_from_file_location("unit", ROOT / "tests/unit.py")
unit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(unit)


def trigger_paths(event):
    """The `paths` list of one event of the workflow, read from its lines, since the checks have no
    YAML library."""
    workflow = (ROOT / ".github/workflows/publish.yml").read_text(encoding="utf-8")
    block = re.search(rf"(?m)^  {event}:\n(?:    .*\n)*?    paths:\n((?:      - .*\n)+)", workflow)
    assert block, f"the {event} trigger has no written-out paths list"
    paths = re.findall(r"(?m)^      - '([^']+)'$", block.group(1))
    for path in paths:
        # GitHub reads ?, +, [ and ! too; matches() reads only * and **.
        assert not re.search(r"[?+\[\]!]", path), f"{path} uses a pattern character this test does not read"
    return paths


def matches(pattern, path):
    """Whether a GitHub filter pattern matches a path of the repository: `*` within a folder and `**`
    across folders."""
    regex = "".join(
        ".*" if part == "**" else "[^/]*" if part == "*" else re.escape(part)
        for part in re.split(r"(\*\*|\*)", pattern)
    )
    return re.fullmatch(regex, path) is not None


@functools.lru_cache(maxsize=None)
def entries(folder):
    return frozenset(os.listdir(folder)) if folder.is_dir() else frozenset()


def named_file(source, value):
    """The file of the repository a string of a source names, as a path from the root, or None. The
    name has to match in case too, as on Linux, and git's own files are none."""
    path = Path(os.path.normpath((source.parent if value.startswith(("./", "../")) else ROOT) / value))
    try:
        parts = path.relative_to(ROOT).parts
    except ValueError:
        return None
    folder = ROOT
    for part in parts:
        if part not in entries(folder):
            return None
        folder = folder / part
    return "/".join(parts) if folder.is_file() and parts[0] != ".git" else None


def files_named(source, text):
    """Every file of the repository a source's strings name; Python's are read exactly."""
    if source.suffix == ".py":
        values = [node.value for node in ast.walk(ast.parse(text)) if isinstance(node, ast.Constant) and isinstance(node.value, str)]
    else:
        values = [match.group(2) for match in QUOTED.finditer(text)]
    return {name for value in values if (name := named_file(source, value))}


def files_tests_read():
    """Every file of the repository the crate, the interface tests and the Python tests name. This
    file names paths only to match them."""
    sources = [
        *(ROOT / "core/manager").rglob("*.rs"),
        *(ROOT / "core/web/tests").rglob("*.ts"),
        *(path for folder in unit.FOLDERS for path in folder.glob("test_*.py") if path.name != Path(__file__).name),
    ]
    return set().union(*(files_named(source, source.read_text(encoding="utf-8")) for source in sources))


class PublishTriggersTest(unittest.TestCase):
    def test_a_pull_request_runs_the_checks_for_every_file_a_test_reads(self):
        patterns = trigger_paths("pull_request")
        read = files_tests_read()
        self.assertLessEqual({"README.md", "docs/API.md", "docs/MCP.md", ".github/workflows/publish.yml"}, read)
        for path in sorted(read):
            with self.subTest(path=path):
                self.assertTrue(
                    any(matches(pattern, path) for pattern in patterns),
                    f"a test reads {path}, so a pull request that changes only it must run the checks; "
                    "add it to the pull_request paths of .github/workflows/publish.yml",
                )

    def test_a_pull_request_runs_the_checks_for_everything_a_push_publishes(self):
        published = trigger_paths("push")
        self.assertEqual([path for path in published if path not in trigger_paths("pull_request")], [])

    def test_a_change_to_the_documentation_alone_publishes_nothing(self):
        published = trigger_paths("push")
        for path in ("README.md", "AGENTS.md", "docs/API.md"):
            with self.subTest(path=path):
                self.assertFalse(any(matches(pattern, path) for pattern in published), f"a push of {path} publishes")

    def test_a_change_to_a_skill_publishes_since_the_manager_serves_it(self):
        # The manager image copies docs/skills and builds every skill in, so a changed skill reaches
        # agents only through a new manager.
        self.assertIn("COPY docs/skills docs/skills", (ROOT / "core/manager/Dockerfile").read_text(encoding="utf-8"))
        published = trigger_paths("push")
        for skill in sorted((ROOT / "docs/skills").glob("*/SKILL.md")):
            path = skill.relative_to(ROOT).as_posix()
            with self.subTest(path=path):
                self.assertTrue(any(matches(pattern, path) for pattern in published), f"a push of {path} publishes nothing")

    def test_the_scan_finds_a_file_however_a_test_names_it(self):
        rust = ROOT / "core/manager/src/example_tests.rs"
        self.assertEqual(
            files_named(rust, """let quote = '"'; include_str!("../../../docs/API.md"); read("README.md");
                assert!(guide.contains("x"), "docs/MCP.md must list it"); read("license"); "../helpers"; ".git";"""),
            {"docs/API.md", "README.md"},
        )
        script = ROOT / "tests/test_example.py"
        self.assertEqual(
            files_named(script, 'for path in ("README.md", "LICENSE"):\n    read(path)\nROOT / "docs/MCP.md"  # "AGENTS.md"\n'),
            {"README.md", "LICENSE", "docs/MCP.md"},
        )
        interface = ROOT / "core/web/tests/example.test.ts"
        self.assertEqual(
            files_named(interface, "read('../../../THIRD_PARTY_NOTICES.md'); new URL(`../../../LICENSE`, import.meta.url)"),
            {"THIRD_PARTY_NOTICES.md", "LICENSE"},
        )

    def test_the_patterns_match_as_github_does(self):
        self.assertTrue(matches("docs/**", "docs/screenshots/thoric.png"))
        self.assertTrue(matches("Cargo.*", "Cargo.lock"))
        self.assertTrue(matches(".github/workflows/publish.yml", ".github/workflows/publish.yml"))
        self.assertFalse(matches("Cargo.*", "Cargo-lock"))
        self.assertFalse(matches("compose*.yml", "deploy/compose.yml"))
        self.assertFalse(matches("core/**", "docs/core/API.md"))


if __name__ == "__main__":
    unittest.main()
