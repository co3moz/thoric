"""The folders of a compose's volume parameters: created where missing, handed to the desktop user
while root owns them and nothing else is in them, kept as they are once they hold anything, and
never reached through a symlink."""

import errno
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

PATHS = Path(__file__).resolve().parents[1] / "helpers/setup_paths.py"
spec = importlib.util.spec_from_file_location("setup_paths", PATHS)
paths = importlib.util.module_from_spec(spec)
spec.loader.exec_module(paths)

DESKTOP = 4242


class Owners:
    """Owners as root would see and change them, for folders the test's user cannot give away."""

    def __init__(self, owned):
        self.by_inode = {os.stat(folder).st_ino: uid for folder, uid in owned.items()}
        self.real = os.fstat
        self.handed = 0

    def fstat(self, fd):
        result = self.real(fd)
        uid = self.by_inode.get(result.st_ino, result.st_uid)
        return os.stat_result((*result[:4], uid, uid, *result[6:10]))

    def fchown(self, fd, uid, gid):
        assert (uid, gid) == (DESKTOP, DESKTOP), (uid, gid)
        self.by_inode[self.real(fd).st_ino] = uid
        self.handed += 1


class PrepareTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp()).resolve()
        self.addCleanup(shutil.rmtree, self.root)

    def prepare(self, path, owned):
        """Prepares `path` with these owners and returns how many folders were handed over."""
        owners = Owners(owned)
        with mock.patch.object(paths.os, "fstat", owners.fstat), mock.patch.object(paths.os, "fchown", owners.fchown):
            paths.prepare(str(path), DESKTOP)
        return owners.handed

    def test_missing_folders_are_created_with_the_owner_of_their_parent(self):
        target = self.root / "unity-install-location" / "unity_v1"
        # The test's own user stands in for the desktop user, so nothing is handed over.
        paths.prepare(str(target), os.getuid())
        self.assertTrue(target.is_dir())
        for folder in (target, target.parent):
            info = folder.stat()
            self.assertEqual((info.st_uid, info.st_gid), (os.getuid(), os.getgid()))
            self.assertEqual(info.st_mode & 0o777, 0o755)

    def test_an_empty_mount_point_root_owns_goes_to_the_desktop_user(self):
        target = self.root / "volume"
        target.mkdir()
        self.assertEqual(self.prepare(target, {target: 0}), 1)

    def test_a_folder_that_holds_anything_keeps_its_owner(self):
        target = self.root / "volume"
        (target / "Editor").mkdir(parents=True)
        self.assertEqual(self.prepare(target, {target: 0}), 0)

    def test_folders_docker_made_above_a_mount_point_in_the_home_are_handed_over(self):
        home = self.root / "home"
        hub = home / "Unity" / "Hub"
        target = hub / "Editor"
        target.mkdir(parents=True)
        owned = {home: DESKTOP, home / "Unity": 0, hub: 0, target: 0}
        self.assertEqual(self.prepare(target, owned), 3)
        # A root folder with other files in it, or outside the desktop user's folders, stays.
        (hub / "notes").write_text("")
        self.assertEqual(self.prepare(target, owned), 2)
        self.assertEqual(self.prepare(target, {**owned, home: 0}), 1)

    def test_a_read_only_mount_point_keeps_its_owner(self):
        # A scale set's instance after the first mounts the volume only instance 1 writes
        # read-only; its setup still runs, and there is nothing to hand over.
        target = self.root / "volume"
        target.mkdir()
        owners = Owners({target: 0})

        def read_only(fd, uid, gid):
            raise OSError(errno.EROFS, "Read-only file system")

        with mock.patch.object(paths.os, "fstat", owners.fstat), mock.patch.object(paths.os, "fchown", read_only):
            paths.prepare(str(target), DESKTOP)

        # Any other failure still stops the setup.
        def denied(fd, uid, gid):
            raise OSError(errno.EACCES, "Permission denied")

        with mock.patch.object(paths.os, "fstat", owners.fstat), mock.patch.object(paths.os, "fchown", denied):
            with self.assertRaises(OSError):
                paths.prepare(str(target), DESKTOP)

    def test_a_symlink_on_the_way_is_refused(self):
        (self.root / "elsewhere").mkdir()
        (self.root / "link").symlink_to(self.root / "elsewhere")
        with self.assertRaises(OSError):
            paths.prepare(str(self.root / "link" / "unity"))
        self.assertFalse((self.root / "elsewhere" / "unity").exists())

    def test_the_manager_passes_the_paths_as_arguments_and_reads_one_line_on_failure(self):
        target = self.root / "a" / "b"
        (self.root / "file").write_text("")
        # The manager runs the helper with `python3 -c`, so it must work without its file name.
        result = subprocess.run(
            [sys.executable, "-c", PATHS.read_text(), str(target), str(self.root / "file" / "x")],
            capture_output=True,
            timeout=30,
        )
        self.assertEqual(result.returncode, 1, result)
        self.assertTrue(target.is_dir())
        self.assertEqual(result.stdout, b"")
        self.assertEqual(len(result.stderr.splitlines()), 1, result.stderr)
        self.assertIn(b"Could not prepare", result.stderr)


if __name__ == "__main__":
    unittest.main()
