"""Write to, read from or stop an interactive terminal session run by the terminal runner.

A terminal counts only in a folder of this user's own, as the runner makes it, and its files are
opened without following links, so a root terminal's control touches nothing the desktop user
prepared."""
import json
import os
import re
import select
import stat
import sys
import time

ESCAPES = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]")
# The most one write hands over, as the manager limits what is typed, plus the newline it adds.
WRITE_LIMIT = 65536 + 1
# How long a write waits for a runner whose program is not taking input.
WRITE_WAIT = 5.0


def ours(directory):
    """Whether the terminal's folder is in one of this user's own, as the runner makes it."""
    try:
        info = os.lstat(os.path.dirname(directory))
    except OSError:
        return False
    return stat.S_ISDIR(info.st_mode) and info.st_uid == os.geteuid()


def opened(path, flags=os.O_RDONLY):
    return os.open(path, flags | os.O_NOFOLLOW, 0o600)


def status(directory):
    if ours(directory):
        try:
            with os.fdopen(opened(os.path.join(directory, "status"))) as handle:
                return json.load(handle)
        except (OSError, ValueError):
            if os.path.isdir(directory):
                return {"state": "starting", "written": 0, "base": 0}
    return {"state": "unknown", "written": 0, "base": 0, "gone": True,
            "error": "Terminal files are not available; the desktop may have restarted"}


def read_file(path):
    try:
        with os.fdopen(opened(path), "rb") as handle:
            return handle.read()
    except OSError:
        return b""


def write(directory, data):
    """Hand DATA, what is typed, to the runner: at most WRITE_WAIT seconds for a runner whose
    program takes no input, so a shell busy with a flood of output never holds this call."""
    info = status(directory)
    if info.get("state") != "running":
        raise RuntimeError("Terminal is not running")
    descriptor = opened(os.path.join(directory, "input"), os.O_WRONLY | os.O_NONBLOCK)
    try:
        deadline = time.monotonic() + WRITE_WAIT
        while data:
            try:
                data = data[os.write(descriptor, data):]
            except BlockingIOError:
                if time.monotonic() > deadline:
                    raise RuntimeError("The terminal is not taking input; its program is busy")
                select.select([], [descriptor], [], 0.1)
    finally:
        os.close(descriptor)
    return status(directory)


def snapshot(directory):
    """The status and both output files as of one moment: the runner appends to the output before
    it records the new count, so the files are read again until the count has not moved."""
    info = status(directory)
    old = new = b""
    for _ in range(3):
        old = read_file(os.path.join(directory, "output.1"))
        new = read_file(os.path.join(directory, "output"))
        again = status(directory)
        if (again.get("written", 0), again.get("base", 0)) == (info.get("written", 0), info.get("base", 0)):
            break
        info = again
    # What arrived after the count was recorded is not counted yet; it is read next time.
    extra = len(old) + len(new) - info.get("written", 0)
    if extra > 0:
        if extra <= len(new):
            new = new[:len(new) - extra]
        else:
            old, new = old[:len(old) - (extra - len(new))], b""
    return info, old, new


def read(directory, offset, limit, strip):
    info, old, new = snapshot(directory)
    written = info.get("written", 0)
    start = written - len(old) - len(new)
    dropped = offset < start
    offset = max(start, min(offset, written))
    chunk = (old + new)[offset - start:offset - start + limit]
    text = chunk.decode("utf-8", "replace")
    if strip:
        text = ESCAPES.sub("", text).replace("\r", "")
    result = dict(info)
    result.update({"output": text, "offset": offset, "next_offset": offset + len(chunk),
                   "more": offset + len(chunk) < written, "dropped": dropped})
    return result


def remove(directory):
    """Delete the files of a terminal that ended, which only its runner and this helper write."""
    descriptor = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for name in os.listdir(descriptor):
            os.unlink(name, dir_fd=descriptor)
    finally:
        os.close(descriptor)
    os.rmdir(directory)


def stop(directory, wait_seconds):
    if ours(directory) and os.path.isdir(directory):
        os.close(opened(os.path.join(directory, "stop"), os.O_WRONLY | os.O_CREAT))
    deadline = time.monotonic() + wait_seconds
    while time.monotonic() < deadline and status(directory).get("state") in ("running", "starting"):
        time.sleep(0.1)
    info = status(directory)
    if info.get("state") == "exited":
        try:
            remove(directory)
        except OSError:
            pass
    return info


command, *arguments = sys.argv[1:]
try:
    if command == "status":
        # Every terminal of a list in one call, in the order asked.
        print(json.dumps([status(directory) for directory in arguments]))
    elif command == "write":
        # What is typed arrives on stdin, never on a command line, which every process reads.
        print(json.dumps(write(arguments[0], sys.stdin.buffer.read(WRITE_LIMIT + 1)[:WRITE_LIMIT])))
    elif command == "read":
        print(json.dumps(read(arguments[0], int(arguments[1]), int(arguments[2]), arguments[3] == "true")))
    elif command == "stop":
        print(json.dumps(stop(arguments[0], float(arguments[1]))))
    else:
        sys.exit(2)
except Exception as error:
    print("Terminal operation failed: " + str(error), file=sys.stderr)
    sys.exit(1)
