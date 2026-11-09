"""Search a folder's files by content, by name or both, as the user who runs it, within bounds.

The request is one line of JSON on stdin, so a pattern or a path never shows on a command line. The
caller keeps stdin open until it has the answer: the search itself runs in a child process, and this
process ends it when the time is up, which a regular expression that backtracks without end never
notices by itself, or when stdin closes, as it does when the caller gives up.

No symlink is followed: the root is opened one component at a time, every folder below it through its
parent's descriptor, and a link is named by a name search and never read. A file or a folder that
cannot be opened is counted and passed over, a file with a NUL byte near its start is taken as
binary, and every direction has its limit: time, entries looked at, bytes read of a file, characters
of a returned line, results and the size of the answer.
"""
import fnmatch
import itertools
import json
import os
import re
import select
import signal
import stat
import sys
import time

# Folders a walk leaves out by their name unless the request includes them: version control data,
# installed packages, build output and caches, which hold many files nobody meant to search.
HEAVY = frozenset({".git", ".hg", ".svn", "node_modules", "target", "__pycache__", ".cache", ".venv"})
# Filesystems the kernel makes up, whose files are not files of the desktop, left out the same way.
PSEUDO = frozenset({
    "proc", "sysfs", "devtmpfs", "devpts", "cgroup", "cgroup2", "mqueue", "debugfs", "tracefs",
    "securityfs", "pstore", "bpf", "configfs", "fusectl", "hugetlbfs", "binfmt_misc", "autofs",
})
# The most entries, files and folders alike, one search looks at.
ENTRIES = 100_000
# The most bytes read of one file; a larger file is searched up to its last whole line before that.
FILE_BYTES = 4 * 1024 * 1024
# The bytes at a file's start that decide whether it is binary.
PROBE = 8192
# The most characters returned of one line.
LINE = 400
# The most bytes of results in one answer.
ANSWER = 256 * 1024
# The deepest folder below the root that is entered.
DEPTH = 64
# How long after its time a search that has not ended by itself is killed.
GRACE = 1.0
# The most bytes of the request line.
REQUEST = 65536


class Stop(Exception):
    """The search ends here, with the limit that ended it."""


def shown(name):
    """A name as text JSON can carry: bytes that are not UTF-8 become replacement characters."""
    return name.encode("utf-8", "surrogateescape").decode("utf-8", "replace")


def pseudo_mounts():
    """The mount points of the kernel's own filesystems, or none where they cannot be read."""
    points = set()
    try:
        with open("/proc/self/mountinfo", "rb") as file:
            for line in file:
                fields, _, rest = line.decode("utf-8", "surrogateescape").partition(" - ")
                fields = fields.split(" ")
                if len(fields) > 4 and rest.split(" ")[0] in PSEUDO:
                    # The kernel writes a space, a tab, a line break and a backslash as octal.
                    points.add(re.sub(r"\\([0-7]{3})", lambda m: chr(int(m.group(1), 8)), fields[4]))
    except OSError:
        pass
    return points


def kind(info):
    if stat.S_ISDIR(info.st_mode):
        return "directory"
    if stat.S_ISLNK(info.st_mode):
        return "symlink"
    if stat.S_ISREG(info.st_mode):
        return "file"
    return "special"


def cut(line, start=0):
    """A line without its line ending, at most LINE characters of it from around `start`, and
    whether anything was left out."""
    line = line.rstrip("\r")
    if len(line) <= LINE:
        return line, False
    begin = max(0, min(start - LINE // 4, len(line) - LINE))
    return line[begin:begin + LINE], True


class Search:
    def __init__(self, request, out, deadline):
        self.out = out
        self.deadline = deadline
        self.limit = request["max_results"]
        self.context = request.get("context", 0)
        self.include = bool(request.get("include_ignored"))
        flags = 0 if request.get("case_sensitive", True) else re.IGNORECASE
        pattern, name = request.get("pattern"), request.get("name")
        self.literal = None
        self.pattern = None
        if pattern is not None:
            if request.get("literal"):
                # A literal that is not in the file at all is in none of its lines.
                self.literal = pattern if not flags else None
                pattern = re.escape(pattern)
            try:
                self.pattern = re.compile(pattern, flags)
            except (re.error, RecursionError, OverflowError) as error:
                raise ValueError(f"pattern is not a valid regular expression: {error}")
        self.name = re.compile(fnmatch.translate(name), flags) if name is not None else None
        self.pseudo = set() if self.include else pseudo_mounts()
        self.counts = {"entries_visited": 0, "files_searched": 0, "files_matched": 0, "binary_skipped": 0,
                       "directories_skipped": 0, "unreadable": 0}
        self.results = 0
        self.size = 0
        self.reported = time.monotonic()

    def send(self, record):
        os.write(self.out, json.dumps(record).encode() + b"\n")

    def check(self):
        now = time.monotonic()
        if now >= self.deadline:
            raise Stop("time")
        if now - self.reported >= 0.2:
            self.reported = now
            self.send({"counts": self.counts})

    def result(self, entry):
        if self.results >= self.limit:
            raise Stop("results")
        self.size += len(json.dumps(entry))
        if self.size > ANSWER:
            raise Stop("size")
        self.results += 1
        self.send({"result": entry})

    def named(self, name):
        return self.name is None or self.name.match(name) is not None

    def entry(self, path, name, info):
        """One entry of a name search: what it is, and its size."""
        if self.pattern is None and self.name.match(name) is not None:
            self.result({"path": shown(path), "type": kind(info), "size": info.st_size})

    def contents(self, folder, name, path):
        """Search one regular file's lines, opened through its folder without following a link."""
        try:
            descriptor = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=folder)
        except OSError:
            self.counts["unreadable"] += 1
            return
        try:
            if not stat.S_ISREG(os.fstat(descriptor).st_mode):
                return
            data = b""
            while len(data) <= FILE_BYTES:
                chunk = os.read(descriptor, min(1 << 20, FILE_BYTES + 1 - len(data)))
                if not chunk:
                    break
                data += chunk
        except OSError:
            self.counts["unreadable"] += 1
            return
        finally:
            os.close(descriptor)
        if b"\0" in data[:PROBE]:
            self.counts["binary_skipped"] += 1
            return
        self.counts["files_searched"] += 1
        partial = len(data) > FILE_BYTES
        text = data[:FILE_BYTES].decode("utf-8", "replace")
        if self.literal is not None and self.literal not in text:
            return
        lines = text.split("\n")
        if partial and len(lines) > 1:
            # The last line was cut by the limit, not by the file.
            lines.pop()
        found = False
        for number, line in enumerate(lines):
            if number % 2000 == 0:
                self.check()
            match = self.pattern.search(line)
            if match is None:
                continue
            if not found:
                found = True
                self.counts["files_matched"] += 1
            part, shortened = cut(line, match.start())
            entry = {"path": shown(path), "line": number + 1, "column": match.start() + 1, "text": part}
            if shortened:
                entry["cut"] = True
            if self.context:
                entry["before"] = [cut(other)[0] for other in lines[max(0, number - self.context):number]]
                entry["after"] = [cut(other)[0] for other in lines[number + 1:number + 1 + self.context]]
            self.result(entry)

    def visit(self):
        self.counts["entries_visited"] += 1
        if self.counts["entries_visited"] > ENTRIES:
            self.counts["entries_visited"] = ENTRIES
            raise Stop("entries")
        self.check()

    def folder(self, descriptor, path, depth):
        """Search a folder: its files first, by name, then the folders in it."""
        try:
            with os.scandir(descriptor) as listing:
                # No more names than the entries still allowed, so a huge folder stays bounded.
                room = ENTRIES - self.counts["entries_visited"] + 1
                names = sorted(entry.name for entry in itertools.islice(listing, room))
        except OSError:
            self.counts["unreadable"] += 1
            return
        folders = []
        for name in names:
            self.visit()
            child = path.rstrip("/") + "/" + name
            try:
                info = os.lstat(name, dir_fd=descriptor)
            except OSError:
                self.counts["unreadable"] += 1
                continue
            self.entry(child, name, info)
            if stat.S_ISDIR(info.st_mode):
                folders.append((name, child))
            elif stat.S_ISREG(info.st_mode) and self.pattern is not None and self.named(name):
                self.contents(descriptor, name, child)
        for name, child in folders:
            if depth >= DEPTH or (not self.include and name in HEAVY) or child in self.pseudo:
                self.counts["directories_skipped"] += 1
                continue
            try:
                inner = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=descriptor)
            except OSError:
                self.counts["unreadable"] += 1
                continue
            try:
                self.folder(inner, child, depth + 1)
            finally:
                os.close(inner)

    def run(self, path):
        parent, name = os.path.split(path.rstrip("/") or "/")
        descriptor = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
        try:
            for part in filter(None, parent.split("/")):
                inner = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=descriptor)
                os.close(descriptor)
                descriptor = inner
            if not name:
                self.folder(descriptor, "/", 0)
                return
            info = os.lstat(name, dir_fd=descriptor)
            if stat.S_ISDIR(info.st_mode):
                inner = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=descriptor)
                try:
                    self.folder(inner, path, 0)
                finally:
                    os.close(inner)
            elif stat.S_ISREG(info.st_mode):
                # A file as the root is searched alone, whatever its name.
                self.visit()
                self.entry(path, name, info)
                if self.pattern is not None:
                    self.contents(descriptor, name, path)
            else:
                raise ValueError("path must be a folder or a regular file, and no symlink")
        finally:
            os.close(descriptor)


def search(request, out, deadline):
    """The child: send results, counts now and then, and at last how the search ended."""
    try:
        work = Search(request, out, deadline)
        try:
            work.run(request["path"])
            ended = None
        except Stop as stop:
            ended = stop.args[0]
        work.send({"counts": work.counts})
        work.send({"end": ended})
    except (OSError, ValueError) as error:
        os.write(out, json.dumps({"error": str(error)}).encode() + b"\n")


def read_request():
    """The first line of stdin, read from the descriptor so nothing after it is taken."""
    line = b""
    while not line.endswith(b"\n"):
        if len(line) > REQUEST:
            raise ValueError("the request is too long")
        byte = os.read(0, 1)
        if not byte:
            break
        line += byte
    request = json.loads(line)
    if not isinstance(request, dict) or not isinstance(request.get("path"), str):
        raise ValueError("the request needs a path")
    if not isinstance(request.get("pattern"), str) and not isinstance(request.get("name"), str):
        raise ValueError("the request needs a pattern or a name")
    request["max_results"] = max(1, min(int(request.get("max_results", 100)), 1000))
    request["context"] = max(0, min(int(request.get("context", 0)), 10))
    request["max_seconds"] = max(0.1, min(float(request.get("max_seconds", 10)), 30.0))
    return request


def main():
    try:
        request = read_request()
    except (ValueError, OSError) as error:
        print("Search failed: " + str(error), file=sys.stderr)
        return 1
    started = time.monotonic()
    deadline = started + request["max_seconds"]
    receive, send = os.pipe()
    child = os.fork()
    if child == 0:
        status = 1
        try:
            os.close(receive)
            search(request, send, deadline)
            status = 0
        finally:
            os._exit(status)
    os.close(send)
    results, counts, state = [], {}, {"ended": False, "by": None, "error": None}

    def take(line):
        record = json.loads(line)
        if "result" in record:
            results.append(record["result"])
        elif "counts" in record:
            counts.update(record["counts"])
        elif "error" in record:
            state["error"] = record["error"]
        elif "end" in record:
            state["ended"], state["by"] = True, record["end"]

    pending, watched, stopped = b"", [receive, 0], None
    while True:
        left = deadline + GRACE - time.monotonic()
        if left <= 0:
            stopped = "time"
            break
        ready, _, _ = select.select(watched, [], [], left)
        if 0 in ready and not os.read(0, 65536):
            # The caller is gone: nobody reads an answer.
            stopped = "caller"
            break
        if receive in ready:
            chunk = os.read(receive, 65536)
            if not chunk:
                break
            *lines, pending = (pending + chunk).split(b"\n")
            for line in lines:
                take(line)
    if stopped:
        try:
            os.kill(child, signal.SIGKILL)
        except ProcessLookupError:
            pass
    _, status = os.waitpid(child, 0)
    if stopped == "caller":
        print("Search failed: the caller stopped waiting", file=sys.stderr)
        return 1
    if state["error"] is not None:
        print("Search failed: " + state["error"], file=sys.stderr)
        return 1
    if stopped is None and not state["ended"]:
        print(f"Search failed: the search ended unexpectedly (status {status})", file=sys.stderr)
        return 3
    by = "time" if stopped else state["by"]
    answer = {"path": request["path"]}
    answer["matches" if isinstance(request.get("pattern"), str) else "files"] = results
    for key in ("entries_visited", "files_searched", "files_matched", "binary_skipped", "directories_skipped",
                "unreadable"):
        answer[key] = counts.get(key, 0)
    answer["truncated"] = by is not None
    answer["truncated_by"] = by
    answer["elapsed_ms"] = int((time.monotonic() - started) * 1000)
    print(json.dumps(answer))
    return 0


if __name__ == "__main__":
    sys.exit(main())
