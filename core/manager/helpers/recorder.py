"""Capture desktop frames at a fixed rate until stopped, within time, frame and size limits."""
import json
import os
import subprocess
import sys
import time

directory, fps, max_seconds, max_width, region = sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), sys.argv[4], sys.argv[5]
# The manager's screenshot reader, the source of the desktop image's desktop-screenshot, which an
# image older than the reader has only as an xwd script that reads Wine's windows as black.
READER = sys.argv[6] if len(sys.argv) > 6 else ""
IMAGE_SCREENSHOT = "/usr/local/bin/desktop-screenshot"
# The reader's exit status when it cannot run in this desktop: the image's command then reads.
CANNOT_RUN = 3
LIMIT_BYTES = 256 * 1024 * 1024
LIMIT_FRAMES = 3000
os.makedirs(directory, 0o700, exist_ok=True)
null = os.open(os.devnull, os.O_RDWR)
for descriptor in (0, 1, 2):
    os.dup2(null, descriptor)
os.close(null)
state = {"state": "recording", "frames": 0, "bytes": 0, "started_at": int(time.time() * 1000),
         "ended_at": None, "reason": None, "fps": fps, "max_width": int(max_width), "region": region or None}


def screenshot():
    """One frame as JPEG on the result's stdout, from the reader, or from the image's command when
    the reader cannot run here."""
    if READER:
        result = subprocess.run([sys.executable, "-I", "-c", READER, max_width, region, ""],
                                capture_output=True, timeout=10)
        if result.returncode != CANNOT_RUN:
            return result
    return subprocess.run([IMAGE_SCREENSHOT, max_width, region, ""], capture_output=True, timeout=10)


def record():
    temporary = os.path.join(directory, "status.tmp")
    with open(temporary, "w") as handle:
        json.dump(state, handle)
    os.replace(temporary, os.path.join(directory, "status"))


record()
start = time.monotonic()
while True:
    if os.path.exists(os.path.join(directory, "stop")):
        state["reason"] = "stopped"
        break
    if time.monotonic() - start >= max_seconds:
        state["reason"] = "max_seconds"
        break
    if state["frames"] >= LIMIT_FRAMES:
        state["reason"] = "max_frames"
        break
    if state["bytes"] >= LIMIT_BYTES:
        state["reason"] = "max_bytes"
        break
    tick = time.monotonic()
    # The screen is read as the screenshot starts, before it is scaled and compressed, so a frame's
    # name carries that moment: the video shows each frame from it.
    captured = int(time.time() * 1000)
    try:
        result = screenshot()
    except (OSError, subprocess.TimeoutExpired):
        result = None
    if result is not None and result.returncode == 0 and result.stdout.startswith(b"\xff\xd8"):
        name = "frame-%05d-%d.jpg" % (state["frames"], captured)
        temporary = os.path.join(directory, name + ".tmp")
        with open(temporary, "wb") as handle:
            handle.write(result.stdout)
        os.replace(temporary, os.path.join(directory, name))
        state["frames"] += 1
        state["bytes"] += len(result.stdout)
        record()
    delay = 1 / fps - (time.monotonic() - tick)
    if delay > 0:
        time.sleep(delay)
state["state"] = "finished"
state["ended_at"] = int(time.time() * 1000)
record()
