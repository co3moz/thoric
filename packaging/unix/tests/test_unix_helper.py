"""The Linux and macOS helper, packaging/unix/thoric.sh, which releases attach beside the Windows
helper and the README runs with `curl ... | bash`: which settings a first install writes from the
answers, which answers end it before anything is written, how it reaches Docker, how an update moves
the manager to the newest release with its installation files and takes everything back when the new
manager does not answer, what stop and remove touch, and that it keeps to what macOS's bash 3.2 has.

The script runs without a terminal, so it reads its answers from stdin, with a PATH that holds only
stand-ins for docker, curl, sudo, sleep, uname and the system's own text tools. `docker run` serves
the manager image's files from a directory standing for its root filesystem, and the manager answers
its health check unless .env holds the line in BROKEN.
"""

import hashlib
import os
from pathlib import Path
import re
import shutil
import socket
import stat
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
HELPER = ROOT / "packaging/unix/thoric.sh"
REPOSITORY = "ghcr.io/co3moz/thoric"
FILES = {"compose.yml": "compose.ghcr.yml", "deploy/Caddyfile": "deploy/Caddyfile", ".env.example": ".env.example"}
LINE = "curl -fsSL https://github.com/co3moz/thoric/releases/latest/download/thoric.sh | bash"
# What the script runs from the system as it is; everything else is a stand-in.
TOOLS = ("awk", "basename", "bash", "cat", "cp", "cut", "dirname", "grep", "head", "id", "ls", "mkdir", "mktemp",
         "mv", "od", "rm", "rmdir", "sed", "sha256sum", "sh", "tail", "tr")

STUB = r"""#!/usr/bin/env bash
name=$(basename "$0")
printf '%s\n' "${SUDO_PREFIX:-}$name $*" >> "$CALLS"
case "$name $*" in
    "docker info"*)
        if [[ -n ${DOCKER_DOWN:-} || (-n ${DOCKER_NEEDS_SUDO:-} && -z ${SUDO_PREFIX:-}) ]]; then
            echo "permission denied while trying to connect to the Docker daemon socket" >&2
            exit 1
        fi
        case "$*" in
            *MemTotal*) echo "${MEMORY:-17179869184}" ;;
            *Runtimes*) echo '{"runc":{}}' ;;
            *) echo "Server: Docker Engine"; if [[ -n ${NO_MEMORY_LIMIT:-} ]]; then echo "WARNING: No memory limit support"; fi ;;
        esac ;;
    "docker version "*) echo "${ARCH:-amd64}" ;;
    "docker inspect --format {{index .Config.Labels "*)
        [[ -n ${MANAGER_DIR:-} ]] || exit 1
        echo "$MANAGER_DIR" ;;
    "docker inspect --format {{.State.Status}} "*)
        [[ -n ${MANAGER_DIR:-} ]] || exit 1
        echo "${STATE:-running}" ;;
    "docker run --rm --network none --entrypoint cat "*) exec cat "$IMAGE_ROOT${*: -1}" ;;
    "docker ps -q --filter label=thoric.environment") printf '%s' "${DESKTOPS:-}" ;;
    "docker compose logs "*) echo "The manager refused its settings." ;;
    "docker compose "*) printf 'in %s\n' "$PWD" >> "$CALLS" ;;
    "curl "*api.github.com*)
        [[ -n ${RELEASE:-} ]] || exit 22
        printf '{\n  "url": "x",\n  "tag_name": "%s",\n  "name": "%s"\n}\n' "$RELEASE" "$RELEASE" ;;
    "curl "*/healthz)
        if [[ -n ${BROKEN:-} ]] && grep -qxF "$BROKEN" "$ENV_FILE"; then exit 7; fi ;;
    "curl -fsSL https://get.docker.com -o "*) printf '#!/bin/sh\necho "Docker installed."\n' > "${*: -1}" ;;
    "sudo "*) SUDO_PREFIX="sudo " exec "$@" ;;
    "uname -s") echo "${SYSTEM:-Linux}" ;;
    "uname -m") echo "${MACHINE:-x86_64}" ;;
esac
exit 0
"""


def free_port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def settings(path):
    lines = path.read_text().splitlines()
    return dict(line.split("=", 1) for line in lines if "=" in line and not line.startswith("#"))


@unittest.skipUnless(os.name == "posix" and shutil.which("bash"), "needs bash on a POSIX system")
class HelperTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.dir)
        self.home = self.dir / "home"
        self.home.mkdir()
        self.folder = self.home / "thoric"
        self.image = self.dir / "image"
        for target, source in FILES.items():
            (self.image / "app/install" / target).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(ROOT / source, self.image / "app/install" / target)
        self.stubs = self.dir / "stubs"
        self.stubs.mkdir()
        for name in ("docker", "curl", "sudo", "sleep", "uname", "usermod"):
            self.add_stub(name)
        tools = self.dir / "tools"
        tools.mkdir()
        for name in TOOLS:
            (tools / name).symlink_to(shutil.which(name))
        # The paths that tell WSL and the NVIDIA driver apart stand in the test's own folder, so the
        # machine running the tests does not change what the helper says.
        self.system = self.dir / "system"
        self.system.mkdir()
        script = HELPER.read_text()
        for path in ("/usr/lib/wsl/package-level", "/proc/sys/kernel/osrelease", "/proc/driver/nvidia/version",
                     "/etc/cdi/nvidia.yaml", "/var/run/cdi/nvidia.yaml"):
            self.assertIn(path, script)
            script = script.replace(path, str(self.system / Path(path).name))
        self.script = self.dir / "thoric.sh"
        self.script.write_text(script)
        self.calls = self.dir / "calls"
        self.port = free_port()
        self.env = {
            "PATH": f"{self.stubs}:{tools}",
            "HOME": str(self.home),
            "TMPDIR": str(self.dir),
            "CALLS": str(self.calls),
            "IMAGE_ROOT": str(self.image),
            "ENV_FILE": str(self.folder / ".env"),
            "RELEASE": "v1.2.0",
        }

    def add_stub(self, name):
        (self.stubs / name).write_text(STUB)
        (self.stubs / name).chmod(0o755)

    def run_helper(self, answers="", *command, **env):
        self.calls.unlink(missing_ok=True)
        result = subprocess.run(["bash", str(self.script), *command], input=answers, capture_output=True, text=True,
                                env={**self.env, **env}, start_new_session=True, timeout=60)
        calls = self.calls.read_text() if self.calls.exists() else ""
        return result, calls

    def install(self, answers=None, **env):
        """A first install that starts the manager, with a password and a random agent token."""
        if answers is None:
            answers = f"\nalice\nsecret123\nsecret123\nyes\nrandom\nthis\n{self.port}\nyes\n"
        result, calls = self.run_helper(answers, **env)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result, calls

    def test_a_first_install_writes_the_settings_and_starts_the_manager(self):
        result, calls = self.install()
        written = settings(self.folder / ".env")
        self.assertEqual(written["AUTH_MODE"], "password")
        self.assertEqual(written["ADMIN_USERNAME"], "alice")
        self.assertEqual(written["ADMIN_PASSWORD"], "'secret123'")
        self.assertEqual(written["ROOT_MCP_ENABLED"], "true")
        self.assertEqual(written["MCP_AUTH_MODE"], "token")
        self.assertRegex(written["MCP_TOKEN"], r"^'[0-9a-f]{64}'$")
        self.assertEqual((written["BIND_ADDRESS"], written["PORT"]), ("127.0.0.1", str(self.port)))
        self.assertEqual(written["ALLOWED_HOSTS"], "localhost")
        # The manager is pinned to the newest release, in .env and in the example beside it.
        self.assertEqual(written["MANAGER_IMAGE"], f"{REPOSITORY}:1.2.0")
        self.assertEqual(settings(self.folder / ".env.example")["MANAGER_IMAGE"], f"{REPOSITORY}:1.2.0")
        self.assertIn(f"docker pull {REPOSITORY}:1.2.0\n", calls)
        # .env holds the password and the token, so only its owner reads it.
        self.assertEqual(stat.S_IMODE((self.folder / ".env").stat().st_mode), 0o600)
        # The installation files are the image's, with the checksums an update compares them with.
        self.assertEqual((self.folder / "compose.yml").read_text(), (ROOT / "compose.ghcr.yml").read_text())
        self.assertEqual((self.folder / "deploy/Caddyfile").read_text(), (ROOT / "deploy/Caddyfile").read_text())
        recorded = dict(reversed(line.split("  ")) for line in (self.folder / ".shipped.sha256").read_text().splitlines())
        self.assertEqual(set(recorded), set(FILES))
        for name, digest in recorded.items():
            self.assertEqual(digest, hashlib.sha256((self.folder / name).read_bytes()).hexdigest(), name)
        # compose.yml mounts the update folder, which Docker would otherwise make as root.
        self.assertTrue((self.folder / "update").is_dir())
        self.assertIn(f"docker compose up -d\nin {self.folder}\n", calls)
        self.assertIn(f"Thoric answers at http://localhost:{self.port}, reachable from this computer only", result.stdout)
        # The token is shown once, for the client; the password never reaches a command line.
        token = written["MCP_TOKEN"].strip("'")
        self.assertIn(f"claude mcp add --transport http thoric http://localhost:{self.port}/mcp --header 'Authorization: Bearer {token}'", result.stdout)
        self.assertNotIn("secret123", calls)
        self.assertNotIn("secret123", result.stdout)
        self.assertIn(f"Run the same line again to start, stop, update or remove Thoric:\n  {LINE}\n", result.stdout)

    def test_other_devices_names_and_no_password(self):
        answers = f"{self.dir}/elsewhere\nalice\n\nno\nother\n{self.port}\nNAS, my-pc.local\nno\n"
        result, calls = self.run_helper(answers)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        written = settings(self.dir / "elsewhere/.env")
        self.assertEqual(written["AUTH_MODE"], "none")
        self.assertEqual(written["ADMIN_PASSWORD"], "")
        self.assertEqual(written["ROOT_MCP_ENABLED"], "false")
        self.assertEqual(written["MCP_TOKEN"], "")
        self.assertEqual((written["BIND_ADDRESS"], written["ALLOWED_HOSTS"]), ("0.0.0.0", "localhost,nas,my-pc.local"))
        self.assertIn("There is no password, so anyone who reaches the dashboard controls every desktop.", result.stdout)
        self.assertIn(f"it listens on 0.0.0.0:{self.port}", result.stdout)
        self.assertIn("it answers to nas, my-pc.local.", result.stdout)
        self.assertIn("Root agent access (/mcp) is off", result.stdout)
        # Not starting it now leaves the manager alone.
        self.assertNotIn("docker compose up", calls)

    def test_a_custom_token_and_no_token(self):
        self.install(f"\nalice\nsecret123\nsecret123\nyes\ncustom\nshort$token\nthis\n{self.port}\nno\n")
        self.assertEqual(settings(self.folder / ".env")["MCP_TOKEN"], "'short$token'")
        shutil.rmtree(self.folder)
        result, _ = self.install(f"\nalice\nsecret123\nsecret123\nyes\nnone\nthis\n{self.port}\nno\n")
        self.assertEqual(settings(self.folder / ".env")["MCP_AUTH_MODE"], "none")
        self.assertIn("Agents need no token", result.stdout)

    def test_an_answer_that_does_not_do_ends_the_install_before_anything_is_written(self):
        (self.home / "full").mkdir()
        (self.home / "full/notes.txt").write_text("mine")
        with socket.socket() as taken:
            taken.bind(("127.0.0.1", 0))
            taken.listen()
            taken_port = taken.getsockname()[1]
            for answers, message in (
                ("thoric\n", "Use a full path"),
                ("~/full\n", "is not empty; choose a new folder."),
                ("\nbad name\n", "Use letters, digits, dots, dashes"),
                ("\nalice\nshort\n", "The password needs at least 8 characters."),
                ("\nalice\nsecret'123\n", "The password must not contain single quotes."),
                ("\nalice\nsecret123\nsecret124\n", "The passwords do not match."),
                ("\nalice\n\nyes\ncustom\nhas space\n", "Use 1 to 256 visible ASCII characters"),
                ("\nalice\n\nmaybe\n", "Unknown answer: maybe"),
                ("\nalice\n\nno\nthis\n0\n", "Use a port number from 1 to 65535."),
                ("\nalice\n\nno\nthis\n65536\n", "Use a port number from 1 to 65535."),
                (f"\nalice\n\nno\nthis\n{taken_port}\n", f"already answers on port {taken_port}"),
                (f"\nalice\n\nno\nother\n{self.port}\nmy_pc\n", "Use letters, digits, dots and dashes"),
            ):
                with self.subTest(answers=answers):
                    result, calls = self.run_helper(answers)
                    self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                    self.assertIn(message, result.stdout)
                    self.assertFalse(self.folder.exists())
                    self.assertNotIn("docker pull", calls)
        self.assertEqual(os.listdir(self.home / "full"), ["notes.txt"])

    def test_without_github_the_install_takes_latest(self):
        result, _ = self.install(RELEASE="")
        self.assertEqual(settings(self.folder / ".env")["MANAGER_IMAGE"], f"{REPOSITORY}:latest")
        self.assertIn(f"Installing {REPOSITORY}:latest instead.", result.stdout)

    def test_the_host_notes(self):
        (self.system / "version").write_text("NVRM version: NVIDIA UNIX Open Kernel Module 580\n")
        result, _ = self.install(MEMORY=str(4 * 1024**3), NO_MEMORY_LIMIT="1")
        self.assertIn("Docker has 4.0 GiB of memory", result.stdout)
        self.assertIn("add cgroup_enable=memory", result.stdout)
        self.assertIn("Docker does not list it.", result.stdout)
        shutil.rmtree(self.folder)
        (self.system / "version").unlink()
        result, _ = self.install()
        self.assertNotIn("GiB of memory", result.stdout)
        self.assertNotIn("NVIDIA", result.stdout)

    @unittest.skipIf(os.geteuid() == 0, "root reaches Docker without sudo")
    def test_a_user_outside_the_docker_group_gets_sudo_docker(self):
        result, calls = self.install(f"yes\n\nalice\nsecret123\nsecret123\nyes\nrandom\nthis\n{self.port}\nyes\n", DOCKER_NEEDS_SUDO="1")
        self.assertIn("is not in the docker group, so the helper runs docker with sudo.", result.stdout)
        self.assertRegex(calls, r"(?m)^sudo usermod -aG docker \S+$")
        self.assertIn(f"sudo docker pull {REPOSITORY}:1.2.0\n", calls)
        self.assertIn("sudo docker compose up -d\n", calls)
        # The files stay the person's own, written by the helper's shell, not by sudo.
        self.assertEqual((self.folder / ".env").stat().st_uid, os.geteuid())

    def test_systems_and_docker_it_cannot_run_on(self):
        for env, message in (
            ({"SYSTEM": "MINGW64_NT-10.0"}, "irm https://github.com/co3moz/thoric/releases/latest/download/thoric.ps1 | iex"),
            ({"MACHINE": "armv7l"}, "install a 64-bit system such as the 64-bit Raspberry Pi OS."),
            ({"ARCH": "arm"}, "Docker runs arm images here"),
            ({"DOCKER_DOWN": "1"}, "Docker does not answer"),
        ):
            with self.subTest(env=env):
                result, calls = self.run_helper("", **env)
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn(message, result.stdout)
                self.assertNotIn("docker pull", calls)
        # The Thoric WSL distribution has the thoric command for all of it.
        (self.system / "package-level").write_text("2\n")
        result, _ = self.run_helper("")
        self.assertEqual(result.returncode, 1)
        self.assertIn("This is the Thoric WSL distribution", result.stdout)

    def test_without_docker(self):
        (self.stubs / "docker").unlink()
        result, calls = self.run_helper("", SYSTEM="Darwin")
        self.assertEqual(result.returncode, 1)
        self.assertIn("https://docs.docker.com/desktop/setup/install/mac-install/", result.stdout)
        self.assertNotIn("get.docker.com", calls)
        result, calls = self.run_helper("no\n")
        self.assertEqual(result.returncode, 1)
        self.assertIn("Install Docker Engine now? No", result.stdout)
        self.assertNotIn("get.docker.com", calls)
        # Docker's own script runs only when asked, as root.
        result, calls = self.run_helper("yes\n")
        self.assertIn("curl -fsSL https://get.docker.com -o ", calls)
        self.assertRegex(calls, r"(?m)^sudo sh \S+/get-docker\.sh$")
        self.assertIn("Docker installed.", result.stdout)

    def installed(self):
        """An installation the helper made, with the manager running from it."""
        self.install()
        self.env["MANAGER_DIR"] = str(self.folder)

    def test_an_installation_not_from_the_helper_is_left_alone(self):
        (self.dir / "own").mkdir()
        (self.dir / "own/compose.yml").write_text("services: {}\n")
        result, calls = self.run_helper("", MANAGER_DIR=str(self.dir / "own"))
        self.assertEqual(result.returncode, 1)
        self.assertIn(f"Thoric runs from {self.dir}/own, which this helper did not install.", result.stdout)
        self.assertEqual(re.findall(r"(?m)^docker compose (?!version$).*$", calls), [])
        result, _ = self.run_helper("", MANAGER_DIR="<no value>")
        self.assertIn("Compose did not start it", result.stdout)
        result, _ = self.run_helper("", MANAGER_DIR="/opt/thoric")
        self.assertIn("Thoric runs from the WSL package here", result.stdout)

    def test_the_menu_quits_by_default_and_runs_what_is_picked(self):
        self.installed()
        result, calls = self.run_helper("\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"Thoric is installed in {self.folder}, with the manager {REPOSITORY}:1.2.0, which is running.", result.stdout)
        self.assertIn("What next? Quit", result.stdout)
        self.assertEqual(re.findall(r"(?m)^docker compose (?!version$).*$", calls), [])
        result, calls = self.run_helper("\n", STATE="exited")
        self.assertIn("What next? Start Thoric", result.stdout)
        self.assertIn("docker compose up -d\n", calls)
        result, _ = self.run_helper("agent\n")
        self.assertIn(f"URL:     http://localhost:{self.port}/mcp", result.stdout)

    def test_stop_stops_the_desktops_and_the_manager(self):
        self.installed()
        result, calls = self.run_helper("", "stop", DESKTOPS="aaa\nbbb\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("docker stop aaa bbb\n", calls)
        self.assertIn("docker compose stop\n", calls)

    def test_remove_takes_only_what_the_helper_made(self):
        self.installed()
        result, calls = self.run_helper("no\n", "remove")
        self.assertNotIn("docker compose down", calls)
        self.assertTrue((self.folder / ".env").exists())
        result, calls = self.run_helper("yes\nno\n", "remove", DESKTOPS="aaa")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("docker stop aaa\n", calls)
        self.assertIn("docker compose down\n", calls)
        # Volumes stay: Compose removes them only when asked with -v.
        self.assertNotIn("down -v", calls)
        self.assertNotIn("volume rm", calls)
        self.assertTrue((self.folder / ".env").exists())
        (self.folder / "notes.txt").write_text("mine")
        result, _ = self.run_helper("yes\nyes\n", "remove")
        self.assertEqual(sorted(os.listdir(self.folder)), ["notes.txt"])
        self.assertIn("stays, since it holds other files too.", result.stdout)
        # Once Compose has removed the manager, nothing names the folder but its place in the home.
        (self.folder / "notes.txt").unlink()
        self.install(MANAGER_DIR="")
        result, _ = self.run_helper("yes\nyes\n", "remove", MANAGER_DIR="")
        self.assertFalse(self.folder.exists())

    def test_an_update_moves_the_manager_and_its_files(self):
        self.installed()
        compose = self.folder / "compose.yml"
        compose.write_text(compose.read_text() + "# mine\n")
        caddyfile = (self.folder / "deploy/Caddyfile").read_text()
        newer = self.image / "app/install/compose.yml"
        newer.write_text(newer.read_text() + "# 1.3.0\n")
        result, calls = self.run_helper("", "update", RELEASE="v1.3.0")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"docker pull {REPOSITORY}:1.3.0\n", calls)
        self.assertEqual(settings(self.folder / ".env")["MANAGER_IMAGE"], f"{REPOSITORY}:1.3.0")
        self.assertIn("The manager moves from 1.2.0 to 1.3.0.", result.stdout)
        # An edited file is kept aside before the release's replaces it; an unchanged one is left.
        self.assertEqual(compose.read_text(), newer.read_text())
        self.assertTrue((self.folder / "compose.yml.bak").read_text().endswith("# mine\n"))
        self.assertEqual((self.folder / "deploy/Caddyfile").read_text(), caddyfile)
        self.assertFalse((self.folder / "deploy/Caddyfile.bak").exists())
        self.assertIn("docker compose up -d\n", calls)
        # The password and the token stay as they were.
        self.assertEqual(settings(self.folder / ".env")["ADMIN_PASSWORD"], "'secret123'")
        # Once there, there is nothing to update.
        for release in ("v1.3.0", "v1.2.9"):
            result, calls = self.run_helper("", "update", RELEASE=release)
            self.assertIn(f"Thoric runs 1.3.0, and the newest release is {release[1:]}, so there is nothing to update.", result.stdout)
            self.assertNotIn("docker pull", calls)

    def test_an_update_whose_manager_does_not_answer_is_taken_back(self):
        self.installed()
        compose = self.folder / "compose.yml"
        compose.write_text(compose.read_text() + "# mine\n")
        before = {name: (self.folder / name).read_text() for name in (".env", *FILES)}
        newer = self.image / "app/install/compose.yml"
        newer.write_text(newer.read_text() + "# 1.3.0\n")
        result, calls = self.run_helper("", "update", RELEASE="v1.3.0", BROKEN=f"MANAGER_IMAGE={REPOSITORY}:1.3.0")
        self.assertEqual(result.returncode, 1)
        self.assertIn("The manager refused its settings.", result.stdout)
        self.assertIn("Taking the update back", result.stdout)
        self.assertIn("The previous manager answers again.", result.stdout)
        self.assertEqual({name: (self.folder / name).read_text() for name in before}, before)
        self.assertEqual(calls.count("docker compose up -d\n"), 2)
        self.assertEqual(stat.S_IMODE((self.folder / ".env").stat().st_mode), 0o600)

    def test_without_github_an_update_changes_nothing(self):
        self.installed()
        result, calls = self.run_helper("", "update", RELEASE="")
        self.assertEqual(result.returncode, 1)
        self.assertNotIn("docker pull", calls)
        self.assertEqual(settings(self.folder / ".env")["MANAGER_IMAGE"], f"{REPOSITORY}:1.2.0")


class ReleaseTest(unittest.TestCase):
    def test_the_release_attaches_the_helper_the_readme_runs(self):
        # The workflow attaches the script under its own name, which the line in its header, the
        # README and the guide run.
        workflow = (ROOT / ".github/workflows/publish.yml").read_text()
        attached = re.search(r"(?m)^ +gh release upload \S+ (.+) --clobber$", workflow).group(1).split()
        self.assertIn("packaging/unix/thoric.sh", attached)
        script = HELPER.read_text()
        self.assertIn(f"#   {LINE}\n", script)
        self.assertIn(f"HELPER='{LINE}'\n", script)
        for document in ("README.md", "docs/INSTALLATION.md"):
            self.assertTrue(f"\n{LINE}\n" in (ROOT / document).read_text(), document)

    def test_the_helper_takes_the_files_every_manager_image_bundles(self):
        script = HELPER.read_text()
        files = re.search(r"^INSTALL_FILES=\((.*)\)$", script, re.M).group(1).split()
        self.assertEqual(sorted(files), sorted(FILES))
        # The WSL package's command takes the same ones.
        cli = (ROOT / "packaging/wsl/thoric").read_text()
        self.assertEqual(re.search(r"^INSTALL_FILES=\((.*)\)$", cli, re.M).group(1).split(), files)
        dockerfile = (ROOT / "core/manager/Dockerfile").read_text()
        for target, source in FILES.items():
            self.assertIn(f"\nCOPY {source} /app/install/{target}\n", dockerfile)

    def test_the_helper_keeps_to_bash_3_2_and_the_tools_macos_has(self):
        # macOS runs the script with /bin/bash 3.2, and its sed, stat, readlink, sort and date are
        # BSD's. Comments may name what they leave out.
        code = "\n".join(line for line in HELPER.read_text().splitlines() if not line.lstrip().startswith("#"))
        for pattern, what in (
            (r"\$\{[A-Za-z_][A-Za-z0-9_]*(,,?|\^\^?)\}", "case conversion, bash 4"),
            (r"\b(mapfile|readarray|coproc)\b", "bash 4 builtins"),
            (r"\b(declare|local|typeset) +-[a-zA-Z]*[An]", "associative arrays and namerefs, bash 4"),
            (r"\|&|;;&|&>>", "bash 4 operators"),
            (r"\$\{[^}]*@[QEPAaUuLK]\}", "transformations, bash 4.4"),
            (r"\[\[ +-v ", "[[ -v ]], bash 4.2"),
            (r"\bwait +-n\b", "wait -n, bash 4.3"),
            (r"\bread\b[^\n]*-t +0\.", "fractional read timeouts, bash 4"),
            (r"\bsed +-i\b", "sed -i, which BSD sed reads differently"),
            (r"\bstat +-c\b|\breadlink +-f\b|\bsort +-V\b|\bdate +-d\b|\bgrep +-[a-zA-Z]*P", "GNU-only options"),
        ):
            self.assertIsNone(re.search(pattern, code), what)
        # sha256sum is GNU's, and a Mac has shasum, so the one line that names it falls back to it.
        lines = [line for line in code.splitlines() if "sha256sum" in line]
        self.assertEqual(len(lines), 1)
        self.assertIn("shasum -a 256", lines[0])


if __name__ == "__main__":
    unittest.main()
