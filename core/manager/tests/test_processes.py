"""The process helper: a list reads every process of a /proc but the helper's own, measures CPU
over its interval, names a process's role, sorts, searches and bounds the list and answers the
fields core/manager/tests/process-list.json holds the manager's types to; a signal reaches the
process and, with tree, its descendants parents first, says which ended within the wait, and tells
a process that is gone or another user's from one it signalled."""

import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

HERE = Path(__file__).resolve().parent
HELPER = HERE.parent / "helpers/processes.py"
spec = importlib.util.spec_from_file_location("processes", HELPER)
processes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(processes)

FIXTURE = json.loads((HERE / "process-list.json").read_text(encoding="utf-8"))
HZ = os.sysconf("SC_CLK_TCK")
PAGE = os.sysconf("SC_PAGE_SIZE")
ME = os.getuid()


class FakeProc:
    """A /proc of made-up processes, with a passwd and a cgroup beside it."""

    def __init__(self, root):
        self.root = Path(root)
        self.proc = self.root / "proc"
        self.proc.mkdir()
        (self.proc / "stat").write_text("cpu  1 2 3\nbtime 1700000000\n")
        (self.root / "passwd").write_text("root:x:0:0::/root:/bin/bash\ntester:x:%d:%d::/home:/bin/sh\n" % (ME, ME))
        (self.root / "cgroup").mkdir()

    def add(self, pid, name, ppid=1, state="S", utime=0, stime=0, threads=1, start=100, rss=10, argv=None):
        folder = self.proc / str(pid)
        folder.mkdir(exist_ok=True)
        # The fields after the name, numbered as proc(5) numbers them from 3.
        fields = ["0"] * 50
        fields[0], fields[1] = state, str(ppid)
        fields[11], fields[12] = str(utime), str(stime)
        fields[17], fields[19], fields[21] = str(threads), str(start), str(rss)
        (folder / "stat").write_text("%d (%s) %s\n" % (pid, name, " ".join(fields)))
        data = b"" if argv is None else b"".join(arg.encode() + b"\0" for arg in argv)
        (folder / "cmdline").write_bytes(data)

    def end(self, pid):
        for name in ("stat", "cmdline"):
            (self.proc / str(pid) / name).unlink()
        (self.proc / str(pid)).rmdir()


class HelperTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.fake = FakeProc(self.temporary.name)
        patches = [
            mock.patch.object(processes, "PROC", str(self.fake.proc)),
            mock.patch.object(processes, "PASSWD", str(self.fake.root / "passwd")),
            mock.patch.object(processes, "CGROUP", str(self.fake.root / "cgroup")),
        ]
        for patch in patches:
            patch.start()
            self.addCleanup(patch.stop)
        self.addCleanup(self.temporary.cleanup)


class Clock:
    """time for the helper: a sleep moves the clock and runs what the processes did meanwhile."""

    def __init__(self, during=None):
        self.now = 1000.0
        self.during = during

    def monotonic(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds
        if self.during:
            self.during()
            self.during = None


class ListingTest(HelperTest):
    def desktop(self):
        self.fake.add(1, "desktop-session", ppid=0, argv=["/bin/bash", "/usr/local/bin/desktop-session"])
        self.fake.add(9, "Xtigervnc", argv=["Xtigervnc", ":0", "-geometry", "1920x1080"], rss=2000, utime=HZ)
        self.fake.add(40, "chrome", ppid=30, argv=["/opt/google/chrome/chrome", "--no-first-run"], rss=9000, threads=30)
        self.fake.add(41, "chrome", ppid=40, argv=["/opt/google/chrome/chrome --type=renderer"], rss=4000)
        self.fake.add(50, "python3", ppid=0, argv=["python3", "-c", '"""Run a desktop application.\nimport os\n"""', "/tmp/x"])
        self.fake.add(os.getpid(), "python3", ppid=0, argv=["python3", "-c", "the helper"])

    def run_listing(self, during=None, order="cpu", limit=50, query=""):
        with mock.patch.object(processes, "time", Clock(during)):
            return processes.listing(500, order, limit, query)

    def test_the_answer_has_the_fields_the_manager_reads(self):
        self.desktop()
        result = self.run_listing()
        self.assertEqual(set(result), set(FIXTURE))
        for item in result["processes"]:
            self.assertEqual(set(item), set(FIXTURE["processes"][0]), item)
        self.assertEqual({item["role"] for item in FIXTURE["processes"]}, {"desktop", "thoric", "program"})

    def test_every_process_but_the_helper_is_listed_with_its_role(self):
        self.desktop()
        result = self.run_listing(order="pid")
        listed = {item["pid"]: item for item in result["processes"]}
        self.assertEqual(sorted(listed), [1, 9, 40, 41, 50])
        self.assertEqual(result["total"], 5)
        self.assertEqual(result["matched"], 5)
        self.assertEqual({pid: item["role"] for pid, item in listed.items()},
                         {1: "desktop", 9: "desktop", 40: "program", 41: "program", 50: "thoric"})
        self.assertEqual(listed[40]["user"], "tester")
        self.assertEqual(listed[40]["uid"], ME)
        self.assertEqual(listed[40]["threads"], 30)
        self.assertEqual(listed[40]["rss_bytes"], 9000 * PAGE)
        self.assertEqual(listed[9]["cpu_seconds"], 1.0)
        self.assertEqual(listed[9]["started_at"], 1700000000 + 100 // HZ)
        self.assertEqual(result["rss_bytes"], sum(item["rss_bytes"] for item in listed.values()))

    def test_cpu_use_is_what_each_process_spent_of_the_interval(self):
        self.desktop()

        def meanwhile():
            # Chrome spends half a core over the half second, and a new process all it ran.
            self.fake.add(40, "chrome", ppid=30, argv=["chrome"], rss=9000, utime=HZ // 4)
            self.fake.add(60, "cc1", ppid=40, argv=["cc1"], utime=HZ // 2, start=500)
            # A pid taken again by a process that started later counts from nothing too.
            self.fake.add(41, "chrome", ppid=40, argv=["chrome"], utime=HZ // 10, start=400)

        listed = {item["pid"]: item for item in self.run_listing(meanwhile)["processes"]}
        self.assertEqual(listed[40]["cpu_percent"], 50.0)
        self.assertEqual(listed[60]["cpu_percent"], 100.0)
        self.assertEqual(listed[41]["cpu_percent"], 20.0)
        self.assertEqual(listed[9]["cpu_percent"], 0.0)

    def test_the_list_is_sorted_searched_and_bounded(self):
        self.desktop()
        by_memory = self.run_listing(order="memory", limit=2)
        self.assertEqual([item["pid"] for item in by_memory["processes"]], [40, 41])
        self.assertEqual(by_memory["matched"], 5)
        found = self.run_listing(query="renderer")
        self.assertEqual([item["pid"] for item in found["processes"]], [41])
        self.assertEqual((found["total"], found["matched"]), (5, 1))
        by_user = self.run_listing(query="tester", order="pid")
        self.assertEqual(by_user["matched"], 5)
        by_name = self.run_listing(query="XTIGER")
        self.assertEqual([item["pid"] for item in by_name["processes"]], [9])

    def test_a_command_line_stays_one_bounded_line(self):
        self.desktop()
        self.fake.add(70, "zombie", state="Z")
        self.fake.add(71, "long", argv=["x" * 2000])
        listed = {item["pid"]: item for item in self.run_listing()["processes"]}
        self.assertEqual(listed[50]["command"], 'python3 -c """Run a desktop application. … /tmp/x')
        self.assertEqual(listed[70]["command"], "[zombie]")
        self.assertEqual(len(listed[71]["command"]), processes.COMMAND_LIMIT)
        self.assertTrue(listed[71]["command"].endswith("…"))

    def test_a_name_the_kernel_cut_is_the_programs_own(self):
        self.assertEqual(processes.display_name("gnome-session-b", "/usr/libexec/gnome-session-binary"),
                         "gnome-session-binary")
        self.assertEqual(processes.display_name("Web Content", "/usr/lib/firefox/firefox"), "Web Content")
        self.assertEqual(processes.display_name("chrome", "/opt/google/chrome/chrome --type=gpu"), "chrome")
        self.assertEqual(processes.display_name("abcdefghijklmno", None), "abcdefghijklmno")

    def test_the_container_figures_come_from_its_cgroup(self):
        cgroup = self.fake.root / "cgroup"
        (cgroup / "memory.current").write_text("123456\n")
        (cgroup / "memory.max").write_text("max\n")
        (cgroup / "cpu.max").write_text("50000 100000\n")
        result = self.run_listing()
        self.assertEqual(result["memory_bytes"], 123456)
        self.assertIsNone(result["memory_limit_bytes"])
        self.assertEqual(result["cpus"], 0.5)
        self.assertEqual(result["interval_ms"], 500)
        (cgroup / "cpu.max").write_text("max 100000\n")
        self.assertGreaterEqual(processes.cpus(), 1)


class SignalTest(HelperTest):
    def setUp(self):
        super().setUp()
        # 10 runs 11 and 12, and 12 runs 13; 20 is root's.
        self.fake.add(10, "make", ppid=1)
        self.fake.add(11, "cc", ppid=10)
        self.fake.add(12, "sh", ppid=10)
        self.fake.add(13, "ld", ppid=12)
        self.fake.add(20, "cron", ppid=1)
        self.sent = []
        self.refuse = set()
        self.ignore = set()

    def kill(self, pid, number):
        if pid in self.refuse:
            raise PermissionError(1, "Operation not permitted")
        if not (self.fake.proc / str(pid) / "stat").exists():
            raise ProcessLookupError(3, "No such process")
        self.sent.append((pid, number))
        if pid not in self.ignore:
            self.fake.end(pid)

    def send(self, pid, name="term", tree=False, wait_ms=0):
        with mock.patch.object(processes.os, "kill", self.kill):
            return processes.send(pid, name, tree, wait_ms)

    def test_a_tree_is_signalled_parents_first(self):
        result = self.send(10, "kill", tree=True)
        self.assertEqual(self.sent, [(pid, processes.SIGNALS["kill"]) for pid in (10, 11, 12, 13)])
        self.assertEqual(result, {"signalled": [10, 11, 12, 13], "ended": [10, 11, 12, 13],
                                  "running": [], "refused": []})

    def test_without_tree_only_the_process_is_signalled(self):
        result = self.send(12)
        self.assertEqual(self.sent, [(12, processes.SIGNALS["term"])])
        self.assertEqual(result["ended"], [12])

    def test_a_process_that_does_not_end_is_still_running_after_the_wait(self):
        self.ignore.add(11)
        result = self.send(10, tree=True, wait_ms=100)
        self.assertEqual(result["running"], [11])
        self.assertEqual(result["ended"], [10, 12, 13])

    def test_an_unreaped_or_replaced_process_has_ended(self):
        def kill(pid, number):
            self.sent.append((pid, number))
            if pid == 11:
                # Ended, and waits for its parent to reap it.
                self.fake.add(11, "cc", ppid=10, state="Z")
            elif pid == 12:
                # Ended, and its pid went to a process that started later.
                self.fake.add(12, "sh", ppid=1, start=999)

        with mock.patch.object(processes.os, "kill", kill):
            result = processes.send(10, "term", True, 100)
        self.assertEqual(result["ended"], [11, 12])
        self.assertEqual(result["running"], [10, 13])

    def test_another_users_descendant_is_named_and_left(self):
        self.refuse.add(13)
        result = self.send(10, tree=True)
        self.assertEqual(result["signalled"], [10, 11, 12])
        self.assertEqual(result["refused"], [13])

    def test_another_users_process_is_refused_by_its_user(self):
        # The fake process belongs to whoever runs the test, whom the fake passwd calls tester.
        self.refuse.add(20)
        result = self.send(20)
        self.assertEqual(result, {"error": "permission", "user": "tester"})
        self.assertEqual(self.sent, [])

    def test_a_missing_process_is_said_to_be(self):
        self.assertEqual(self.send(99), {"error": "missing"})
        self.assertEqual(self.sent, [])


class CommandTest(unittest.TestCase):
    def test_bad_arguments_print_the_usage(self):
        for arguments in ([], ["list", "500", "name", "10"], ["signal", "5", "stop", "0", "0"], ["list", "500"]):
            with mock.patch("sys.stderr"), mock.patch("builtins.print") as printed:
                self.assertEqual(processes.main(arguments), 2, arguments)
            self.assertTrue(printed.called)


if __name__ == "__main__":
    unittest.main()
