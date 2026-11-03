"""Describe, list, move, delete or create files as the desktop user without following symlinks in paths."""
import json
import os
import stat
import sys


def open_dir(path):
    """Open a directory by walking each component with O_NOFOLLOW."""
    descriptor = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    for part in filter(None, path.split("/")):
        child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=descriptor)
        os.close(descriptor)
        descriptor = child
    return descriptor


def describe(name, info):
    if stat.S_ISDIR(info.st_mode):
        kind = "directory"
    elif stat.S_ISLNK(info.st_mode):
        kind = "symlink"
    elif stat.S_ISREG(info.st_mode):
        kind = "file"
    else:
        kind = "special"
    return {"name": name, "type": kind, "size": info.st_size, "modified_at": int(info.st_mtime),
            "mode": stat.S_IMODE(info.st_mode)}


def stat_file(path):
    parent, name = os.path.split(path)
    descriptor = open_dir(parent)
    try:
        entry = {"path": path, **describe(name, os.lstat(name, dir_fd=descriptor))}
        if entry["type"] == "file":
            # Whether a download would get past opening it, told before any headers are sent.
            try:
                os.close(os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=descriptor))
                entry["readable"] = True
            except OSError:
                entry["readable"] = False
        return entry
    finally:
        os.close(descriptor)


# The most paths one stats call describes.
STATS_LIMIT = 1000


def stat_paths(paths):
    """Describe each of the paths, or give None for one that is gone or lies behind a symlink."""
    if not isinstance(paths, list) or len(paths) > STATS_LIMIT or not all(isinstance(path, str) for path in paths):
        raise ValueError(f"stats takes a JSON list of at most {STATS_LIMIT} paths")
    described = []
    for path in paths:
        parent, name = os.path.split(path)
        try:
            descriptor = open_dir(parent)
        except OSError:
            described.append(None)
            continue
        try:
            entry = describe(name, os.lstat(name, dir_fd=descriptor))
            del entry["name"]
            described.append(entry)
        except OSError:
            described.append(None)
        finally:
            os.close(descriptor)
    return described


def list_files(path, limit):
    descriptor = open_dir(path)
    try:
        names = sorted(os.listdir(descriptor))
        items = []
        for name in names[:limit]:
            try:
                items.append(describe(name, os.lstat(name, dir_fd=descriptor)))
            except OSError:
                continue
        return {"path": path, "items": items, "total": len(names), "truncated": len(names) > limit}
    finally:
        os.close(descriptor)


def move_file(source, destination, replace):
    source_dir, source_name = os.path.split(source)
    destination_dir, destination_name = os.path.split(destination)
    source_fd = open_dir(source_dir)
    try:
        destination_fd = open_dir(destination_dir)
        try:
            os.lstat(source_name, dir_fd=source_fd)
            if not replace:
                try:
                    os.lstat(destination_name, dir_fd=destination_fd)
                except FileNotFoundError:
                    pass
                else:
                    print("CONFLICT: Destination already exists. Set replace=true to overwrite it.", file=sys.stderr)
                    sys.exit(17)
            try:
                os.rename(source_name, destination_name, src_dir_fd=source_fd, dst_dir_fd=destination_fd)
            except OSError as error:
                if error.errno == 18:
                    raise ValueError("Moving between different volumes is not supported; copy the file with a script instead")
                raise
            return {"source": source, "destination": destination}
        finally:
            os.close(destination_fd)
    finally:
        os.close(source_fd)


def remove_tree(descriptor):
    for name in os.listdir(descriptor):
        info = os.lstat(name, dir_fd=descriptor)
        if stat.S_ISDIR(info.st_mode):
            child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=descriptor)
            try:
                remove_tree(child)
            finally:
                os.close(child)
            os.rmdir(name, dir_fd=descriptor)
        else:
            os.unlink(name, dir_fd=descriptor)


def delete_file(path, recursive):
    parent, name = os.path.split(path)
    descriptor = open_dir(parent)
    try:
        info = os.lstat(name, dir_fd=descriptor)
        if stat.S_ISDIR(info.st_mode):
            if recursive:
                child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=descriptor)
                try:
                    remove_tree(child)
                finally:
                    os.close(child)
            os.rmdir(name, dir_fd=descriptor)
        else:
            os.unlink(name, dir_fd=descriptor)
        return {"path": path, "deleted": True}
    finally:
        os.close(descriptor)


def make_directory(path):
    parent, name = os.path.split(path)
    descriptor = open_dir(parent)
    try:
        try:
            os.mkdir(name, 0o755, dir_fd=descriptor)
        except FileExistsError:
            print("CONFLICT: Something with this name already exists.", file=sys.stderr)
            sys.exit(17)
        return {"path": path, "created": True}
    finally:
        os.close(descriptor)


command, *arguments = sys.argv[1:]
try:
    if command == "stat":
        print(json.dumps(stat_file(arguments[0])))
    elif command == "list":
        print(json.dumps(list_files(arguments[0], int(arguments[1]))))
    elif command == "move":
        print(json.dumps(move_file(arguments[0], arguments[1], arguments[2] == "true")))
    elif command == "delete":
        print(json.dumps(delete_file(arguments[0], arguments[1] == "true")))
    elif command == "mkdir":
        print(json.dumps(make_directory(arguments[0])))
    elif command == "stats":
        # The paths come on stdin, since a name may hold anything but a NUL.
        print(json.dumps(stat_paths(json.loads(sys.stdin.buffer.read()))))
    else:
        sys.exit(2)
except SystemExit:
    raise
except Exception as error:
    print("File operation failed: " + str(error), file=sys.stderr)
    sys.exit(1)
