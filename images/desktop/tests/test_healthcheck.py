"""The desktop's health check: a display that takes the connection and does not answer, as one a
program holds with a server grab, fails within three seconds, below Docker's four, and says so as
display-held, a display that is not there as display-gone, and a session that has not started as
desktop-starting. Each word the check begins a failure with is one the dashboard has words for
(core/web/src/lib/format.ts), since it shows Docker's log of the latest checks."""

import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[3]
CHECK = ROOT / "images/desktop/desktop-healthcheck.sh"
FORMAT = ROOT / "core/web/src/lib/format.ts"


def words(text, pattern):
    return sorted(set(re.findall(pattern, text)))


class WordsTest(unittest.TestCase):
    def test_every_failure_the_check_names_has_words_in_the_dashboard(self):
        named = words(CHECK.read_text(), r"echo '([a-z-]+): ")
        self.assertEqual(named, ["desktop-starting", "display-gone", "display-held", "session-failed"])
        known = words(FORMAT.read_text(), r'\["([a-z-]+):", "health\.')
        self.assertEqual(named, known)


@unittest.skipUnless(sys.platform.startswith("linux"), "runs bash with coreutils' timeout")
class CheckTest(unittest.TestCase):
    def run_check(self, xdpyinfo, ready=True):
        """Run the check with a runuser that runs its command as it is and an xdpyinfo whose body
        is `xdpyinfo`, the ready file in a folder of the test's own."""
        with tempfile.TemporaryDirectory() as folder:
            folder = Path(folder)
            for name, body in (("runuser", 'while [ "$1" != -- ]; do shift; done; shift; exec "$@"'),
                               ("xdpyinfo", xdpyinfo)):
                path = folder / name
                path.write_text("#!/bin/sh\n" + body + "\n")
                path.chmod(path.stat().st_mode | stat.S_IXUSR)
            ready_file = folder / "ready"
            if ready:
                ready_file.write_text("")
            script = folder / "check.sh"
            script.write_text(CHECK.read_text().replace("/run/desktop/ready", str(ready_file)))
            environment = dict(os.environ, PATH="%s:%s" % (folder, os.environ.get("PATH", "")))
            started = time.monotonic()
            done = subprocess.run(["bash", str(script)], capture_output=True, text=True, env=environment,
                                  timeout=30)
            return done, time.monotonic() - started

    def test_a_display_that_answers_is_healthy(self):
        done, _ = self.run_check("exit 0")
        self.assertEqual((done.returncode, done.stdout), (0, ""))

    def test_a_held_display_fails_before_docker_gives_up_and_says_so(self):
        done, took = self.run_check("exec sleep 30")
        self.assertEqual(done.returncode, 1)
        self.assertTrue(done.stdout.startswith("display-held: "), done.stdout)
        self.assertLess(took, 3.8)

    def test_a_display_that_is_not_there_is_gone(self):
        done, _ = self.run_check("echo 'unable to open display' >&2; exit 1")
        self.assertEqual(done.returncode, 1)
        self.assertTrue(done.stdout.startswith("display-gone: "), done.stdout)

    def test_a_session_that_has_not_started_says_so(self):
        done, _ = self.run_check("exit 0", ready=False)
        self.assertEqual(done.returncode, 1)
        self.assertTrue(done.stdout.startswith("desktop-starting: "), done.stdout)


if __name__ == "__main__":
    unittest.main()
