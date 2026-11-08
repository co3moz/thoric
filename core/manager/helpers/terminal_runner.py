"""Run an interactive shell on a pseudo-terminal: input arrives through a FIFO, output goes to a bounded log.

The terminal's folder is new, inside a folder of the terminals of the user it runs as that only
that user opens, so a root terminal's files are out of the desktop user's reach."""
import fcntl
import json
import os
import pty
import select
import signal
import struct
import sys
import termios
import time

directory, cwd = sys.argv[1], sys.argv[2]
LIMIT = 1024 * 1024

def private_folder(path):
    """Make PATH a folder only this user opens, or take the one it made before. One another user
    made, or a link, is refused: the desktop user can make either in /tmp before a root helper."""
    try:
        os.mkdir(path, 0o700)
    except FileExistsError:
        pass
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        if os.fstat(descriptor).st_uid != os.geteuid():
            raise RuntimeError(path + " belongs to another user")
        os.fchmod(descriptor, 0o700)
    finally:
        os.close(descriptor)


private_folder(os.path.dirname(directory))
os.mkdir(directory, 0o700)
fifo = os.path.join(directory, "input")
os.mkfifo(fifo, 0o600)
state = {"state": "starting", "pid": None, "exit_code": None, "signal": None, "written": 0, "base": 0,
         "started_at": int(time.time())}


def record():
    temporary = os.path.join(directory, "status.tmp")
    with open(temporary, "w") as handle:
        json.dump(state, handle)
    os.replace(temporary, os.path.join(directory, "status"))


# Detach from the docker exec stream before forking so the shell outlives the request.
null = os.open(os.devnull, os.O_RDWR)
for descriptor in (0, 1, 2):
    os.dup2(null, descriptor)
os.close(null)
pid, master = pty.fork()
if pid == 0:
    try:
        os.chdir(cwd)
    except OSError:
        os.chdir("/")
    os.environ.update({"TERM": "dumb", "PS1": "$ ", "PS2": "> ", "COLUMNS": "120", "LINES": "40",
                       "DISPLAY": ":0", "PULSE_SERVER": "unix:/run/desktop/pulse/native",
                       "XDG_RUNTIME_DIR": "/run/desktop"})
    os.execvp("bash", ["bash", "--noprofile", "--norc", "-i"])
fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
# Input goes to the pty only as it takes it: a program that floods output without reading its
# input would otherwise block the write here, and with it the draining of its output, the stop
# file and so the terminal for good.
os.set_blocking(master, False)
reader = os.open(fifo, os.O_RDWR | os.O_NONBLOCK)
state.update({"state": "running", "pid": pid})
record()
output_path = os.path.join(directory, "output")
output = open(output_path, "ab", buffering=0)
size = 0
stopped = False
pending = b""
while True:
    if os.path.exists(os.path.join(directory, "stop")):
        stopped = True
        try:
            os.kill(pid, signal.SIGHUP)
        except ProcessLookupError:
            pass
        break
    # More input is taken from the FIFO only once what came before is in the pty.
    ready, writable, _ = select.select([master] + ([reader] if not pending else []),
                                       [master] if pending else [], [], 0.5)
    if pending and master in writable:
        try:
            pending = pending[os.write(master, pending):]
        except BlockingIOError:
            pass
        except OSError:
            break
    if master in ready:
        try:
            data = os.read(master, 65536)
        except BlockingIOError:
            data = None
        except OSError:
            data = b""
        if data is None:
            continue
        if not data:
            break
        if size + len(data) > LIMIT:
            output.close()
            os.replace(output_path, output_path + ".1")
            state["base"] = state["written"]
            output = open(output_path, "ab", buffering=0)
            size = 0
        output.write(data)
        size += len(data)
        state["written"] += len(data)
        record()
    if reader in ready:
        try:
            pending = os.read(reader, 65536)
        except BlockingIOError:
            pending = b""
output.close()
if stopped:
    # A shell that ignores the hangup is killed, so a closed terminal always ends.
    deadline = time.monotonic() + 3
    try:
        while os.waitid(os.P_PID, pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is None:
            if time.monotonic() > deadline:
                os.kill(pid, signal.SIGKILL)
                break
            time.sleep(0.1)
    except (ChildProcessError, ProcessLookupError):
        pass
try:
    _, status = os.waitpid(pid, 0)
except ChildProcessError:
    status = 0
state["state"] = "exited"
if os.WIFSIGNALED(status):
    state["signal"] = os.WTERMSIG(status)
else:
    state["exit_code"] = os.WEXITSTATUS(status)
record()
