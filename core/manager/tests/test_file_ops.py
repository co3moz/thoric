"""Describing the paths a change list names: each one as it is now, without following a symlink in
its folders or at its end, and nothing for one that is gone, so a list of them never fails as a
whole."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "helpers/file_ops.py"


class StatsTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp()).resolve()
        self.addCleanup(shutil.rmtree, self.root)

    def stats(self, paths):
        return subprocess.run([sys.executable, str(SCRIPT), "stats"], input=json.dumps(paths).encode(),
                              capture_output=True, timeout=30)

    def test_each_path_is_described_or_none(self):
        (self.root / "project").mkdir()
        (self.root / "project" / "notes.txt").write_bytes(b"hello")
        (self.root / "elsewhere").mkdir()
        (self.root / "elsewhere" / "secret").write_bytes(b"secret")
        os.symlink("elsewhere", self.root / "link")
        os.symlink("project/notes.txt", self.root / "file-link")
        paths = [
            str(self.root / "project"),
            str(self.root / "project" / "notes.txt"),
            str(self.root / "link" / "secret"),
            str(self.root / "file-link"),
            str(self.root / "gone"),
        ]
        result = self.stats(paths)
        self.assertEqual(result.returncode, 0, result.stderr)
        described = json.loads(result.stdout)
        self.assertEqual(len(described), len(paths))
        self.assertEqual(described[0]["type"], "directory")
        self.assertEqual((described[1]["type"], described[1]["size"]), ("file", 5))
        self.assertIn("modified_at", described[1])
        self.assertNotIn("name", described[1])
        # A folder behind a symlink is not entered, and a link is described as the link itself.
        self.assertIsNone(described[2])
        self.assertEqual(described[3]["type"], "symlink")
        self.assertIsNone(described[4])

    def test_a_name_with_a_line_break_or_other_characters_arrives_whole(self):
        name = "two\nlines é"
        (self.root / name).write_bytes(b"x")
        result = self.stats([str(self.root / name)])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)[0]["type"], "file")

    def test_too_many_paths_are_refused(self):
        result = self.stats([str(self.root)] * 1001)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("at most 1000 paths", result.stderr.decode())


if __name__ == "__main__":
    unittest.main()
