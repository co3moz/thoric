#!/usr/bin/env python3
"""Register MCP servers of this desktop with Thoric.

Thoric starts a registered server as the desktop user when an agent needs it, and offers its
tools on the MCP endpoints that cover this container. The image's own servers are in
/etc/thoric/mcp.d; this command manages the desktop user's, in ~/.config/thoric/mcp.
The tray's MCP servers window runs this command too, and shows the command line it runs.
"""
import argparse
import json
import os
import re
import sys

IMAGE = "/etc/thoric/mcp.d"
FOLDER = "/home/desktop/.config/thoric/mcp"
# What the manager last said about these servers and the endpoints that reach this desktop. It is
# there only while the manager watches the desktop.
STATUS = "/run/desktop/thoric/mcp-status.json"
STATUS_LIMIT = 16 << 20
NAME = re.compile(r"^[a-z0-9][a-z0-9-]{0,31}$")
# The manager checks a declaration again; these catch the usual mistakes before it is written.
LOOPBACK = re.compile(r"^https?://(?:127\.0\.0\.1|localhost|\[::1\]):(\d{1,5}|\$\{[^}/]+\})(?:/\S*)?$")
VARIABLE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]{0,127}$")
HEADER = re.compile(r"^[A-Za-z0-9-]{1,64}$")
USAGE = """thoric-mcp add NAME [--description TEXT] [--env KEY=VALUE]... [--cwd DIR] -- COMMAND [ARG]...
       thoric-mcp add NAME --url http://127.0.0.1:PORT/mcp [--header 'Name: value']... [--description TEXT]
       thoric-mcp remove NAME
       thoric-mcp list [--json]
       thoric-mcp show NAME"""


def folder(path, create):
    """Open a folder without following a symlink on the way, creating it when asked."""
    descriptor = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in path.strip("/").split("/"):
            try:
                child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=descriptor)
            except FileNotFoundError:
                if not create:
                    raise
                os.mkdir(part, 0o700, dir_fd=descriptor)
                child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=descriptor)
            os.close(descriptor)
            descriptor = child
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def declaration(arguments):
    spec = {}
    if arguments.description:
        spec["description"] = arguments.description
    if arguments.url:
        if arguments.command or arguments.env or arguments.cwd:
            raise ValueError("--url is for an HTTP server; a command takes --env and --cwd")
        port = LOOPBACK.match(arguments.url)
        if not port or (port.group(1).isdigit() and not 0 < int(port.group(1)) < 65536):
            raise ValueError("An HTTP server's url is http(s)://127.0.0.1, localhost or [::1] with a "
                             "port and a path, such as http://127.0.0.1:8931/mcp")
        spec["url"] = arguments.url
        headers = {}
        for header in arguments.header:
            name, separator, value = header.partition(":")
            if not separator or not name.strip():
                raise ValueError("A header is 'Name: value': " + header)
            if not HEADER.match(name.strip()):
                raise ValueError("Invalid header name: " + name.strip())
            headers[name.strip()] = value.strip()
        if headers:
            spec["headers"] = headers
        return spec
    if arguments.header:
        raise ValueError("--header is for an HTTP server added with --url")
    if not arguments.command:
        raise ValueError("Give the server's command after --, or its --url")
    spec["command"] = arguments.command[0]
    if len(arguments.command) > 1:
        spec["args"] = arguments.command[1:]
    env = {}
    for variable in arguments.env:
        key, separator, value = variable.partition("=")
        if not separator or not key:
            raise ValueError("An environment variable is KEY=VALUE: " + variable)
        if not VARIABLE.match(key):
            raise ValueError("Invalid environment variable name: " + key)
        env[key] = value
    if env:
        spec["env"] = env
    if arguments.cwd:
        spec["cwd"] = arguments.cwd
    return spec


def add(arguments, target):
    if not NAME.match(arguments.name):
        raise ValueError("A server name is lowercase letters, digits and dashes, at most 32")
    data = (json.dumps(declaration(arguments), indent=2) + "\n").encode("utf-8")
    directory = folder(target, create=True)
    try:
        temporary = ".%s.%d.tmp" % (arguments.name, os.getpid())
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600,
                             dir_fd=directory)
        try:
            with os.fdopen(descriptor, "wb") as file:
                file.write(data)
            os.rename(temporary, arguments.name + ".json", src_dir_fd=directory, dst_dir_fd=directory)
        except BaseException:
            try:
                os.unlink(temporary, dir_fd=directory)
            except OSError:
                pass
            raise
    finally:
        os.close(directory)
    print("Registered %s. Thoric reads its tools within a few seconds." % arguments.name)


def remove(arguments, target):
    if not NAME.match(arguments.name):
        raise ValueError("A server name is lowercase letters, digits and dashes, at most 32")
    directory = folder(target, create=False)
    try:
        os.unlink(arguments.name + ".json", dir_fd=directory)
    finally:
        os.close(directory)
    print("Removed %s." % arguments.name)


def servers(image, target):
    """Both folders' declarations, the image's first: a spec each, or why it could not be read.
    The desktop user's declaration of a name replaces the image's, which is then not active."""
    rows = []
    for source, path in (("image", image), ("container", target)):
        try:
            directory = folder(path, create=False)
        except OSError:
            continue
        try:
            for entry in sorted(os.listdir(directory)):
                if not entry.endswith(".json") or entry.startswith("."):
                    continue
                row = {"name": entry[:-5], "source": source}
                try:
                    descriptor = os.open(entry, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=directory)
                    with os.fdopen(descriptor, "rb") as file:
                        spec = json.loads(file.read(65537))
                    if not isinstance(spec, dict):
                        raise ValueError
                    row["spec"] = spec
                except (OSError, ValueError):
                    row["error"] = "Cannot read " + entry
                rows.append(row)
        finally:
            os.close(directory)
    mine = {row["name"] for row in rows if row["source"] == "container"}
    for row in rows:
        row["active"] = row["source"] == "container" or row["name"] not in mine
    return rows


def status(path):
    """What the manager last said, or None while it does not watch this desktop."""
    head, _, name = path.rpartition("/")
    try:
        directory = folder(head, create=False)
    except OSError:
        return None
    try:
        descriptor = os.open(name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=directory)
        with os.fdopen(descriptor, "rb") as file:
            document = json.loads(file.read(STATUS_LIMIT + 1))
        return document if isinstance(document, dict) else None
    except (OSError, ValueError):
        return None
    finally:
        os.close(directory)


def shown(spec):
    """What a server runs, in one line."""
    try:
        return spec.get("url") or " ".join([spec.get("command", "")] + list(spec.get("args", [])))
    except (AttributeError, TypeError):
        return "(unreadable)"


def listing(image, target, as_json, status_path):
    rows = servers(image, target)
    if as_json:
        known = status(status_path)
        states = {}
        if known is not None:
            for server in known.get("servers") or []:
                if isinstance(server, dict) and isinstance(server.get("name"), str):
                    states[server["name"]] = {key: server.get(key) for key in
                                              ("state", "runs_in", "error", "tools", "tools_at",
                                               "log")}
        for row in rows:
            if row["active"] and row["name"] in states:
                row["status"] = states[row["name"]]
        document = {"servers": rows, "watched": known is not None}
        if known is not None:
            document["status_at"] = known.get("status_at")
            document["endpoints"] = known.get("endpoints") or []
        print(json.dumps(document, indent=2))
        return
    for row in rows:
        print("%-24s %-10s %s" % (row["name"], row["source"],
                                 shown(row["spec"]) if "spec" in row else "(unreadable)"))
    if not rows:
        print("No MCP servers are registered in this desktop.")


def show(arguments, image, target):
    """The active declaration of a name, as a template's or a build's mcp list holds it."""
    for row in servers(image, target):
        if row["name"] == arguments.name and row["active"]:
            if "spec" not in row:
                raise ValueError(row["error"])
            print(json.dumps(dict({"name": row["name"]}, **row["spec"]), indent=2))
            return
    raise FileNotFoundError(arguments.name)


def main(argv, image=IMAGE, target=FOLDER, uid=None, status_path=STATUS):
    parser = argparse.ArgumentParser(prog="thoric-mcp", usage=USAGE, description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    adding = commands.add_parser("add")
    adding.add_argument("name")
    adding.add_argument("--description", default="")
    adding.add_argument("--env", action="append", default=[])
    adding.add_argument("--cwd")
    adding.add_argument("--url")
    adding.add_argument("--header", action="append", default=[])
    removing = commands.add_parser("remove")
    removing.add_argument("name")
    listing_parser = commands.add_parser("list")
    listing_parser.add_argument("--json", action="store_true")
    showing = commands.add_parser("show")
    showing.add_argument("name")
    # Everything after -- is the server's command line, whatever options it has.
    program = []
    if "--" in argv:
        at = argv.index("--")
        argv, program = argv[:at], argv[at + 1:]
    arguments = parser.parse_args(argv)
    arguments.command = program
    if (os.geteuid() if uid is None else uid) == 0 and arguments.action in ("add", "remove"):
        raise ValueError("Run thoric-mcp as the desktop user; the servers run as that user")
    if arguments.action == "add":
        add(arguments, target)
    elif arguments.action == "remove":
        remove(arguments, target)
    elif arguments.action == "show":
        show(arguments, image, target)
    else:
        listing(image, target, arguments.json, status_path)


if __name__ == "__main__":
    try:
        main(sys.argv[1:])
    except FileNotFoundError:
        sys.stderr.write("thoric-mcp: no such server\n")
        sys.exit(1)
    except (OSError, ValueError) as error:
        sys.stderr.write("thoric-mcp: %s\n" % error)
        sys.exit(1)
