"""The display check: a display answers when a connection setup gets any reply in time, is held when
it takes the connection and says nothing, and is gone when nothing takes it. sock_diag's answer
reads as the sockets with their names, peers and unread bytes, and on Linux the programs connected
to a server are found by their peers, newest first, each waiting when the server has bytes from it
it has not read, and named by the application the manager launched them as."""

import importlib.util
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest

HELPER = Path(__file__).resolve().parents[1] / "helpers/display_check.py"
spec = importlib.util.spec_from_file_location("display_check", HELPER)
display_check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(display_check)

UNIX = hasattr(socket, "AF_UNIX")


def attribute(kind, value):
    data = struct.pack("=HH", 4 + len(value), kind) + value
    return data + b"\0" * (display_check.aligned(len(data)) - len(data))


def message(kind, payload):
    data = struct.pack("=IHHII", 16 + len(payload), kind, 2, 1, 0) + payload
    return data + b"\0" * (display_check.aligned(len(data)) - len(data))


class ReadTest(unittest.TestCase):
    def test_a_sock_diag_answer_reads_as_its_sockets(self):
        name = b"\0/tmp/.X11-unix/X0"
        payload = (struct.pack("=BBBxIII", 1, 1, display_check.ESTABLISHED, 4242, 0, 0)
                   + attribute(display_check.UNIX_DIAG_NAME, name)
                   + attribute(display_check.UNIX_DIAG_PEER, struct.pack("=I", 4343))
                   + attribute(display_check.UNIX_DIAG_RQLEN, struct.pack("=II", 12, 0)))
        data = message(display_check.SOCK_DIAG_BY_FAMILY, payload) + message(display_check.NLMSG_DONE, b"")
        found = list(display_check.messages(data))
        self.assertEqual([kind for kind, _ in found], [display_check.SOCK_DIAG_BY_FAMILY, display_check.NLMSG_DONE])
        self.assertEqual(display_check.unix_socket(found[0][1]),
                         {"inode": 4242, "state": display_check.ESTABLISHED, "name": name, "peer": 4343, "unread": 12})

    def test_only_connections_the_display_serves_count_by_their_peers(self):
        sockets = [
            {"inode": 1, "state": 10, "name": b"\0/tmp/.X11-unix/X0", "peer": 0, "unread": 2},
            {"inode": 2, "state": 1, "name": b"\0/tmp/.X11-unix/X0", "peer": 20, "unread": 0},
            {"inode": 3, "state": 1, "name": b"/tmp/.X11-unix/X0\0", "peer": 30, "unread": 12},
            {"inode": 4, "state": 1, "name": b"/run/dbus", "peer": 40, "unread": 0},
            {"inode": 20, "state": 1, "name": b"", "peer": 2, "unread": 0},
        ]
        self.assertEqual(display_check.served(sockets, display_check.ADDRESSES), {20: False, 30: True})


@unittest.skipUnless(UNIX, "needs Unix sockets")
class AnswersTest(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.folder.name, "X0")

    def tearDown(self):
        self.folder.cleanup()

    def serve(self, reply):
        """A display at self.path that takes one connection and answers it with `reply`, closes it
        when `reply` is empty, and says nothing when it is None."""
        server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        server.bind(self.path)
        server.listen(1)
        self.addCleanup(server.close)
        taken = []

        def answer():
            connection, _ = server.accept()
            taken.append(connection)
            connection.recv(12)
            if reply:
                connection.sendall(reply)
            elif reply is not None:
                connection.close()

        threading.Thread(target=answer, daemon=True).start()
        self.addCleanup(lambda: [connection.close() for connection in taken])

    def test_any_reply_is_an_answer(self):
        # Failed, as a server that wants authorization says.
        self.serve(b"\0")
        self.assertEqual(display_check.answers((self.path,), 2), "answers")

    def test_a_display_that_takes_the_connection_and_says_nothing_is_held(self):
        self.serve(None)
        started = time.monotonic()
        self.assertEqual(display_check.answers((self.path,), 0.3), "held")
        self.assertLess(time.monotonic() - started, 2)

    def test_nothing_to_connect_to_or_a_closed_connection_is_gone(self):
        self.assertEqual(display_check.answers((self.path,), 0.3), "gone")
        self.serve(b"")
        self.assertEqual(display_check.answers((self.path,), 2), "gone")


@unittest.skipUnless(sys.platform.startswith("linux"), "reads sock_diag and /proc")
class ClientsTest(unittest.TestCase):
    def test_the_programs_connected_to_a_display_are_found_newest_first(self):
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "X0")
            server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            server.bind(path)
            server.listen(8)
            self.addCleanup(server.close)
            connect = ("import socket, sys, time\n"
                       "s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1])\n"
                       "s.sendall(sys.argv[2].encode()); print('connected', flush=True); time.sleep(60)\n")
            started = []
            for data in ("", "setup"):
                program = subprocess.Popen([sys.executable, "-c", connect, path, data], stdout=subprocess.PIPE,
                                           start_new_session=True)
                # Cleanups run last first: kill it, wait for it, then close its pipe.
                self.addCleanup(program.stdout.close)
                self.addCleanup(program.wait)
                self.addCleanup(program.kill)
                self.assertEqual(program.stdout.readline().strip(), b"connected")
                started.append(program)
                # Start times count in clock ticks, so the second program starts in a later one.
                time.sleep(0.05)
            taken = [server.accept()[0] for _ in started]
            self.addCleanup(lambda: [connection.close() for connection in taken])
            # The server has read nothing, so the second program waits for it.
            applications = os.path.join(folder, "apps")
            os.makedirs(os.path.join(applications, "first"))
            with open(os.path.join(applications, "first", "pid"), "w") as handle:
                handle.write(str(started[0].pid))
            found = display_check.clients(names=(path,), root=applications)
            self.assertEqual([client["pid"] for client in found], [started[1].pid, started[0].pid])
            self.assertEqual([client["waiting"] for client in found], [True, False])
            self.assertEqual([client["application_id"] for client in found], [None, "first"])
            self.assertTrue(all(client["command"].startswith("python") for client in found), found)
            self.assertTrue(all(client["age_seconds"] < 30 for client in found), found)


@unittest.skipUnless(UNIX, "needs Unix sockets")
class MainTest(unittest.TestCase):
    def test_the_answer_is_one_json_document(self):
        # The desktop's display is not there where the tests run, unless they run in a desktop.
        done = subprocess.run([sys.executable, str(HELPER), "0.3"], capture_output=True, text=True, timeout=30)
        self.assertEqual(done.returncode, 0, done.stderr)
        answer = json.loads(done.stdout)
        self.assertIn(answer["display"], ("answers", "held", "gone"))


if __name__ == "__main__":
    unittest.main()
