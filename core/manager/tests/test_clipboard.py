"""The clipboard helper: Turkish letters, CJK and emoji pass whole and are never cut inside a
character, the text from stdin is bounded, UTF-8 and free of NUL, a desktop image without GTK 3
for Python is answered with what it lacks, also from the owner process a set leaves, the exit
codes are those the manager reads, and where Xvfb and GTK 3 are installed, a set outlives its call
on a private display, reads back, and gives way to the next set."""

import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time
import unittest

HELPER = Path(__file__).resolve().parents[1] / "helpers/clipboard.py"
MANAGER = Path(__file__).resolve().parents[1] / "src/clipboard.rs"
spec = importlib.util.spec_from_file_location("clipboard", HELPER)
clipboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(clipboard)

UNICODE = "Türkçe: ç ğ ı İ ö ş ü\n中文 日本語 한국어\nemoji 😀👍🏽\ttab"
LIMIT = 512 * 1024
# Runs the helper as the manager does, with PyGObject out of reach, as in an older desktop image.
WITHOUT_GTK = ("import sys; sys.modules['gi'] = None; path = sys.argv.pop(1); "
               "exec(compile(open(path).read(), path, 'exec'))")


def run(arguments, data=None, env=None, code=None):
    command = [sys.executable] + (["-c", code] if code else []) + [str(HELPER)] + arguments
    return subprocess.run(command, input=data, capture_output=True, env=env, timeout=60)


class CutTest(unittest.TestCase):
    def test_a_text_within_the_limit_is_whole(self):
        self.assertEqual(clipboard.cut(UNICODE, LIMIT), (UNICODE, len(UNICODE.encode()), False))

    def test_a_cut_never_splits_a_character(self):
        size = len(UNICODE.encode())
        for limit in range(size):
            text, total, truncated = clipboard.cut(UNICODE, limit)
            self.assertTrue(truncated)
            self.assertEqual(total, size)
            self.assertTrue(UNICODE.startswith(text), limit)
            self.assertLessEqual(len(text.encode()), limit)
            # Only the character the limit falls inside is left out.
            self.assertGreater(len(text.encode()) + 4, limit)


class ReadTextTest(unittest.TestCase):
    def test_unicode_up_to_the_limit_is_read(self):
        data = UNICODE.encode()
        self.assertEqual(clipboard.read_text(io.BytesIO(data), len(data)), UNICODE)

    def test_one_byte_more_is_refused(self):
        data = UNICODE.encode()
        with self.assertRaisesRegex(ValueError, "limited to %d bytes" % (len(data) - 1)):
            clipboard.read_text(io.BytesIO(data), len(data) - 1)

    def test_text_that_is_not_utf8_or_holds_nul_is_refused(self):
        with self.assertRaisesRegex(ValueError, "must be UTF-8"):
            clipboard.read_text(io.BytesIO("ğ".encode()[:1]), LIMIT)
        with self.assertRaisesRegex(ValueError, "NUL"):
            clipboard.read_text(io.BytesIO(b"a\0b"), LIMIT)


class CommandTest(unittest.TestCase):
    def test_unknown_selections_and_commands_are_refused(self):
        for arguments in (["get", "both", "10"], ["get", "secondary", "10"], ["copy", "clipboard", "10"]):
            result = run(arguments)
            self.assertEqual(result.returncode, 1, arguments)
            self.assertIn(b"Clipboard operation failed: Unknown", result.stderr)

    def test_an_image_without_gtk_is_named_on_a_read(self):
        result = run(["get", "clipboard", str(LIMIT)], code=WITHOUT_GTK)
        self.assertEqual(result.returncode, 3)
        self.assertEqual(result.stderr.decode().strip(), clipboard.UNSUPPORTED)

    def test_an_image_without_gtk_is_named_by_the_owner_of_a_set(self):
        started = time.monotonic()
        result = run(["set", "clipboard", str(LIMIT)], UNICODE.encode(), code=WITHOUT_GTK)
        self.assertEqual(result.returncode, 3)
        self.assertEqual(result.stderr.decode().strip(), clipboard.UNSUPPORTED)
        self.assertEqual(result.stdout, b"")
        self.assertLess(time.monotonic() - started, clipboard.WAIT)

    def test_a_set_reads_no_more_than_the_limit(self):
        result = run(["set", "clipboard", "4"], "çğıİ".encode(), code=WITHOUT_GTK)
        self.assertEqual(result.returncode, 1)
        self.assertIn(b"limited to 4 bytes", result.stderr)

    def test_the_exit_codes_are_those_the_manager_reads(self):
        source = MANAGER.read_text()
        for name, kind in (("TIMED_OUT", clipboard.TimedOut), ("FAILED", clipboard.Failure)):
            self.assertIn("const %s: i32 = %d;" % (name, kind.code), source)
        # A refused request exits 1, which none of the failures share.
        self.assertEqual(sorted(clipboard.FAILURES), [3, 4, 5])


def missing():
    """What a private display for the round trip lacks, Xvfb or GTK 3 for Python, if anything."""
    if not shutil.which("Xvfb"):
        return "Xvfb is not installed"
    probe = subprocess.run([sys.executable, "-c", "import gi; gi.require_version('Gtk', '3.0')"],
                           capture_output=True)
    return None if probe.returncode == 0 else "GTK 3 for Python is not installed"


MISSING = missing()


@unittest.skipIf(MISSING, MISSING or "")
class RoundTripTest(unittest.TestCase):
    """On a display of its own, so a desktop's clipboard, or the one WSLg shares with Windows, is
    never touched."""

    @classmethod
    def setUpClass(cls):
        read, write = os.pipe()
        cls.server = subprocess.Popen(["Xvfb", "-displayfd", str(write), "-nolisten", "tcp"],
                                      pass_fds=(write,), stderr=subprocess.DEVNULL)
        os.close(write)
        with os.fdopen(read) as numbers:
            cls.display = ":" + numbers.readline().strip()
        cls.env = dict(os.environ, DISPLAY=cls.display)

    @classmethod
    def tearDownClass(cls):
        for pid in cls.owners():
            os.kill(pid, signal.SIGKILL)
        cls.server.terminate()
        cls.server.wait(timeout=10)

    @classmethod
    def owners(cls):
        """The owner processes the helper left on this display."""
        found = []
        for pid in filter(str.isdigit, os.listdir("/proc")):
            try:
                with open("/proc/%s/cmdline" % pid, "rb") as handle:
                    command = handle.read()
                with open("/proc/%s/environ" % pid, "rb") as handle:
                    environment = handle.read().split(b"\0")
            except OSError:
                continue
            if str(HELPER).encode() in command and ("DISPLAY=" + cls.display).encode() in environment:
                found.append(int(pid))
        return found

    def wait_for_owners(self, count):
        deadline = time.monotonic() + 3 * clipboard.CHECK_SECONDS
        while len(self.owners()) != count and time.monotonic() < deadline:
            time.sleep(0.1)
        return self.owners()

    def get(self, selection):
        result = run(["get", selection, str(LIMIT)], env=self.env)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_a_set_outlives_its_call_and_gives_way_to_the_next(self):
        # The call returns while the owner serves: the owner holds none of the call's streams.
        result = run(["set", "clipboard", str(LIMIT)], UNICODE.encode(), env=self.env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout), {"selection": "clipboard", "bytes": len(UNICODE.encode()),
                                                     "characters": len(UNICODE)})
        first = self.wait_for_owners(1)
        self.assertEqual(len(first), 1)
        self.assertEqual(self.get("clipboard"), {"selection": "clipboard", "text": UNICODE,
                                                 "bytes": len(UNICODE.encode()), "truncated": False})
        result = run(["set", "both", str(LIMIT)], "ikinci 二".encode(), env=self.env)
        self.assertEqual(result.returncode, 0, result.stderr)
        second = self.wait_for_owners(1)
        self.assertEqual(len(second), 1)
        self.assertNotEqual(first, second)
        self.assertEqual(self.get("clipboard")["text"], "ikinci 二")
        self.assertEqual(self.get("primary")["text"], "ikinci 二")

    def test_a_long_text_is_read_back_cut_at_the_limit(self):
        text = "ğ" * (LIMIT // 2) + "😀"
        result = run(["set", "clipboard", str(2 * LIMIT)], text.encode(), env=self.env)
        self.assertEqual(result.returncode, 0, result.stderr)
        read = self.get("clipboard")
        self.assertEqual((read["bytes"], read["truncated"]), (LIMIT + 4, True))
        self.assertEqual(read["text"], "ğ" * (LIMIT // 2))

    def test_an_empty_text_reads_back_empty(self):
        result = run(["set", "both", str(LIMIT)], b"", env=self.env)
        self.assertEqual(result.returncode, 0, result.stderr)
        for selection in ("clipboard", "primary"):
            self.assertEqual(self.get(selection)["text"], "", selection)

    def test_a_display_that_does_not_open_is_the_helpers_failure(self):
        env = dict(self.env, DISPLAY=":%d" % (int(self.display[1:]) + 4321))
        for arguments, data in ((["get", "clipboard", "10"], None), (["set", "clipboard", "10"], b"x")):
            result = run(arguments, data, env=env)
            self.assertEqual(result.returncode, clipboard.Failure.code, result.stderr)
            self.assertIn(b"Could not open the desktop's display", result.stderr)


if __name__ == "__main__":
    unittest.main()
