"""List the desktop's processes with their CPU and memory use, or signal one and its descendants.

The manager runs it with `python3 -I -c` through `docker exec`: `list` as the desktop user, who
reads every process's stat and command line in /proc, and `signal` as the desktop user or, when
asked, as root. It answers one line of JSON."""
import json
import os
import signal
import sys
import time

PROC = "/proc"
PASSWD = "/etc/passwd"
CGROUP = "/sys/fs/cgroup"
# The processes that make the desktop, its screen, sound and session, by the name the kernel
# keeps, at most 15 bytes: ending one ends the screen, the sound or the session.
DESKTOP = frozenset({
    "desktop-session", "Xtigervnc", "pulseaudio", "dbus-run-session", "dbus-daemon",
    "startxfce4", "xfce4-session", "xfwm4", "xfce4-panel", "xfdesktop", "xfsettingsd",
    "desktop-gnome", "gnome-session-b", "gnome-shell",
})
COMMAND_LIMIT = 512
SIGNALS = {"term": signal.SIGTERM, "kill": signal.SIGKILL, "int": signal.SIGINT, "hup": signal.SIGHUP}
ORDERS = ("cpu", "memory", "pid")
NAME_LIMIT = 15


def stat(pid):
    """The name and the fields after it of /proc/PID/stat, or None once the process is gone."""
    try:
        with open(os.path.join(PROC, str(pid), "stat"), "rb") as handle:
            text = handle.read().decode("utf-8", "replace")
    except OSError:
        return None
    start, end = text.find("("), text.rfind(")")
    fields = text[end + 2:].split()
    if start < 0 or len(fields) < 22:
        return None
    return text[start + 1:end], fields


def users():
    """User names by ID, from the desktop's own passwd."""
    names = {}
    try:
        with open(PASSWD, encoding="utf-8", errors="replace") as handle:
            for line in handle:
                parts = line.split(":")
                if len(parts) > 2 and parts[2].isdigit():
                    names.setdefault(int(parts[2]), parts[0])
    except OSError:
        pass
    return names


def command_line(pid, name):
    """The command line as one bounded line: a multi-line argument, such as a `python3 -c`
    script, keeps its first line, and a process without one, a kernel thread or one that has
    ended, shows its name in brackets as ps does."""
    try:
        with open(os.path.join(PROC, str(pid), "cmdline"), "rb") as handle:
            data = handle.read(64 * 1024)
    except OSError:
        data = b""
    args = [arg.decode("utf-8", "replace") for arg in data.split(b"\0")]
    while args and not args[-1]:
        args.pop()
    if not args:
        return "[%s]" % name, None
    shown = []
    for arg in args:
        lines = arg.strip().splitlines() or [""]
        shown.append(lines[0] + (" …" if len(lines) > 1 else ""))
    text = " ".join(shown)
    if len(text) > COMMAND_LIMIT:
        text = text[:COMMAND_LIMIT - 1] + "…"
    return text, args[0]


def display_name(name, first):
    """The kernel's name, or the program's own when the kernel cut it at 15 bytes."""
    if first and len(name.encode()) >= NAME_LIMIT:
        program = os.path.basename(first.split(" ", 1)[0])
        if program.startswith(name):
            return program
    return name


def read_all(skip):
    """Every process but `skip`, the helper itself, by pid."""
    found = {}
    for entry in os.listdir(PROC):
        if not entry.isdigit() or int(entry) == skip:
            continue
        pid = int(entry)
        read = stat(pid)
        if read is None:
            continue
        try:
            uid = os.stat(os.path.join(PROC, entry)).st_uid
        except OSError:
            continue
        name, fields = read
        found[pid] = {"name": name, "fields": fields, "uid": uid}
    return found


def ticks(fields):
    return int(fields[11]) + int(fields[12])


def started(fields):
    return int(fields[19])


def role(pid, ppid, name):
    """desktop for what makes the desktop, thoric for what the manager started through docker
    exec, whose parent is outside the container, and program for everything else."""
    if pid == 1 or name in DESKTOP:
        return "desktop"
    if ppid == 0:
        return "thoric"
    return "program"


def boot_time():
    try:
        with open(os.path.join(PROC, "stat")) as handle:
            for line in handle:
                if line.startswith("btime "):
                    return int(line.split()[1])
    except (OSError, ValueError):
        pass
    return None


def cgroup(name):
    try:
        with open(os.path.join(CGROUP, name)) as handle:
            return handle.read().strip()
    except OSError:
        return None


def memory(name):
    value = cgroup(name)
    try:
        return None if value in (None, "max") else int(value)
    except ValueError:
        return None


def cpus():
    """The CPUs the container may use: those it may run on, fewer when its quota says so."""
    try:
        count = float(len(os.sched_getaffinity(0)))
    except (AttributeError, OSError):
        count = float(os.cpu_count() or 1)
    quota = (cgroup("cpu.max") or "").split()
    if len(quota) == 2 and quota[0] != "max":
        try:
            count = min(count, int(quota[0]) / int(quota[1]))
        except (ValueError, ZeroDivisionError):
            pass
    return round(count, 2)


def describe(pid, item, used, elapsed, hz, page, boot, names):
    fields = item["fields"]
    ppid = int(fields[1])
    command, first = command_line(pid, item["name"])
    return {
        "pid": pid,
        "ppid": ppid,
        "uid": item["uid"],
        "user": names.get(item["uid"], str(item["uid"])),
        "state": fields[0],
        "name": display_name(item["name"], first),
        "command": command,
        "role": role(pid, ppid, item["name"]),
        "threads": int(fields[17]),
        "rss_bytes": int(fields[21]) * page,
        "cpu_percent": round(used / hz / elapsed * 100, 1) if elapsed > 0 else 0.0,
        "cpu_seconds": round(ticks(fields) / hz, 2),
        "started_at": boot + started(fields) // hz if boot is not None else None,
    }


def ordered(items, order):
    if order == "memory":
        return sorted(items, key=lambda item: (-item["rss_bytes"], -item["cpu_percent"], item["pid"]))
    if order == "pid":
        return sorted(items, key=lambda item: item["pid"])
    return sorted(items, key=lambda item: (-item["cpu_percent"], -item["rss_bytes"], item["pid"]))


def matches(item, query):
    return not query or any(query in item[key].lower() for key in ("name", "command", "user"))


def listing(interval_ms, order, limit, query):
    """The processes over an interval of interval_ms: CPU use is what each spent of it, 100 for
    one full core, and a process that started within it counts all it spent."""
    me = os.getpid()
    hz = os.sysconf("SC_CLK_TCK")
    page = os.sysconf("SC_PAGE_SIZE")
    before = {(pid, started(item["fields"])): ticks(item["fields"]) for pid, item in read_all(me).items()}
    begun = time.monotonic()
    time.sleep(interval_ms / 1000)
    after = read_all(me)
    elapsed = time.monotonic() - begun
    boot, names = boot_time(), users()
    items = []
    for pid, item in after.items():
        spent = ticks(item["fields"]) - before.get((pid, started(item["fields"])), 0)
        items.append(describe(pid, item, max(spent, 0), elapsed, hz, page, boot, names))
    query = (query or "").lower()
    found = ordered([item for item in items if matches(item, query)], order)
    return {
        "processes": found[:limit],
        "total": len(items),
        "matched": len(found),
        "cpu_percent": round(sum(item["cpu_percent"] for item in items), 1),
        "rss_bytes": sum(item["rss_bytes"] for item in items),
        "memory_bytes": memory("memory.current"),
        "memory_limit_bytes": memory("memory.max"),
        "cpus": cpus(),
        "interval_ms": int(elapsed * 1000),
    }


def descendants(table, pid):
    """The descendants of pid as they are now, parents before their children."""
    children = {}
    for child, item in table.items():
        children.setdefault(int(item["fields"][1]), []).append(child)
    found, queue = [], sorted(children.get(pid, []))
    while queue:
        child = queue.pop(0)
        if child in found or child == pid:
            continue
        found.append(child)
        queue.extend(sorted(children.get(child, [])))
    return found


def alive(pid, start):
    """Whether the process that had start time `start` still runs: one that has ended and waits to
    be reaped has ended, and a new process under the same pid is another."""
    read = stat(pid)
    return read is not None and started(read[1]) == start and read[1][0] not in ("Z", "X")


def send(pid, name, tree, wait_ms):
    """Send signal `name` to pid and, with tree, to its descendants, parents first so a program
    can end its own children, then wait up to wait_ms for them to end."""
    table = read_all(os.getpid())
    if pid not in table:
        return {"error": "missing"}
    number = SIGNALS[name]
    targets = [pid] + (descendants(table, pid) if tree else [])
    signalled, refused = [], []
    for target in targets:
        try:
            os.kill(target, number)
        except ProcessLookupError:
            pass
        except PermissionError:
            if target == pid:
                uid = table[pid]["uid"]
                return {"error": "permission", "user": users().get(uid, str(uid))}
            refused.append(target)
            continue
        signalled.append(target)
    starts = {target: started(table[target]["fields"]) for target in signalled}
    deadline = time.monotonic() + wait_ms / 1000
    while True:
        running = [target for target in signalled if alive(target, starts[target])]
        if not running or time.monotonic() >= deadline:
            break
        time.sleep(0.05)
    return {
        "signalled": signalled,
        "ended": [target for target in signalled if target not in running],
        "running": running,
        "refused": refused,
    }


def main(arguments):
    command = arguments[0] if arguments else ""
    if command == "list" and len(arguments) in (4, 5) and arguments[2] in ORDERS:
        query = arguments[4] if len(arguments) == 5 else ""
        result = listing(int(arguments[1]), arguments[2], int(arguments[3]), query)
    elif command == "signal" and len(arguments) == 5 and arguments[2] in SIGNALS:
        result = send(int(arguments[1]), arguments[2], arguments[3] == "1", int(arguments[4]))
    else:
        print("Usage: list INTERVAL_MS cpu|memory|pid LIMIT [QUERY] | signal PID term|kill|int|hup 0|1 WAIT_MS",
              file=sys.stderr)
        return 2
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
