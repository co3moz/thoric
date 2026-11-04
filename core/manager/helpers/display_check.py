"""Whether the desktop's X display answers, and which programs are connected to it when it does not.

The manager runs this as the desktop user once an X command of a desktop has taken longer than it
should. A program can hold an X server with a server grab, as ImageMagick's import does while it
waits for a window to be clicked, which it falls back to when the window it was given is gone.
Until it lets go, the server serves that program alone, and every other program that connects, the
health check's xdpyinfo, the window tools' xdotool and wmctrl and the screenshot's xwd, waits
without an error.

Whether the display answers is one connection setup within a deadline. It needs no X library and no
authorization, since any reply, a refusal too, is an answer. Which programs are connected comes
from the kernel: sock_diag lists the Unix sockets the display serves its connections on, each with
the peer at the program's end and the bytes the server has not read yet, and the processes are the
desktop user's that hold those peers. A program whose connection has unread bytes is one the display
keeps waiting, so the one that holds it is among the others, and usually the newest of them.

stdout carries one JSON document: {"display": "answers"}, {"display": "gone"} when nothing answers
on the display, or {"display": "held", "clients": [...]} with the connected programs, newest first.
"""
import json
import os
import socket
import struct
import sys
import time

# The display every desktop runs, :0, by its abstract name first as Xlib connects.
DISPLAY = "/tmp/.X11-unix/X0"
ADDRESSES = ("\0" + DISPLAY, DISPLAY)
# X11's connection setup in little-endian order: protocol 11.0 and no authorization.
SETUP = struct.pack("<BxHHHHxx", ord("l"), 11, 0, 0, 0)
# Where the manager supervises the applications it launches, one folder each with the session
# leader's pid (apps.rs).
APPLICATIONS = "/tmp/thoric-apps"
CLIENT_LIMIT = 32

NETLINK_SOCK_DIAG = 4
SOCK_DIAG_BY_FAMILY = 20
NLM_F_REQUEST = 0x1
NLM_F_DUMP = 0x300
NLMSG_ERROR = 2
NLMSG_DONE = 3
UDIAG_SHOW_NAME = 0x1
UDIAG_SHOW_PEER = 0x4
UDIAG_SHOW_RQLEN = 0x10
UNIX_DIAG_NAME = 0
UNIX_DIAG_PEER = 2
UNIX_DIAG_RQLEN = 4
ESTABLISHED = 1


def answers(addresses=ADDRESSES, deadline=2.0):
    """`answers` when a connection setup gets a reply within the deadline, `held` when the display
    took the connection and said nothing, and `gone` when nothing takes it."""
    for address in addresses:
        connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        connection.settimeout(deadline)
        try:
            try:
                connection.connect(address)
            except OSError:
                continue
            try:
                connection.sendall(SETUP)
                return "answers" if connection.recv(1) else "gone"
            except socket.timeout:
                return "held"
            except OSError:
                return "gone"
        finally:
            connection.close()
    return "gone"


def aligned(length):
    return (length + 3) & ~3


def messages(data):
    """The netlink messages in one datagram, as (type, payload)."""
    offset = 0
    while offset + 16 <= len(data):
        length, kind = struct.unpack_from("=IH", data, offset)
        if length < 16:
            break
        yield kind, data[offset + 16:offset + length]
        offset += aligned(length)


def unix_socket(payload):
    """One unix_diag_msg with its name, peer and receive queue."""
    _, _, state, inode = struct.unpack_from("=BBBxI", payload)
    found = {"inode": inode, "state": state, "name": b"", "peer": 0, "unread": 0}
    offset = 16
    while offset + 4 <= len(payload):
        length, kind = struct.unpack_from("=HH", payload, offset)
        if length < 4:
            break
        value = payload[offset + 4:offset + length]
        if kind == UNIX_DIAG_NAME:
            found["name"] = bytes(value)
        elif kind == UNIX_DIAG_PEER and len(value) >= 4:
            found["peer"] = struct.unpack_from("=I", value)[0]
        elif kind == UNIX_DIAG_RQLEN and len(value) >= 4:
            found["unread"] = struct.unpack_from("=I", value)[0]
        offset += aligned(length)
    return found


def unix_sockets():
    """Every Unix socket of the network namespace, from the kernel's sock_diag."""
    request = struct.pack("=BBHIIIII", socket.AF_UNIX, 0, 0, 0xFFFFFFFF, 0,
                          UDIAG_SHOW_NAME | UDIAG_SHOW_PEER | UDIAG_SHOW_RQLEN, 0xFFFFFFFF, 0xFFFFFFFF)
    header = struct.pack("=IHHII", 16 + len(request), SOCK_DIAG_BY_FAMILY, NLM_F_REQUEST | NLM_F_DUMP, 1, 0)
    found = []
    with socket.socket(socket.AF_NETLINK, socket.SOCK_RAW, NETLINK_SOCK_DIAG) as diag:
        diag.settimeout(5)
        diag.send(header + request)
        while True:
            data = diag.recv(1 << 17)
            if not data:
                return found
            for kind, payload in messages(data):
                if kind == NLMSG_DONE:
                    return found
                if kind == NLMSG_ERROR:
                    raise OSError("sock_diag refused the request")
                found.append(unix_socket(payload))


def served(sockets, names):
    """The connections a server listening on one of `names` serves, as {the peer's inode: whether
    the server has bytes from it it has not read}. An accepted socket carries its listener's name."""
    wanted = {name.encode() for name in names}
    return {item["peer"]: item["unread"] > 0 for item in sockets
            if item["state"] == ESTABLISHED and item["peer"] and item["name"].rstrip(b"\0") in wanted}


def owners(inodes, proc="/proc"):
    """The processes holding each socket inode, among those this user can read: {pid: inodes}."""
    held = {}
    for pid in os.listdir(proc):
        if not pid.isdigit() or int(pid) == os.getpid():
            continue
        folder = os.path.join(proc, pid, "fd")
        try:
            descriptors = os.listdir(folder)
        except OSError:
            continue
        for descriptor in descriptors:
            try:
                target = os.readlink(os.path.join(folder, descriptor))
            except OSError:
                continue
            if target.startswith("socket:[") and int(target[8:-1]) in inodes:
                held.setdefault(int(pid), set()).add(int(target[8:-1]))
    return held


def boot_time(proc="/proc"):
    with open(os.path.join(proc, "stat")) as handle:
        for line in handle:
            if line.startswith("btime "):
                return int(line.split()[1])
    return 0


def process(pid, proc="/proc"):
    """A process's name, session and start time in seconds since the epoch. Its command line stays
    out, since it may carry what the desktop user typed."""
    with open(os.path.join(proc, str(pid), "stat")) as handle:
        text = handle.read()
    name_end = text.rfind(")")
    fields = text[name_end + 2:].split()
    started = boot_time(proc) + int(fields[19]) / os.sysconf("SC_CLK_TCK")
    return {"pid": pid, "command": text[text.find("(") + 1:name_end], "session": int(fields[3]),
            "started_at": started}


def applications(root=APPLICATIONS):
    """The applications the manager launched, by the session their runner started them in."""
    found = {}
    try:
        names = os.listdir(root)
    except OSError:
        return found
    for name in names:
        try:
            with open(os.path.join(root, name, "pid")) as handle:
                found[int(handle.read().strip())] = name
        except (OSError, ValueError):
            continue
    return found


def clients(names=ADDRESSES, proc="/proc", root=APPLICATIONS, now=None):
    """The programs connected to the display, newest first. A program waits when every connection
    it holds has bytes the server has not read."""
    now = time.time() if now is None else now
    connections = served(unix_sockets(), names)
    launched = applications(root)
    found = []
    for pid, inodes in owners(set(connections), proc).items():
        try:
            info = process(pid, proc)
        except (OSError, ValueError, IndexError):
            continue
        found.append((-info["started_at"], -pid, {
            "pid": pid,
            "command": info["command"],
            "age_seconds": max(0, int(now - info["started_at"])),
            "waiting": all(connections[inode] for inode in inodes),
            "application_id": launched.get(info["session"]),
        }))
    found.sort(key=lambda item: item[:2])
    return [client for _, _, client in found[:CLIENT_LIMIT]]


def main(arguments):
    deadline = float(arguments[0]) if arguments else 2.0
    state = answers(deadline=deadline)
    result = {"display": state}
    if state == "held":
        try:
            result["clients"] = clients()
        except OSError as error:
            result["clients"] = []
            result["error"] = "Could not list the display's connections: %s" % error
    sys.stdout.write(json.dumps(result) + "\n")


if __name__ == "__main__":
    main(sys.argv[1:])
