"""recorder.py, which a recording runs in the desktop: its frames are read by the reader the manager
passes, the source of the desktop image's desktop-screenshot, and by the image's own command only
when that reader says it cannot run there (status 3) or none was passed."""

from pathlib import Path
import json
import os
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
RECORDER = (ROOT / "core/manager/helpers/recorder.py").read_text(encoding="utf-8")
IMAGE_COMMAND = 'IMAGE_SCREENSHOT = "/usr/local/bin/desktop-screenshot"'


def reader(status, mark=b""):
    """A reader that prints a JPEG marked with its arguments when it succeeds, and exits status."""
    return ("import sys\n"
            f"if {status} == 0:\n"
            f"    sys.stdout.buffer.write(b'\\xff\\xd8' + {mark!r} + ' '.join(sys.argv[1:]).encode())\n"
            f"sys.exit({status})\n")


@unittest.skipUnless(sys.platform.startswith("linux"), "runs the recorder as the desktop does")
class RecorderTest(unittest.TestCase):
    def record(self, *reader_argument):
        with tempfile.TemporaryDirectory() as folder:
            image = Path(folder, "desktop-screenshot")
            image.write_text("#!/bin/sh\nprintf '\\377\\330image %s' \"$*\"\n")
            image.chmod(0o755)
            self.assertIn(IMAGE_COMMAND, RECORDER)
            source = RECORDER.replace(IMAGE_COMMAND, f"IMAGE_SCREENSHOT = {str(image)!r}")
            directory = os.path.join(folder, "recording")
            subprocess.run([sys.executable, "-c", source, directory, "20", "0.3", "320", "0,0,64,48",
                            *reader_argument], check=True, timeout=30)
            status = json.loads(Path(directory, "status").read_text())
            frames = sorted(Path(directory).glob("frame-*.jpg"))
            return status, {frame.read_bytes() for frame in frames}

    def test_frames_come_from_the_reader_the_manager_passes(self):
        status, frames = self.record(reader(0, b"reader "))
        self.assertEqual(status["state"], "finished")
        self.assertGreater(status["frames"], 0)
        self.assertEqual(frames, {b"\xff\xd8reader 320 0,0,64,48 "})

    def test_the_image_command_reads_when_the_reader_cannot_run_or_is_missing(self):
        for argument in ((reader(3),), ()):
            status, frames = self.record(*argument)
            self.assertGreater(status["frames"], 0, argument)
            self.assertEqual(frames, {b"\xff\xd8image 320 0,0,64,48 "}, argument)

    def test_a_reader_that_fails_otherwise_gives_no_frames(self):
        status, frames = self.record(reader(1))
        self.assertEqual((status["state"], status["frames"], frames), ("finished", 0, set()))


if __name__ == "__main__":
    unittest.main()
