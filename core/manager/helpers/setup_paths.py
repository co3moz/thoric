"""Prepare the folders of a compose's volume parameters before its setup script runs.

Run as root inside the desktop with the paths as arguments. Each path becomes a folder the desktop
user can write. A missing folder is created with the owner of the folder it is created in. The
last one is handed to the desktop user while it is owned by root and empty, which is what Docker
leaves at a new volume's mount point, and what the path is when no volume was chosen and the image
lacks it. Docker also creates the missing folders above a mount point, owned by root; one of those
inside a folder of the desktop user, holding nothing but the next folder of the path, is handed to
the desktop user too. A folder that holds anything else keeps its owner. No symlink on the way is
followed. A volume mounted read-only, as a scale set's instance after the first mounts one only
instance 1 writes, keeps its owner too: nothing can be written there anyway.
"""

import errno
import os
import sys

DESKTOP = 1001


def prepare(path, desktop=DESKTOP):
    parts = [part for part in path.split("/") if part]
    folder = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    try:
        for index, part in enumerate(parts):
            parent = os.fstat(folder)
            try:
                os.mkdir(part, 0o755, dir_fd=folder)
            except FileExistsError:
                pass
            else:
                os.chown(part, parent.st_uid, parent.st_gid, dir_fd=folder, follow_symlinks=False)
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=folder)
            os.close(folder)
            folder = child
            if os.fstat(folder).st_uid != 0:
                continue
            contents = os.listdir(folder)
            if index == len(parts) - 1:
                left = not contents
            else:
                left = parent.st_uid == desktop and contents == [parts[index + 1]]
            if left:
                try:
                    os.fchown(folder, desktop, desktop)
                except OSError as error:
                    if error.errno != errno.EROFS:
                        raise
    finally:
        os.close(folder)


def main(paths):
    for path in paths:
        try:
            prepare(path)
        except OSError as error:
            print(f"Could not prepare {path}: {error.strerror or error}", file=sys.stderr)
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
