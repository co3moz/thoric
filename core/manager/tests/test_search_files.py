"""Searching a folder's files: what matches by content and by name, that no symlink leads out of the
tree, that binary and unreadable files are passed over without failing, and that every bound, the
time of a regular expression that never ends among them, ends the search with what it found."""

import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

HELPER = Path(__file__).resolve().parents[1] / "helpers/search_files.py"
SOURCE = HELPER.read_text(encoding="utf-8")
# A pattern that backtracks without end on a line of the letter a that ends otherwise.
ENDLESS = "(a+)+$"


def start(source=SOURCE):
    """The helper as the manager runs it, the script as an argument and the request on stdin."""
    return subprocess.Popen([sys.executable, "-c", source], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE)


class SearchTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp()).resolve()
        self.addCleanup(shutil.rmtree, self.root, True)

    def write(self, name, data):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data if isinstance(data, bytes) else data.encode())
        return path

    def run_helper(self, request, edits=()):
        """Run one search with stdin held open, as the manager holds it, and return the process's
        exit code, its answer and what it said on stderr."""
        source = SOURCE
        for old, new in edits:
            self.assertIn(old, source)
            source = source.replace(old, new)
        process = start(source)
        self.addCleanup(process.kill)
        process.stdin.write(json.dumps({"path": str(self.root), **request}).encode() + b"\n")
        process.stdin.flush()
        output = process.stdout.read()
        error = process.stderr.read().decode()
        process.stdin.close()
        process.stdout.close()
        process.stderr.close()
        code = process.wait(timeout=30)
        return code, (json.loads(output) if output else None), error

    def search(self, request, edits=()):
        code, answer, error = self.run_helper(request, edits)
        self.assertEqual(code, 0, error)
        return answer

    def test_content_matches_carry_path_line_column_and_context(self):
        self.write("project/main.py", "import os\n\ndef main():\n    print('needle here')\n    return 0\n")
        self.write("project/notes.txt", "nothing\nNEEDLE in capitals\n")
        answer = self.search({"pattern": "needle", "context": 1})
        self.assertEqual(len(answer["matches"]), 1)
        match = answer["matches"][0]
        self.assertEqual(match["path"], str(self.root / "project/main.py"))
        self.assertEqual((match["line"], match["column"]), (4, 12))
        self.assertEqual(match["text"], "    print('needle here')")
        self.assertEqual(match["before"], ["def main():"])
        self.assertEqual(match["after"], ["    return 0"])
        self.assertFalse(answer["truncated"])
        self.assertIsNone(answer["truncated_by"])
        self.assertEqual((answer["files_searched"], answer["files_matched"]), (2, 1))
        both = self.search({"pattern": "needle", "case_sensitive": False})
        self.assertEqual([match["line"] for match in both["matches"]], [4, 2])
        self.assertNotIn("before", both["matches"][0])

    def test_a_pattern_is_a_regular_expression_unless_it_is_literal(self):
        self.write("a.txt", "price: 10\nprice: 1.0\na.b\naxb\n")
        self.assertEqual([m["line"] for m in self.search({"pattern": r"price: \d+$"})["matches"]], [1])
        self.assertEqual([m["line"] for m in self.search({"pattern": "a.b"})["matches"]], [3, 4])
        self.assertEqual([m["line"] for m in self.search({"pattern": "a.b", "literal": True})["matches"]], [3])
        literal = self.search({"pattern": "A.B", "literal": True, "case_sensitive": False})
        self.assertEqual([m["line"] for m in literal["matches"]], [3])

    def test_a_name_search_lists_paths_with_type_and_size(self):
        self.write("src/app.py", "12345")
        self.write("src/App.PY", "1")
        self.write("src/readme.md", "x")
        (self.root / "docs.py").mkdir()
        os.symlink("src/app.py", self.root / "link.py")
        answer = self.search({"name": "*.py"})
        self.assertNotIn("matches", answer)
        found = {entry["path"][len(str(self.root)) + 1:]: (entry["type"], entry["size"]) for entry in answer["files"]}
        self.assertEqual(found["src/app.py"], ("file", 5))
        self.assertEqual(found["docs.py"][0], "directory")
        self.assertEqual(found["link.py"][0], "symlink")
        self.assertNotIn("src/App.PY", found)
        self.assertEqual(len(found), 3)
        loose = self.search({"name": "*.py", "case_sensitive": False})
        self.assertEqual(len(loose["files"]), 4)

    def test_a_name_limits_which_files_a_pattern_is_looked_for_in(self):
        self.write("a.py", "needle\n")
        self.write("a.txt", "needle\n")
        answer = self.search({"pattern": "needle", "name": "*.py"})
        self.assertEqual([match["path"] for match in answer["matches"]], [str(self.root / "a.py")])
        self.assertEqual(answer["files_searched"], 1)

    def test_a_file_as_the_root_is_searched_alone(self):
        self.write("log.txt", "one\ntwo needle\n")
        self.write("other.txt", "needle\n")
        answer = self.search({"path": str(self.root / "log.txt"), "pattern": "needle"})
        self.assertEqual([(match["path"], match["line"]) for match in answer["matches"]],
                         [(str(self.root / "log.txt"), 2)])

    def test_no_symlink_leads_out_of_the_tree(self):
        outside = Path(tempfile.mkdtemp()).resolve()
        self.addCleanup(shutil.rmtree, outside, True)
        (outside / "secret.txt").write_text("needle outside\n")
        self.write("tree/inside.txt", "needle inside\n")
        os.symlink(outside, self.root / "tree" / "folder-link")
        os.symlink(outside / "secret.txt", self.root / "tree" / "file-link.txt")
        answer = self.search({"path": str(self.root / "tree"), "pattern": "needle"})
        self.assertEqual([match["path"] for match in answer["matches"]], [str(self.root / "tree/inside.txt")])
        names = self.search({"path": str(self.root / "tree"), "name": "secret*"})
        self.assertEqual(names["files"], [])
        # Nor is a root reached through a link, at its end or in a folder above it.
        os.symlink(self.root / "tree", self.root / "root-link")
        for path in (self.root / "root-link", self.root / "root-link" / "inside.txt",
                     self.root / "tree" / "folder-link"):
            code, answer, error = self.run_helper({"path": str(path), "pattern": "needle"})
            self.assertEqual((code, answer), (1, None), path)
            self.assertTrue(error.startswith("Search failed: "), error)
            self.assertNotIn("Traceback", error)

    def test_binary_files_are_skipped_for_a_content_search(self):
        self.write("image.bin", b"\x89PNG\0\0needle\n")
        self.write("text.txt", "needle\n")
        self.write("latin.txt", b"caf\xe9 needle\n")
        answer = self.search({"pattern": "needle"})
        self.assertEqual(sorted(Path(match["path"]).name for match in answer["matches"]), ["latin.txt", "text.txt"])
        self.assertEqual(answer["binary_skipped"], 1)
        # A name search still names it.
        self.assertEqual(len(self.search({"name": "*.bin"})["files"]), 1)

    @unittest.skipIf(os.geteuid() == 0, "root opens every file")
    def test_what_cannot_be_read_is_counted_and_passed_over(self):
        self.write("open/a.txt", "needle\n")
        self.write("closed/b.txt", "needle\n")
        locked = self.write("locked.txt", "needle\n")
        os.chmod(self.root / "closed", 0)
        os.chmod(locked, 0)
        self.addCleanup(os.chmod, self.root / "closed", 0o700)
        answer = self.search({"pattern": "needle"})
        self.assertEqual([match["path"] for match in answer["matches"]], [str(self.root / "open/a.txt")])
        self.assertEqual(answer["unreadable"], 2)
        self.assertFalse(answer["truncated"])

    def test_heavy_folders_are_left_out_unless_included(self):
        for folder in (".git", "node_modules", "target", "src"):
            self.write(f"{folder}/file.txt", "needle\n")
        answer = self.search({"pattern": "needle"})
        self.assertEqual([match["path"] for match in answer["matches"]], [str(self.root / "src/file.txt")])
        self.assertEqual(answer["directories_skipped"], 3)
        everything = self.search({"pattern": "needle", "include_ignored": True})
        self.assertEqual(len(everything["matches"]), 4)
        self.assertEqual(everything["directories_skipped"], 0)
        # Such a folder is searched when it is the root itself.
        inside = self.search({"path": str(self.root / ".git"), "pattern": "needle"})
        self.assertEqual(len(inside["matches"]), 1)

    @unittest.skipUnless(os.path.isdir("/proc/self"), "needs /proc")
    def test_the_kernels_filesystems_are_left_out(self):
        spec = importlib.util.spec_from_file_location("search_files", HELPER)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        self.assertIn("/proc", module.pseudo_mounts())
        self.assertNotIn(str(self.root), module.pseudo_mounts())
        # Entered only when it is the root itself, or when the request includes what is left out.
        self.assertEqual(self.search({"path": "/proc/sys/kernel", "name": "ostype"})["files"][0]["path"],
                         "/proc/sys/kernel/ostype")

    def test_results_stop_at_max_results(self):
        self.write("many.txt", "needle\n" * 50)
        answer = self.search({"pattern": "needle", "max_results": 7})
        self.assertEqual(len(answer["matches"]), 7)
        self.assertEqual((answer["truncated"], answer["truncated_by"]), (True, "results"))
        exact = self.search({"pattern": "needle", "max_results": 50})
        self.assertEqual((len(exact["matches"]), exact["truncated"]), (50, False))

    def test_entries_visited_are_bounded(self):
        for number in range(20):
            self.write(f"f{number:02}.txt", "needle\n")
        answer = self.search({"pattern": "needle"}, [("ENTRIES = 100_000", "ENTRIES = 5")])
        self.assertEqual(len(answer["matches"]), 5)
        self.assertEqual((answer["truncated_by"], answer["entries_visited"]), ("entries", 5))

    def test_a_long_line_is_cut_around_its_match(self):
        self.write("min.js", "x" * 5000 + "needle" + "y" * 5000 + "\nshort\n")
        match = self.search({"pattern": "needle", "context": 1})["matches"][0]
        self.assertEqual(len(match["text"]), 400)
        self.assertIn("needle", match["text"])
        self.assertTrue(match["cut"])
        self.assertEqual(match["column"], 5001)
        self.assertEqual(match["after"], ["short"])

    def test_a_large_file_is_read_up_to_its_limit(self):
        self.write("big.log", "early needle\n" + "filler line\n" * 100 + "late needle\n")
        answer = self.search({"pattern": "needle"}, [("FILE_BYTES = 4 * 1024 * 1024", "FILE_BYTES = 200")])
        self.assertEqual([match["line"] for match in answer["matches"]], [1])

    def test_the_answer_size_is_bounded(self):
        self.write("wide.txt", ("needle " + "z" * 300 + "\n") * 100)
        answer = self.search({"pattern": "needle", "max_results": 1000}, [("ANSWER = 256 * 1024", "ANSWER = 4096")])
        self.assertEqual(answer["truncated_by"], "size")
        self.assertLess(len(json.dumps(answer["matches"])), 4096 + 512)
        self.assertGreater(len(answer["matches"]), 1)

    def test_an_endless_pattern_is_killed_when_its_time_is_up(self):
        self.write("a.txt", "found needle\n")
        self.write("b.txt", "a" * 64 + "!\n")
        began = time.monotonic()
        answer = self.search({"pattern": "needle|" + ENDLESS, "max_seconds": 1})
        self.assertLess(time.monotonic() - began, 10)
        self.assertEqual((answer["truncated"], answer["truncated_by"]), (True, "time"))
        # What was found before the time ran out is still the answer.
        self.assertEqual([match["path"] for match in answer["matches"]], [str(self.root / "a.txt")])

    def test_closing_stdin_stops_the_search(self):
        self.write("b.txt", "a" * 64 + "!\n")
        process = start()
        self.addCleanup(process.kill)
        process.stdin.write(json.dumps({"path": str(self.root), "pattern": ENDLESS, "max_seconds": 30}).encode() + b"\n")
        process.stdin.flush()
        time.sleep(0.5)
        self.assertIsNone(process.poll())
        process.stdin.close()
        self.assertEqual(process.wait(timeout=10), 1)
        self.assertIn("stopped waiting", process.stderr.read().decode())
        process.stdout.close()
        process.stderr.close()

    def test_a_bad_pattern_or_request_is_refused_in_one_line(self):
        self.write("a.txt", "x\n")
        for request, said in (
            ({"pattern": "(unclosed"}, "not a valid regular expression"),
            ({"path": str(self.root / "missing"), "pattern": "x"}, "No such file"),
            ({}, "needs a pattern or a name"),
        ):
            code, answer, error = self.run_helper(request)
            self.assertEqual((code, answer), (1, None), request)
            self.assertTrue(error.startswith("Search failed: "), error)
            self.assertIn(said, error)
            self.assertEqual(error.count("\n"), 1, error)
        # The same text is a fine literal.
        self.assertEqual(self.search({"pattern": "(unclosed", "literal": True})["matches"], [])

    def test_a_name_that_is_not_utf8_still_answers_as_json(self):
        os.close(os.open(os.fsencode(self.root) + b"/bad-\xff.txt", os.O_CREAT | os.O_WRONLY, 0o600))
        answer = self.search({"name": "bad-*"})
        self.assertEqual(len(answer["files"]), 1)
        json.dumps(answer, ensure_ascii=False).encode("utf-8")


if __name__ == "__main__":
    unittest.main()
