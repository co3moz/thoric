"""The WSL package's `thoric update`: which manager release it moves an installation to, which
desktop images it refreshes for that manager, how it brings the installation files of that release
without losing edits made by hand, how it takes everything back when the new manager does not
answer, when it replaces the command itself with the one that release bundles, and how it keeps
the distribution's system current: its packages and the package's own system files.

The script runs against a temporary installation directory with stand-ins for docker, curl,
sudo, systemctl, sleep, apt-get, dpkg-query and dockerd on PATH, each recording its arguments.
`docker run` serves the manager image's files from a directory standing for its root filesystem,
and the manager answers its health check unless .env holds the line in BROKEN. The package's
system files are under a temporary root too, and upgrading the packages makes Docker's socket
again when SOCKET_MOVES is set, as upgrading Docker Engine does.
"""

import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import stat
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
CLI = ROOT / "packaging/wsl/thoric"
REPOSITORY = "ghcr.io/co3moz/thoric"
FILES = ("compose.yml", "deploy/Caddyfile", ".env.example")
UNITS = ("thoric-update.path", "thoric-update.service")
SETTINGS_UNITS = ("thoric-settings.path", "thoric-settings.service")

STUB = """#!/usr/bin/env bash
printf '%s\\n' "$(basename "$0") $*" >> "$CALLS"
case "$(basename "$0") $*" in
    "curl "*) [[ -n ${RELEASE:-} ]] || exit 22; printf '{\\n  "url": "x",\\n  "tag_name": "%s",\\n  "name": "%s"\\n}\\n' "$RELEASE" "$RELEASE" ;;
    "docker pull "*":24.04") touch "$MOVED"; exit "${PULL_STATUS:-0}" ;;
    "docker pull "*) exit "${PULL_STATUS:-0}" ;;
    "docker tag "*) touch "$MOVED" ;;
    "docker run "*) exec cat "$IMAGE_ROOT${*: -1}" ;;
    "docker compose pull") exit "${COMPOSE_PULL_STATUS:-0}" ;;
    "docker compose exec -T manager python3 "*) printf '%s\\n' "${RECORDS_SAY-free}"; exit "${RECORDS_STATUS:-0}" ;;
    "docker compose exec "*)
        [[ -n ${BROKEN:-} ]] && grep -qxF "$BROKEN" "$ENV_FILE" && exit 1
        printf '[{"reference": "ghcr.io/co3moz/thoric/ubuntu-xfce:24.04"}]\\n' ;;
    "docker compose logs "*) echo "The database was moved to a later layout." ;;
    "docker compose ps "*) echo manager-container ;;
    "docker inspect "*) printf '%s\\n' "${RELEASE_LABEL-<no value>}" ;;
    "docker image inspect --format {{.Id}} "*":24.04")
        [[ -n ${DOWNLOADED:-} ]] || exit 1
        if [[ -e $MOVED ]]; then echo "${NEW_ID:-}"; else echo "${OLD_ID:-}"; fi ;;
    "docker image inspect --format {{len .RepoTags}} "*) echo "${OLD_TAGS:-0}" ;;
    "docker image rm sha256:"*) exit "${RM_STATUS:-0}" ;;
    "docker image inspect "*":24.04") [[ -n ${DOWNLOADED:-} ]] || exit 1 ;;
    "docker image inspect "*) [[ -n ${RELEASE_TAG_HERE:-} ]] || exit 1 ;;
    "sudo "*) exec "$@" ;;
    "systemctl is-active --quiet thoric-update.service") [[ -n ${UPDATING:-} ]]; exit $? ;;
    "apt-get "*" update") exit "${APT_STATUS:-0}" ;;
    "systemctl show --property MainPID --value docker") if [[ -f $DOCKER_PID ]]; then cat "$DOCKER_PID"; fi ;;
    "apt-get "*" upgrade")
        if [[ -n ${SOCKET_MOVES:-} ]]; then
            touch "$SOCKET.new"; mv -f "$SOCKET.new" "$SOCKET"; echo upgraded > "$DOCKER_PID"
        fi
        if [[ -n ${PACKAGES_AFTER:-} ]]; then printf '%s\\n' "$PACKAGES_AFTER" > "$PACKAGES"; fi ;;
    "dpkg-query "*) if [[ -f $PACKAGES ]]; then cat "$PACKAGES"; fi ;;
    "dockerd --validate "*) exit "${VALIDATE_STATUS:-0}" ;;
esac
exit 0
"""


def shipped(files):
    """The checksum file the package writes with sha256sum."""
    return "".join(f"{hashlib.sha256(content.encode()).hexdigest()}  {name}\n" for name, content in files.items())


SCRIPT = CLI.read_text()
# The package's own system files, which an update keeps as the release of its command has them.
SYSTEM_FILES = tuple(re.search(r"^SYSTEM_FILES=\((.*)\)$", SCRIPT, re.M).group(1).split())
# What the packages before the record of their checksums shipped, as "checksum path".
SHIPPED_BEFORE_RECORD = re.findall(r'^    "([0-9a-f]{64} /\S+)"$', SCRIPT, re.M)
SYSTEM_LEVEL = int(re.search(r"^SYSTEM_LEVEL=(\d+)$", SCRIPT, re.M).group(1))
DAEMON = "/etc/docker/daemon.json"
PIN = "/etc/apt/preferences.d/thoric-docker.pref"
WATCHER = "/usr/lib/wsl/firewall-watch.sh"
WATCHER_UNIT = "/etc/systemd/system/thoric-firewall.service"
# daemon.json as the packages of 0.8.0 to 0.8.4 shipped it, before it rotated container logs.
OLD_DAEMON = """{
    "live-restore": true,
    "runtimes": {
        "nvidia": {
            "args": [],
            "path": "nvidia-container-runtime"
        }
    }
}
"""


def sha256(content):
    return hashlib.sha256(content.encode()).hexdigest()


def copy_source(dockerfile, target):
    """The source of the COPY that puts a file at target, or None."""
    copy = re.search(rf"^COPY (?:--chmod=\d+ )?(\S+) {re.escape(target)}$", dockerfile, re.M)
    return copy and copy.group(1)


def package_level():
    """The level packaging/wsl/Dockerfile records for the package's own files, and where."""
    match = re.search(r"printf '(\d+)\\n' > (\S+)", (ROOT / "packaging/wsl/Dockerfile").read_text())
    return match and (int(match.group(1)), match.group(2))


@unittest.skipUnless(os.name == "posix" and shutil.which("bash"), "needs bash on a POSIX system")
class UpdateTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.dir)
        self.install = self.dir / "install"
        self.install.mkdir()
        self.cli = self.dir / "thoric"
        self.level = self.dir / "package-level"
        self.units = self.dir / "systemd"
        self.units.mkdir()
        # The distribution's root, which holds the package's system files, and Docker's socket.
        self.root = self.dir / "root"
        self.root.mkdir()
        self.record = self.dir / "system.sha256"
        self.socket = self.dir / "docker.sock"
        self.socket.touch()
        self.script = CLI.read_text()
        for line, replacement in (
            ("INSTALL=/opt/thoric", f"INSTALL={self.install}"),
            ("CLI=/usr/local/bin/thoric", f"CLI={self.cli}"),
            ("PACKAGE_LEVEL=/usr/lib/wsl/package-level", f"PACKAGE_LEVEL={self.level}"),
            ("UNIT_DIR=/etc/systemd/system", f"UNIT_DIR={self.units}"),
            ("UPDATE_WAIT=180", "UPDATE_WAIT=2"),
            ("SYSTEM_RECORD=/usr/lib/wsl/system.sha256", f"SYSTEM_RECORD={self.record}"),
            ("SYSTEM_ROOT=", f"SYSTEM_ROOT={self.root}"),
            ("DOCKER_SOCKET=/var/run/docker.sock", f"DOCKER_SOCKET={self.socket}"),
        ):
            self.assertEqual(self.script.count(f"\n{line}\n"), 1, line)
            self.script = self.script.replace(f"\n{line}\n", f"\n{replacement}\n")
        # The command and the package level as the package of the installed release left them.
        self.cli.write_text(self.command("0.2.0"))
        self.cli.chmod(0o755)
        self.level.write_text(f"{package_level()[0]}\n")
        bin_dir = self.dir / "bin"
        bin_dir.mkdir()
        for name in ("docker", "curl", "sudo", "systemctl", "sleep", "apt-get", "dpkg-query", "dockerd"):
            (bin_dir / name).write_text(STUB)
            (bin_dir / name).chmod(0o755)
        self.calls = self.dir / "calls"
        self.image = self.dir / "image"
        self.tmp = self.dir / "tmp"
        self.tmp.mkdir()
        self.env = {
            **os.environ,
            "PATH": f"{bin_dir}:{os.environ['PATH']}",
            "CALLS": str(self.calls),
            "IMAGE_ROOT": str(self.image),
            "TMPDIR": str(self.tmp),
            "ENV_FILE": str(self.install / ".env"),
            # Exists once a pull or a tag moved the desktop's catalog reference to a new image.
            "MOVED": str(self.dir / "moved"),
            "SOCKET": str(self.socket),
            # Docker's process, which upgrading Docker Engine restarts along with its socket.
            "DOCKER_PID": str(self.dir / "docker-pid"),
            # The versions dpkg-query reports, which an upgrade replaces with PACKAGES_AFTER.
            "PACKAGES": str(self.dir / "packages"),
        }
        # The installation as the 0.2.0 package left it, and the files the newer manager bundles.
        self.old = {
            "compose.yml": "services:\n  manager:\n    environment:\n      AUTH_MODE: x\n",
            "deploy/Caddyfile": "https://{$DESKTOP_HOST} {\n}\n",
            ".env.example": f"PORT=8080\nMANAGER_IMAGE={REPOSITORY}:0.2.0\n",
        }
        self.write_files(self.install, self.old)
        (self.install / "compose.yml").chmod(0o640)
        (self.install / ".shipped.sha256").write_text(shipped(self.old))
        self.new = {
            "compose.yml": "services:\n  manager:\n    environment:\n      AUTH_MODE: x\n      NEW_SETTING: y\n",
            "deploy/Caddyfile": self.old["deploy/Caddyfile"],
            ".env.example": f"PORT=8080\nMANAGER_IMAGE={REPOSITORY}:latest\nNEW_SETTING=y\n",
        }
        self.write_files(self.image / "app/install", self.new)

    @staticmethod
    def write_files(directory, files):
        for name, content in files.items():
            (directory / name).parent.mkdir(parents=True, exist_ok=True)
            (directory / name).write_text(content)

    def installed(self, name):
        return (self.install / name).read_text()

    def command(self, version="", marker=None):
        """This command with the test's paths, recording its release as the package and an update do, or
        none, as the manager image bundles it. A marker makes it record each run in the call list."""
        script = self.script.replace("\nCLI_VERSION=\n", f"\nCLI_VERSION={version}\n")
        if marker:
            self.assertEqual(script.count("\nset -euo pipefail\n"), 1)
            script = script.replace("\nset -euo pipefail\n", f"\nset -euo pipefail\nprintf '%s\\n' \"{marker} $*\" >> \"$CALLS\"\n")
        return script

    def bundle(self, script):
        """Puts a command into the manager image."""
        (self.image / "app/install/thoric").write_text(script)

    def update(self, env_file, release="v0.3.0", pull_status=0, **stubs):
        (self.install / ".env").write_text(env_file)
        (self.dir / "moved").unlink(missing_ok=True)
        env = {**self.env, "RELEASE": release or "", "PULL_STATUS": str(pull_status), **stubs}
        result = subprocess.run(["bash", str(self.cli), "update"], env=env, capture_output=True, text=True)
        calls = self.calls.read_text().splitlines() if self.calls.exists() else []
        return result, (self.install / ".env").read_text(), calls

    def test_a_pinned_release_moves_to_the_newest_one(self):
        result, env_file, calls = self.update(f"PORT=8080\nMANAGER_IMAGE={REPOSITORY}:0.2.0\nAUTH_MODE=none\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(env_file, f"PORT=8080\nMANAGER_IMAGE={REPOSITORY}:0.3.0\nAUTH_MODE=none\n")
        self.assertIn(f"docker pull {REPOSITORY}:0.3.0", calls)
        self.assertLess(calls.index(f"docker pull {REPOSITORY}:0.3.0"), calls.index("docker compose up -d"))
        self.assertIn("docker compose pull", calls)
        self.assertIn("0.2.0 to 0.3.0", result.stdout)

    def test_the_newest_release_stays_without_a_new_pin(self):
        before = f"MANAGER_IMAGE='{REPOSITORY}:0.3.0'\n"
        result, env_file, calls = self.update(before)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(env_file, before)
        self.assertNotIn("moves", result.stdout)
        self.assertIn("docker compose up -d", calls)

    def test_a_release_newer_than_the_newest_one_is_never_moved_back(self):
        before = f"MANAGER_IMAGE={REPOSITORY}:0.10.0\n"
        result, env_file, calls = self.update(before, release="v0.9.1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(env_file, before)
        self.assertNotIn(f"docker pull {REPOSITORY}:0.9.1", calls)
        self.assertIn(f"docker run --rm --network none --entrypoint cat {REPOSITORY}:0.10.0 /app/install/compose.yml", calls)

    def test_an_image_chosen_by_hand_is_only_pulled_again(self):
        # Even a command that records no release of its own stays, though the manager bundles a newer one.
        installed = self.command()
        self.cli.write_text(installed)
        self.bundle(self.command(marker="new command"))
        # An edge build of master stays on edge. The last one names a Docker Hub image that only
        # looks like a release.
        images = (
            f"{REPOSITORY}:edge",
            f"{REPOSITORY}:sha-{'a' * 40}",
            f"{REPOSITORY}:latest",
            f"{REPOSITORY}@sha256:{'a' * 64}",
            "ghcr.io/someone/thoric:0.1.0",
            "0.9.0",
        )
        for image in images:
            with self.subTest(image=image):
                self.calls.unlink(missing_ok=True)
                before = f"MANAGER_IMAGE={image}\n"
                result, env_file, calls = self.update(before, release="v9.9.9")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(env_file, before)
                self.assertFalse(any(call.startswith("curl ") for call in calls))
                self.assertIn(f"docker pull {image}", calls)
                self.assertIn("docker compose pull", calls)
                self.assertIn("docker compose up -d", calls)
                self.assertEqual(self.cli.read_text(), installed)
                self.assertFalse(any(call.endswith("/app/install/thoric") for call in calls))
                self.assertIn("command stays as it is", result.stdout)
                # The installation files come from the manager it runs, so its settings reach it.
                self.assertIn(f"docker run --rm --network none --entrypoint cat {image} /app/install/compose.yml", calls)
                self.assertEqual(self.installed("compose.yml"), self.new["compose.yml"])

    def test_a_missing_image_setting_is_pinned_to_the_newest_release(self):
        result, env_file, _ = self.update("AUTH_MODE=none\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(env_file, f"AUTH_MODE=none\nMANAGER_IMAGE={REPOSITORY}:0.3.0\n")

    def test_nothing_changes_when_the_release_or_its_image_is_unavailable(self):
        before = f"MANAGER_IMAGE={REPOSITORY}:0.2.0\n"
        for release, pull_status in ((None, 0), ("nightly", 0), ("v0.3.0", 1)):
            with self.subTest(release=release, pull_status=pull_status):
                self.calls.unlink(missing_ok=True)
                result, env_file, calls = self.update(before, release=release, pull_status=pull_status)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(env_file, before)
                self.assertFalse(any(call.startswith("docker run ") for call in calls))
                self.assertEqual(self.installed("compose.yml"), self.old["compose.yml"])
                self.assertNotIn("docker compose up -d", calls)

    def test_downloaded_desktops_follow_the_release_of_the_manager(self):
        desktop = "ghcr.io/co3moz/thoric/ubuntu-xfce"
        before = f"MANAGER_IMAGE={REPOSITORY}:0.3.0\n"
        _, _, calls = self.update(before, DOWNLOADED="1", RELEASE_LABEL="0.3.0")
        refresh = calls[calls.index("docker compose ps -q manager") + 1 :]
        self.assertEqual(
            [call for call in refresh if not call.startswith(("docker inspect", "docker image inspect"))],
            [f"docker pull {desktop}:0.3.0", f"docker tag {desktop}:0.3.0 {desktop}:24.04", f"docker image rm {desktop}:0.3.0"],
        )
        # A release tag that was already here is not the update's to remove.
        self.calls.unlink()
        _, _, calls = self.update(before, DOWNLOADED="1", RELEASE_LABEL="0.3.0", RELEASE_TAG_HERE="1")
        self.assertIn(f"docker tag {desktop}:0.3.0 {desktop}:24.04", calls)
        self.assertFalse(any(call.startswith("docker image rm ") for call in calls))
        # An edge build of master downloads the desktops published with it, under its commit.
        commit = "0123456789abcdef0123456789abcdef01234567"
        self.calls.unlink()
        _, _, calls = self.update(f"MANAGER_IMAGE={REPOSITORY}:edge\n", DOWNLOADED="1", RELEASE_LABEL=f"sha-{commit}")
        self.assertIn(f"docker pull {desktop}:sha-{commit}", calls)
        self.assertIn(f"docker tag {desktop}:sha-{commit} {desktop}:24.04", calls)
        self.assertIn(f"docker image rm {desktop}:sha-{commit}", calls)
        # A manager built from the source, or one too old to carry the label, downloads the reference.
        for label in ("", "<no value>"):
            with self.subTest(label=label):
                self.calls.unlink()
                _, _, calls = self.update(before, DOWNLOADED="1", RELEASE_LABEL=label)
                self.assertIn(f"docker pull {desktop}:24.04", calls)
                self.assertFalse(any(call.startswith("docker tag ") for call in calls))

    def test_the_desktop_image_an_update_replaced_goes_when_nothing_needs_it(self):
        desktop = "ghcr.io/co3moz/thoric/ubuntu-xfce"
        old, new = "sha256:" + "a" * 64, "sha256:" + "b" * 64
        before = f"MANAGER_IMAGE={REPOSITORY}:0.3.0\n"
        stubs = {"DOWNLOADED": "1", "RELEASE_LABEL": "0.3.0", "OLD_ID": old, "NEW_ID": new}
        result, _, calls = self.update(before, **stubs)
        self.assertEqual(result.returncode, 0, result.stderr)
        # Without force, after the catalog tag moved and once the manager's records keep nothing of it.
        self.assertLess(calls.index(f"docker tag {desktop}:0.3.0 {desktop}:24.04"), calls.index(f"docker image rm {old}"))
        # The call spans lines, since the script does; the image is its last argument.
        records = next(i for i, call in enumerate(calls) if call.startswith("docker compose exec -T manager python3 -c "))
        self.assertLess(records, calls.index(f" {old}"))
        self.assertLess(calls.index(f" {old}"), calls.index(f"docker image rm {old}"))
        self.assertIn(f"Removed the image {desktop}:24.04 named before, aaaaaaaaaaaa.", result.stdout)
        self.assertFalse(any("prune" in call or call.startswith("docker image rm -f") for call in calls))
        for changed, said, removed in (
            # A release tag that was here before still names it.
            ({"OLD_TAGS": "1"}, "stays, since another tag names it", False),
            # A record keeps it, or the records cannot be read.
            ({"RECORDS_SAY": "", "RECORDS_STATUS": "1"}, "stays, since a container or a saved image was made from it", False),
            ({"RECORDS_SAY": "needed"}, "stays, since a container or a saved image was made from it", False),
            # Docker refuses while a container uses it.
            ({"RM_STATUS": "1"}, "stays, since a container or an image built on it uses it", True),
            # The reference still names the image it named before.
            ({"NEW_ID": old}, None, False),
        ):
            with self.subTest(changed=changed):
                self.calls.unlink()
                result, _, calls = self.update(before, **{**stubs, **changed})
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(f"docker image rm {old}" in calls, removed)
                if said:
                    self.assertIn(f"The image {desktop}:24.04 named before, aaaaaaaaaaaa, {said}.", result.stdout)
                else:
                    self.assertNotIn("named before", result.stdout)

    @unittest.skipUnless(shutil.which("python3"), "needs python3")
    def test_the_records_that_keep_an_image_are_read_from_the_managers_database(self):
        # The script the update runs with python3 inside the manager's container, against a database
        # with the manager's records table.
        script = re.search(r"^IMAGE_RECORDS='(.*?)^'$", CLI.read_text(), re.M | re.S).group(1)
        data = self.dir / "data"
        data.mkdir()
        database = sqlite3.connect(data / "state.sqlite")
        database.execute("CREATE TABLE records(kind TEXT NOT NULL, id TEXT NOT NULL, data TEXT NOT NULL, PRIMARY KEY(kind,id))")
        for kind, record in (
            ("environment", {"id": "a", "source_image_id": "sha256:started"}),
            ("image", {"id": "s", "image_id": "sha256:saved"}),
            ("job", {"id": "j", "image_id": "sha256:free"}),
        ):
            database.execute("INSERT INTO records VALUES (?, ?, ?)", (kind, record["id"], json.dumps(record)))
        database.commit()

        def records(image):
            return subprocess.run(["python3", "-c", script, image], env={**os.environ, "DATA_DIR": str(data)},
                                  capture_output=True, text=True)

        self.assertEqual(records("sha256:free").stdout, "free\n")
        for image in ("sha256:started", "sha256:saved"):
            self.assertNotEqual(records(image).returncode, 0)
            self.assertEqual(records(image).stdout, "")
        # A record that does not read may keep it.
        database.execute("INSERT INTO records VALUES ('environment', 'b', '{')")
        database.commit()
        self.assertNotEqual(records("sha256:free").returncode, 0)
        database.close()


    def test_a_renamed_setting_gets_its_new_name_before_the_manager_starts(self):
        before = f"MANAGER_IMAGE={REPOSITORY}:0.2.0\nMCP_ENABLED=true\nMCP_TOKEN='a$b'\n"
        result, env_file, calls = self.update(before)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(env_file, f"MANAGER_IMAGE={REPOSITORY}:0.3.0\nROOT_MCP_ENABLED=true\nMCP_TOKEN='a$b'\n")
        self.assertIn("Renamed MCP_ENABLED to ROOT_MCP_ENABLED in .env.", result.stdout)
        rename = next(i for i, call in enumerate(calls) if call.startswith("sudo sed -i s/^MCP_ENABLED="))
        self.assertLess(rename, calls.index("docker compose up -d"))
        # With the new name there already, it applies and the old one goes.
        result, env_file, _ = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.3.0\nROOT_MCP_ENABLED=false\nMCP_ENABLED=true\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(env_file, f"MANAGER_IMAGE={REPOSITORY}:0.3.0\nROOT_MCP_ENABLED=false\n")
        self.assertIn("Removed MCP_ENABLED from .env; ROOT_MCP_ENABLED applies.", result.stdout)
        # An .env without the old name is left as it is.
        result, env_file, _ = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.3.0\nROOT_MCP_ENABLED=true\n")
        self.assertEqual(env_file, f"MANAGER_IMAGE={REPOSITORY}:0.3.0\nROOT_MCP_ENABLED=true\n")
        self.assertNotIn("MCP_ENABLED", result.stdout)

    def test_the_installation_files_come_from_the_new_manager_before_it_starts(self):
        result, env_file, calls = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.2.0\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(env_file, f"MANAGER_IMAGE={REPOSITORY}:0.3.0\n")
        self.assertEqual(self.installed("compose.yml"), self.new["compose.yml"])
        self.assertEqual(self.installed("deploy/Caddyfile"), self.new["deploy/Caddyfile"])
        # Like the package's, the example names the release the installation runs.
        example = f"PORT=8080\nMANAGER_IMAGE={REPOSITORY}:0.3.0\nNEW_SETTING=y\n"
        self.assertEqual(self.installed(".env.example"), example)
        self.assertEqual(self.installed(".shipped.sha256"), shipped({**self.new, ".env.example": example}))
        self.assertEqual(stat.S_IMODE((self.install / "compose.yml").stat().st_mode), 0o640)
        self.assertEqual([p.name for p in self.install.rglob("*") if p.suffix in (".bak", ".new")], [])
        self.assertEqual(list(self.tmp.iterdir()), [])
        copies = [
            i for i, call in enumerate(calls)
            if call.startswith("docker run ") and not call.endswith("/thoric") and "/systemd/" not in call
        ]
        self.assertEqual(len(copies), len(FILES))
        for i in copies:
            self.assertIn(f" {REPOSITORY}:0.3.0 /app/install/", calls[i])
        self.assertLess(calls.index(f"docker pull {REPOSITORY}:0.3.0"), copies[0])
        self.assertLess(copies[-1], calls.index("docker compose pull"))
        self.assertIn("Updated compose.yml.", result.stdout)
        self.assertNotIn("Updated deploy/Caddyfile.", result.stdout)
        # The pin moves only once the files of its release are in place, so an update cut short in
        # between never leaves .env naming a manager the compose.yml beside it does not know.
        files = max(i for i, call in enumerate(calls) if call.startswith("sudo mv -f ") and call.endswith("/.shipped.sha256"))
        pin = next(i for i, call in enumerate(calls) if call.startswith("sudo sed -i s|^MANAGER_IMAGE="))
        self.assertLess(files, pin)
        self.assertLess(pin, calls.index("docker compose up -d"))
        self.assertIn("The manager answers.", result.stdout)

    def test_a_manager_that_does_not_answer_is_taken_back(self):
        # Such as a manager that refuses the database: compose up succeeds, and it exits again and again.
        before = f"MANAGER_IMAGE={REPOSITORY}:0.2.0\nMCP_ENABLED=true\n"
        result, env_file, calls = self.update(before, BROKEN=f"MANAGER_IMAGE={REPOSITORY}:0.3.0", DOWNLOADED="1",
                                              RELEASE_LABEL="0.3.0")
        self.assertEqual(result.returncode, 1, result.stdout)
        # .env comes back whole, the renamed setting too, since the earlier manager reads the old name.
        self.assertEqual(env_file, before)
        for name, content in self.old.items():
            self.assertEqual(self.installed(name), content)
        self.assertEqual(self.installed(".shipped.sha256"), shipped(self.old))
        self.assertEqual(stat.S_IMODE((self.install / "compose.yml").stat().st_mode), 0o640)
        self.assertEqual(calls.count("docker compose up -d"), 2)
        # The log the dashboard's job shows says why.
        self.assertIn(f"The manager {REPOSITORY}:0.3.0 did not answer within 2 seconds.", result.stdout)
        self.assertIn("The database was moved to a later layout.", result.stdout)
        self.assertIn("Taking the update back", result.stdout)
        self.assertIn("The previous manager answers again.", result.stdout)
        # The desktops stay on the images of the manager that runs.
        self.assertFalse(any(call.startswith("docker pull ghcr.io/co3moz/thoric/") for call in calls))
        self.assertEqual(list(self.tmp.iterdir()), [])

    def test_an_update_cut_short_puts_the_files_back(self):
        # Without checksums of its own, the installation gets the release's, which go with the files.
        (self.install / ".shipped.sha256").unlink()
        before = f"MANAGER_IMAGE={REPOSITORY}:0.2.0\n"
        result, env_file, calls = self.update(before, COMPOSE_PULL_STATUS="1")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(env_file, before)
        self.assertEqual(self.installed("compose.yml"), self.old["compose.yml"])
        self.assertFalse((self.install / ".shipped.sha256").exists())
        self.assertEqual(calls.count("docker compose up -d"), 1)
        self.assertIn("Taking the update back", result.stdout)
        self.assertEqual(list(self.tmp.iterdir()), [])

    def test_a_previous_manager_that_does_not_answer_either_is_said(self):
        before = f"MANAGER_IMAGE={REPOSITORY}:0.2.0\n"
        result, env_file, _ = self.update(before, BROKEN=f"MANAGER_IMAGE={REPOSITORY}:0.2.0", COMPOSE_PULL_STATUS="1")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(env_file, before)
        self.assertIn("The previous manager does not answer either.", result.stdout)

    def test_a_file_edited_by_hand_is_saved_before_a_new_release_replaces_it(self):
        edited = self.old["compose.yml"] + '    ports: ["0.0.0.0:9000:8080"]\n'
        (self.install / "compose.yml").write_text(edited)
        result, _, _ = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.2.0\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.installed("compose.yml"), self.new["compose.yml"])
        self.assertEqual(self.installed("compose.yml.bak"), edited)
        self.assertEqual(stat.S_IMODE((self.install / "compose.yml.bak").stat().st_mode), 0o640)
        self.assertIn("compose.yml.bak", result.stdout)
        # The next release finds the file as the update left it, replaces it and keeps the saved copy.
        newer = self.new["compose.yml"] + "      NEWER_SETTING: z\n"
        (self.image / "app/install/compose.yml").write_text(newer)
        result, _, _ = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.3.0\n", release="v0.4.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.installed("compose.yml"), newer)
        self.assertEqual(self.installed("compose.yml.bak"), edited)
        self.assertNotIn(".bak", result.stdout)

    def test_a_file_edited_by_hand_stays_when_the_release_brings_the_same_file(self):
        edited = self.old["deploy/Caddyfile"].replace("{\n}", "{\n    tls internal\n}")
        (self.install / "deploy/Caddyfile").write_text(edited)
        result, _, _ = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.2.0\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.installed("deploy/Caddyfile"), edited)
        self.assertFalse((self.install / "deploy/Caddyfile.bak").exists())
        self.assertNotIn("Caddyfile", result.stdout)
        self.assertEqual(self.installed("compose.yml"), self.new["compose.yml"])

    def test_without_checksums_a_differing_file_is_saved_first(self):
        (self.install / ".shipped.sha256").unlink()
        result, _, _ = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.2.0\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.installed("compose.yml"), self.new["compose.yml"])
        self.assertEqual(self.installed("compose.yml.bak"), self.old["compose.yml"])
        self.assertFalse((self.install / "deploy/Caddyfile.bak").exists())
        self.assertTrue((self.install / ".shipped.sha256").exists())

    def test_a_manager_without_installation_files_leaves_them_as_they_are(self):
        (self.image / "app/install/deploy/Caddyfile").unlink()
        result, env_file, calls = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.2.0\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(env_file, f"MANAGER_IMAGE={REPOSITORY}:0.3.0\n")
        for name, content in self.old.items():
            self.assertEqual(self.installed(name), content)
        self.assertEqual(self.installed(".shipped.sha256"), shipped(self.old))
        self.assertIn("/app/install/deploy/Caddyfile", result.stderr)
        self.assertIn("docker compose up -d", calls)

    def bundle_units(self, names=UNITS + SETTINGS_UNITS):
        """Puts the units that take the dashboard's update requests and settings changes into the manager
        image."""
        for name in names:
            path = self.image / "app/install/systemd" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"[Unit]\nDescription={name}\n")

    def test_a_release_brings_the_units_that_take_dashboard_requests(self):
        self.bundle_units()
        result, _, calls = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.2.0\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        for name in UNITS + SETTINGS_UNITS:
            self.assertEqual((self.units / name).read_text(), f"[Unit]\nDescription={name}\n")
            self.assertEqual(stat.S_IMODE((self.units / name).stat().st_mode), 0o644)
            self.assertIn(f"docker run --rm --network none --entrypoint cat {REPOSITORY}:0.3.0 /app/install/systemd/{name}", calls)
        # The manager offers the update and settings changes once the path units watch the folder.
        self.assertTrue((self.install / "update/watched").is_file())
        self.assertTrue((self.install / "update/settings-watched").is_file())
        for path_unit in ("thoric-update.path", "thoric-settings.path"):
            enable = calls.index(f"systemctl enable --now {path_unit}")
            self.assertLess(calls.index("systemctl daemon-reload"), enable)
            self.assertLess(enable, calls.index("docker compose up -d"))
        # The same units again are left as they are, so a path unit turned off by hand stays off.
        self.calls.unlink()
        result, _, calls = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.3.0\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("systemctl daemon-reload", calls)
        self.assertNotIn("systemctl enable --now thoric-update.path", calls)
        self.assertNotIn("systemctl enable --now thoric-settings.path", calls)

    def test_a_release_with_only_the_update_units_leaves_settings_changes_as_they_are(self):
        self.bundle_units(UNITS)
        result, _, calls = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.2.0\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("systemctl enable --now thoric-update.path", calls)
        self.assertNotIn("systemctl enable --now thoric-settings.path", calls)
        self.assertIn("settings changes from the dashboard stay as they are", result.stdout)
        self.assertFalse((self.install / "update/settings-watched").exists())
        self.assertEqual(sorted(p.name for p in self.units.iterdir()), sorted(UNITS))

    def test_the_units_come_only_from_releases(self):
        self.bundle_units()
        for image in (f"{REPOSITORY}:latest", "ghcr.io/someone/thoric:0.1.0"):
            with self.subTest(image=image):
                self.calls.unlink(missing_ok=True)
                result, _, calls = self.update(f"MANAGER_IMAGE={image}\n")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertFalse(any("/app/install/systemd/" in call for call in calls))
                self.assertEqual(list(self.units.iterdir()), [])
                self.assertFalse((self.install / "update").exists())
                self.assertIn("docker compose up -d", calls)

    def test_a_release_without_the_units_leaves_dashboard_updates_as_they_are(self):
        result, _, calls = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.2.0\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("updates from the dashboard stay as they are", result.stdout)
        self.assertEqual(list(self.units.iterdir()), [])
        self.assertNotIn("systemctl enable --now thoric-update.path", calls)
        self.assertIn("docker compose up -d", calls)

    def test_a_newer_release_replaces_the_command_and_the_update_continues_with_it(self):
        bundled = self.command(marker="new command")
        self.bundle(bundled)
        # The new command gets the mode a command needs, whatever the old one had.
        self.cli.chmod(0o775)
        result, env_file, calls = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.2.0\n", DOWNLOADED="1", RELEASE_LABEL="0.3.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.cli.read_text(), bundled.replace("\nCLI_VERSION=\n", "\nCLI_VERSION=0.3.0\n"))
        self.assertEqual(stat.S_IMODE(self.cli.stat().st_mode), 0o755)
        self.assertIn("command moves from 0.2.0 to 0.3.0", result.stdout)
        self.assertEqual(env_file, f"MANAGER_IMAGE={REPOSITORY}:0.3.0\n")
        # The new command runs the whole update again, after the manager it came from is pulled and
        # before anything else changes: it copies the installation files by its own rules, finds
        # itself current, moves the pin, recreates the manager once and refreshes the desktops for it.
        restart = calls.index("new command update")
        self.assertLess(calls.index(f"docker pull {REPOSITORY}:0.3.0"), restart)
        self.assertFalse(any(call.startswith("docker compose ") for call in calls[:restart]))
        self.assertFalse(any("/app/install/compose.yml" in call for call in calls[:restart]))
        self.assertFalse(any(call.startswith("sudo sed -i s|^MANAGER_IMAGE=") for call in calls[:restart]))
        self.assertEqual(sum(call.endswith("/app/install/thoric") for call in calls), 1)
        self.assertEqual(calls.count("docker compose up -d"), 1)
        desktop = "ghcr.io/co3moz/thoric/ubuntu-xfce"
        self.assertEqual(calls.count(f"docker tag {desktop}:0.3.0 {desktop}:24.04"), 1)
        self.assertLess(restart, calls.index(f"docker tag {desktop}:0.3.0 {desktop}:24.04"))
        self.assertEqual(self.installed("compose.yml"), self.new["compose.yml"])
        self.assertEqual(list(self.tmp.iterdir()), [])
        self.assertEqual(sorted(p.name for p in self.dir.glob("thoric*")), ["thoric"])

    def test_the_command_never_moves_to_the_same_or_an_older_release(self):
        self.bundle(self.command(marker="new command"))
        for version in ("0.3.0", "0.10.0"):
            with self.subTest(version=version):
                self.calls.unlink(missing_ok=True)
                installed = self.command(version)
                self.cli.write_text(installed)
                result, env_file, calls = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.2.0\n")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(env_file, f"MANAGER_IMAGE={REPOSITORY}:0.3.0\n")
                self.assertEqual(self.cli.read_text(), installed)
                self.assertFalse(any(call.endswith("/app/install/thoric") for call in calls))
                self.assertNotIn("new command update", calls)
                self.assertIn("docker compose up -d", calls)

    def test_a_command_is_installed_only_on_a_package_with_the_level_it_needs(self):
        installed = self.cli.read_text()
        # The level the package recorded, if any, and the one the bundled command names.
        for level, needs, replaced in (
            (None, 1, True),
            (None, 2, False),
            ("2", 2, True),
            ("2", 3, False),
            ("unreadable", 2, False),
        ):
            with self.subTest(level=level, needs=needs):
                self.calls.unlink(missing_ok=True)
                self.cli.write_text(installed)
                self.level.unlink(missing_ok=True)
                if level:
                    self.level.write_text(f"{level}\n")
                bundled = re.sub(r"^NEEDS_PACKAGE=\d+$", f"NEEDS_PACKAGE={needs}", self.command(marker="new command"), flags=re.M)
                self.bundle(bundled)
                result, _, calls = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.2.0\n")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual("new command update" in calls, replaced)
                self.assertEqual(self.cli.read_text() == installed, not replaced)
                if not replaced:
                    self.assertIn("needs a newer WSL package", result.stdout)
                # The update finishes either way, with the command the installation has afterwards.
                self.assertEqual(calls.count("docker compose up -d"), 1)
                self.assertEqual(self.installed("compose.yml"), self.new["compose.yml"])

    def test_an_incomplete_command_is_not_installed(self):
        complete = self.command(marker="new command")
        installed = self.cli.read_text()
        for name, bundled in (
            ("empty", ""),
            ("cut off", complete[: complete.index("\ncase ${1:-help} in\n")] + "\ncase x in\n"),
            ("no package level", re.sub(r"^NEEDS_PACKAGE=.*\n", "", complete, flags=re.M)),
            ("no version line", complete.replace("\nCLI_VERSION=\n", "\n")),
        ):
            with self.subTest(name):
                self.calls.unlink(missing_ok=True)
                self.bundle(bundled)
                result, env_file, calls = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.2.0\n")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.cli.read_text(), installed)
                self.assertNotIn("new command update", calls)
                self.assertIn("thoric command", result.stderr)
                self.assertEqual(env_file, f"MANAGER_IMAGE={REPOSITORY}:0.3.0\n")
                self.assertIn("docker compose up -d", calls)

    @staticmethod
    def repository_system():
        """The package's system files as this repository has them, by path."""
        manager = (ROOT / "core/manager/Dockerfile").read_text()
        return {path: (ROOT / copy_source(manager, f"/app/install/system{path}")).read_text() for path in SYSTEM_FILES}

    def bundle_system(self, files):
        """Puts system files into the manager image."""
        self.write_files(self.image / "app/install/system", {path.lstrip("/"): content for path, content in files.items()})

    def put_system(self, files, level, record=None):
        """The distribution's system files, its package level and the checksums its package or an update
        recorded, if any."""
        self.write_files(self.root, {path.lstrip("/"): content for path, content in files.items()})
        self.level.write_text(f"{level}\n")
        if record is not None:
            self.record.write_text("".join(f"{sha256(record[path])}  {path}\n" for path in SYSTEM_FILES))

    def old_package(self):
        """The system files as the packages of 0.8.0 to 0.8.4 left them: no Docker pin, a daemon.json that
        does not rotate logs, no record of their checksums and package level 1."""
        files = {path: content for path, content in self.repository_system().items() if path != PIN}
        self.put_system({**files, DAEMON: OLD_DAEMON}, 1)

    def system(self, path):
        return (self.root / path.lstrip("/")).read_text()

    def update_by_release(self, **stubs):
        """An update to 0.3.0 by the command of 0.3.0, which brings the system files of its release."""
        self.cli.write_text(self.command("0.3.0"))
        return self.update(f"MANAGER_IMAGE={REPOSITORY}:0.2.0\n", **stubs)

    def test_a_release_brings_the_system_files_of_an_older_package(self):
        release = self.repository_system()
        self.bundle_system(release)
        self.old_package()
        keepalive = self.root / "usr/lib/wsl/keepalive.sh"
        keepalive.unlink()
        result, env_file, calls = self.update_by_release()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(env_file, f"MANAGER_IMAGE={REPOSITORY}:0.3.0\n")
        # The old package's daemon.json was as it shipped, so it is replaced, and the missing files,
        # the pin among them, are installed, a script runnable.
        for path in SYSTEM_FILES:
            self.assertEqual(self.system(path), release[path], path)
        self.assertIn(f"{sha256(OLD_DAEMON)} {DAEMON}", SHIPPED_BEFORE_RECORD)
        self.assertEqual(stat.S_IMODE(keepalive.stat().st_mode), 0o755)
        self.assertEqual(stat.S_IMODE((self.root / PIN.lstrip("/")).stat().st_mode), 0o644)
        self.assertIn(f"Updated {DAEMON}.", result.stdout)
        self.assertIn(f"Updated {PIN}.", result.stdout)
        self.assertNotIn(f"Updated {WATCHER}.", result.stdout)
        # The update records what it installed and raises the package level.
        self.assertEqual(self.record.read_text(), "".join(f"{sha256(release[path])}  {path}\n" for path in SYSTEM_FILES))
        self.assertEqual(self.level.read_text(), f"{SYSTEM_LEVEL}\n")
        # Docker checks the new daemon.json first and restarts with it before the manager is created
        # again; apt upgrades the packages once the pin is in place. The firewall watcher, whose files
        # stay the same, keeps running.
        upgrade = next(i for i, call in enumerate(calls) if call.startswith("apt-get ") and call.endswith(" upgrade"))
        pin = next(i for i, call in enumerate(calls) if call.startswith("sudo install ") and call.endswith(f"{PIN}.new"))
        self.assertLess(pin, upgrade)
        self.assertTrue(any(call.startswith("dockerd --validate --config-file ") for call in calls))
        self.assertLess(upgrade, calls.index("systemctl restart docker"))
        self.assertLess(calls.index("systemctl restart docker"), calls.index("docker compose up -d"))
        self.assertNotIn("systemctl daemon-reload", calls)
        self.assertNotIn("systemctl try-restart thoric-firewall.service", calls)

    def test_a_system_file_edited_by_hand_stays_and_the_update_shows_the_release_version(self):
        repository = self.repository_system()
        release = {**repository, WATCHER: repository[WATCHER] + "# newer\n"}
        self.bundle_system(release)
        edited = repository[WATCHER] + "# mine\n"
        self.put_system({**repository, WATCHER: edited}, 2, record=repository)
        result, _, calls = self.update_by_release()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.system(WATCHER), edited)
        self.assertIn(f"{WATCHER} has changes of your own", result.stdout)
        self.assertIn("\n-# mine\n+# newer\n", result.stdout)
        self.assertNotIn("systemctl try-restart thoric-firewall.service", calls)
        # An edit in a package before the record is told from what that package shipped as well.
        self.calls.unlink()
        self.record.unlink()
        self.put_system({**repository, WATCHER: edited}, 1)
        result, _, _ = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.3.0\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.system(WATCHER), edited)
        self.assertIn(f"{WATCHER} has changes of your own", result.stdout)
        # Once the update has shown the release's version, the same release leaves the file alone quietly.
        self.calls.unlink()
        result, _, _ = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.3.0\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.system(WATCHER), edited)
        self.assertNotIn("changes of your own", result.stdout)

    def test_a_new_firewall_unit_reloads_systemd_and_restarts_the_watcher(self):
        repository = self.repository_system()
        release = {**repository, WATCHER_UNIT: repository[WATCHER_UNIT].replace("RestartSec=10", "RestartSec=20")}
        self.assertNotEqual(release[WATCHER_UNIT], repository[WATCHER_UNIT])
        self.bundle_system(release)
        self.put_system(repository, 2, record=repository)
        result, _, calls = self.update_by_release()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.system(WATCHER_UNIT), release[WATCHER_UNIT])
        self.assertLess(calls.index("systemctl daemon-reload"), calls.index("systemctl try-restart thoric-firewall.service"))
        # Docker's configuration stayed, so Docker is not restarted.
        self.assertNotIn("systemctl restart docker", calls)

    def test_the_system_files_come_only_from_the_release_of_the_command(self):
        self.bundle_system({path: "from the image\n" for path in SYSTEM_FILES})
        for case, env_file, command in (
            ("an edge build of master", f"MANAGER_IMAGE={REPOSITORY}:edge\n", self.command("0.3.0")),
            # The image bundles no command, so the command stays older than the release it moves to.
            ("a release newer than the command", f"MANAGER_IMAGE={REPOSITORY}:0.2.0\n", self.command("0.2.0")),
            ("a command that records no release", f"MANAGER_IMAGE={REPOSITORY}:0.2.0\n", self.command()),
        ):
            with self.subTest(case):
                self.calls.unlink(missing_ok=True)
                self.cli.write_text(command)
                result, _, calls = self.update(env_file)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertFalse(any("/app/install/system/" in call for call in calls))
                self.assertEqual(list(self.root.iterdir()), [])
                self.assertFalse(self.record.exists())
                self.assertIn("system files stay as they are", result.stdout)
                # The packages are the command's own work, so they are upgraded all the same.
                self.assertTrue(any(call.startswith("apt-get ") and call.endswith(" upgrade") for call in calls))
                self.assertIn("docker compose up -d", calls)

    def test_the_packages_are_upgraded_and_a_failure_leaves_the_update_going(self):
        (self.dir / "packages").write_text("docker-ce 5:29.9.2-1\ncontainerd.io 2.4.2-1\n")
        before = f"MANAGER_IMAGE={REPOSITORY}:0.3.0\n"
        result, _, calls = self.update(before, PACKAGES_AFTER="docker-ce 5:29.9.3-1\ncontainerd.io 2.4.2-1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("  docker-ce: 5:29.9.2-1 to 5:29.9.3-1\n", result.stdout)
        self.assertNotIn("containerd.io:", result.stdout)
        # Package scripts restart no service in the distribution, so the update restarts Docker to run
        # its new version, before the manager is created again.
        self.assertIn("systemctl restart docker", calls)
        self.assertLess(calls.index("systemctl restart docker"), calls.index("docker compose up -d"))
        apt = [call for call in calls if call.startswith(("apt-get ", "sudo env "))]
        self.assertEqual(len(apt), 4, apt)
        self.assertTrue(apt[0].startswith("sudo env DEBIAN_FRONTEND=noninteractive apt-get "))
        self.assertTrue(apt[1].endswith(" update"))
        # New packages an upgrade needs come along, and configuration edited by hand stays.
        for option in ("-y", "--with-new-pkgs", "Dpkg::Options::=--force-confold"):
            self.assertIn(f" {option} ", apt[3])
        self.assertTrue(apt[3].endswith(" upgrade"))
        # A failing apt-get is said, and the update goes on to recreate the manager.
        self.calls.unlink()
        result, env_file, calls = self.update(before, APT_STATUS="100")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(env_file, before)
        self.assertIn("packages were not updated", result.stderr)
        self.assertFalse(any(call.startswith("apt-get ") and call.endswith(" upgrade") for call in calls))
        self.assertIn("docker compose up -d", calls)

    def test_a_new_docker_socket_creates_the_manager_again(self):
        # Upgrading Docker Engine makes its socket again, which a running manager does not see.
        before = f"MANAGER_IMAGE={REPOSITORY}:0.3.0\n"
        result, _, calls = self.update(before, SOCKET_MOVES="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("docker compose up -d --force-recreate", calls)
        self.assertNotIn("docker compose up -d", calls)
        # With the same socket, Compose creates again only what changed.
        self.calls.unlink()
        result, _, calls = self.update(before)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("docker compose up -d", calls)
        self.assertNotIn("docker compose up -d --force-recreate", calls)

    def test_containerd_and_docker_restart_for_their_new_versions(self):
        (self.dir / "packages").write_text("containerd.io 2.4.2-1\ndocker-ce 5:29.9.2-1\nnvidia-container-toolkit 1.20.1-1\n")
        before = f"MANAGER_IMAGE={REPOSITORY}:0.3.0\n"
        result, _, calls = self.update(before, PACKAGES_AFTER="containerd.io 2.4.3-1\ndocker-ce 5:29.9.2-1\nnvidia-container-toolkit 1.20.1-1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("systemctl restart containerd docker", calls)
        # The NVIDIA Container Toolkit runs with each container, so its new version needs no restart,
        # and packages that stay the same restart nothing.
        for after in ("containerd.io 2.4.2-1\ndocker-ce 5:29.9.2-1\nnvidia-container-toolkit 1.20.2-1", ""):
            with self.subTest(after=after):
                self.calls.unlink()
                (self.dir / "packages").write_text("containerd.io 2.4.2-1\ndocker-ce 5:29.9.2-1\nnvidia-container-toolkit 1.20.1-1\n")
                result, _, calls = self.update(before, PACKAGES_AFTER=after)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertFalse(any(call.startswith("systemctl restart ") for call in calls), calls)

    def test_docker_restarts_once_for_a_new_daemon_json(self):
        # Upgrading Docker Engine restarts it after the new daemon.json is in place, which is enough.
        self.bundle_system(self.repository_system())
        self.old_package()
        result, _, calls = self.update_by_release(SOCKET_MOVES="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.system(DAEMON), self.repository_system()[DAEMON])
        self.assertNotIn("systemctl restart docker", calls)

    def test_a_daemon_json_docker_refuses_waits_for_a_later_update(self):
        release = self.repository_system()
        self.bundle_system(release)
        self.old_package()
        result, _, calls = self.update_by_release(VALIDATE_STATUS="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.system(DAEMON), OLD_DAEMON)
        self.assertIn("refuses the release's daemon.json", result.stdout)
        self.assertNotIn("systemctl restart docker", calls)
        # The other files come all the same, and the next update, with a Docker Engine that takes it,
        # replaces the file it recorded as shipped.
        self.assertEqual(self.system(PIN), release[PIN])
        self.calls.unlink()
        result, _, calls = self.update(f"MANAGER_IMAGE={REPOSITORY}:0.3.0\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.system(DAEMON), release[DAEMON])
        self.assertIn("systemctl restart docker", calls)

    def system_command(self, env_file, **stubs):
        (self.install / ".env").write_text(env_file)
        result = subprocess.run(["bash", str(self.cli), "system"], env={**self.env, **stubs}, capture_output=True, text=True)
        calls = self.calls.read_text().splitlines() if self.calls.exists() else []
        return result, calls

    def test_the_system_command_brings_the_system_current_without_moving_the_manager(self):
        release = self.repository_system()
        self.bundle_system(release)
        self.old_package()
        self.cli.write_text(self.command("0.3.0"))
        env_file = f"MANAGER_IMAGE={REPOSITORY}:0.3.0\n"
        result, calls = self.system_command(env_file)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.system(DAEMON), release[DAEMON])
        self.assertTrue(any(call.startswith("apt-get ") and call.endswith(" upgrade") for call in calls))
        self.assertEqual(self.installed(".env"), env_file)
        self.assertFalse(any(call.startswith(("curl ", "docker pull ", "docker compose up")) for call in calls))
        # A new socket creates the running manager again, and the command waits for it to answer.
        self.calls.unlink()
        result, calls = self.system_command(env_file, SOCKET_MOVES="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        recreate = calls.index("docker compose up -d --force-recreate")
        self.assertTrue(any(call.startswith("docker compose exec -T manager curl ") for call in calls[recreate:]))
        self.assertIn("The manager answers.", result.stdout)
        # While an update runs, which does the same, it changes nothing.
        self.calls.unlink()
        result, calls = self.system_command(env_file, UPDATING="1")
        self.assertEqual(result.returncode, 1)
        self.assertIn("An update is running", result.stderr)
        self.assertFalse(any(call.startswith(("apt-get ", "sudo ")) for call in calls))


# sudoedit as root: the editor works on a copy, which is written back over the original. Like sudo,
# it falls back to a default of its own, vi here, when none of the editor variables is set.
SUDO_EDIT = """#!/usr/bin/env bash
[[ $1 == --edit && $# == 2 ]] || exit 64
copy=$(mktemp)
cat "$2" > "$copy"
${SUDO_EDITOR:-${VISUAL:-${EDITOR:-vi}}} "$copy" || exit 1
chmod u+w "$2" && cat "$copy" > "$2" && chmod u-w "$2"
"""
EDITOR = """#!/usr/bin/env bash
printf 'EDITED_BY=%s\\n' "$(basename "$0")" >> "$1"
"""


@unittest.skipUnless(os.name == "posix" and shutil.which("bash"), "needs bash on a POSIX system")
class SettingsTest(unittest.TestCase):
    """`thoric settings` as the first-start user, who may read .env but not write it."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.dir)
        self.install = self.dir / "install"
        self.install.mkdir()
        self.cli = self.dir / "thoric"
        self.cli.write_text(CLI.read_text().replace("\nINSTALL=/opt/thoric\n", f"\nINSTALL={self.install}\n"))
        bin_dir = self.dir / "bin"
        bin_dir.mkdir()
        (bin_dir / "sudo").write_text(SUDO_EDIT)
        for name in ("nano", "my-editor", "my-visual"):
            (bin_dir / name).write_text(EDITOR)
        for stub in bin_dir.iterdir():
            stub.chmod(0o755)
        inherited = {k: v for k, v in os.environ.items() if k not in ("EDITOR", "VISUAL", "SUDO_EDITOR")}
        self.env = {**inherited, "PATH": f"{bin_dir}:{os.environ['PATH']}"}

    def test_the_users_editor_saves_the_read_only_file(self):
        env_file = self.install / ".env"
        for editors, editor in (({}, "nano"), ({"EDITOR": "my-editor"}, "my-editor"),
                                ({"VISUAL": "my-visual", "EDITOR": "my-editor"}, "my-visual")):
            with self.subTest(editors=editors):
                if env_file.exists():
                    env_file.chmod(0o640)
                env_file.write_text("AUTH_MODE=none\n")
                env_file.chmod(0o440)
                result = subprocess.run(["bash", str(self.cli), "settings"], env={**self.env, **editors},
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(env_file.read_text(), f"AUTH_MODE=none\nEDITED_BY={editor}\n")
                self.assertIn("thoric restart", result.stdout)


# docker, sudo, systemctl and sleep for apply-settings. The manager answers its health check unless
# .env holds the line in BREAKS, and thoric-update.service is active only with UPDATING set.
APPLY_STUB = """#!/usr/bin/env bash
printf '%s\\n' "$(basename "$0") $*" >> "$CALLS"
case "$(basename "$0") $*" in
    "docker compose exec "*) [[ -n ${BREAKS:-} ]] && grep -qxF "$BREAKS" "$ENV_FILE" && exit 1; exit 0 ;;
    "systemctl is-active --quiet thoric-update.service") [[ -n ${UPDATING:-} ]] && exit 0; exit 3 ;;
    "sudo "*) exec "$@" ;;
esac
exit 0
"""


@unittest.skipUnless(os.name == "posix" and shutil.which("bash"), "needs bash on a POSIX system")
class ApplySettingsTest(unittest.TestCase):
    """`thoric apply-settings`, which thoric-settings.service runs as root with the request the
    dashboard left: which settings it accepts, how it rewrites .env and how it goes back when the
    manager does not answer with the new settings."""

    ENV = "# Settings\nSECURE_COOKIE=false\nADMIN_PASSWORD='a$ecret#1'\n\n# Limits\nUPLOAD_MAX_MB=1024\nWSL_GPU=true\n"

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.dir)
        self.install = self.dir / "install"
        self.install.mkdir()
        self.env_file = self.install / ".env"
        self.env_file.write_text(self.ENV)
        self.env_file.chmod(0o640)
        script = CLI.read_text()
        for line, replacement in (
            ("INSTALL=/opt/thoric", f"INSTALL={self.install}"),
            ("SETTINGS_WAIT=90", "SETTINGS_WAIT=2"),
        ):
            self.assertEqual(script.count(f"\n{line}\n"), 1, line)
            script = script.replace(f"\n{line}\n", f"\n{replacement}\n")
        self.cli = self.dir / "thoric"
        self.cli.write_text(script)
        bin_dir = self.dir / "bin"
        bin_dir.mkdir()
        for name in ("docker", "sudo", "systemctl", "sleep"):
            (bin_dir / name).write_text(APPLY_STUB)
            (bin_dir / name).chmod(0o755)
        self.calls = self.dir / "calls"
        self.tmp = self.dir / "tmp"
        self.tmp.mkdir()
        self.env = {
            **os.environ,
            "PATH": f"{bin_dir}:{os.environ['PATH']}",
            "CALLS": str(self.calls),
            "ENV_FILE": str(self.env_file),
            "TMPDIR": str(self.tmp),
        }

    def apply(self, *lines, **stubs):
        request = self.dir / "settings-started"
        request.write_text("".join(f"{line}\n" for line in ("job-id", *lines)))
        self.calls.unlink(missing_ok=True)
        result = subprocess.run(["bash", str(self.cli), "apply-settings", str(request)],
                                env={**self.env, **stubs}, capture_output=True, text=True)
        calls = self.calls.read_text().splitlines() if self.calls.exists() else []
        return result, calls

    def test_only_the_lines_asked_for_change_and_the_rest_of_env_stays(self):
        result, calls = self.apply("SECURE_COOKIE=true", "UPLOAD_MAX_MB=2048", "VOLUME_FILTER=team=desktops",
                                   "GPU_ADAPTER=GeForce RTX")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.env_file.read_text(),
                         "# Settings\nSECURE_COOKIE=true\nADMIN_PASSWORD='a$ecret#1'\n\n# Limits\nUPLOAD_MAX_MB=2048\n"
                         "WSL_GPU=true\nVOLUME_FILTER=team=desktops\nGPU_ADAPTER=GeForce RTX\n")
        self.assertEqual(stat.S_IMODE(self.env_file.stat().st_mode), 0o640)
        # The previous file is kept, with the same mode, since it holds the password too.
        previous = self.install / ".env.previous"
        self.assertEqual(previous.read_text(), self.ENV)
        self.assertEqual(stat.S_IMODE(previous.stat().st_mode), 0o640)
        self.assertEqual(calls.count("docker compose up -d --force-recreate manager"), 1)
        self.assertIn("The manager answers with the new settings.", result.stdout)
        self.assertNotIn("a$ecret", result.stdout)
        self.assertEqual(list(self.tmp.iterdir()), [])

    def test_a_secret_reaches_env_alone(self):
        url = "https://ntfy.sh/topic-of-the-phone?token=tk_notify"
        result, _ = self.apply(f"NOTIFY_URL={url}", "SECURE_COOKIE=true")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.env_file.read_text().endswith(f"\nNOTIFY_URL={url}\n"))
        # The output is the dashboard job's log, so it names the secret without its value.
        self.assertIn("  NOTIFY_URL=[hidden]\n", result.stdout)
        self.assertIn("  SECURE_COOKIE=true\n", result.stdout)
        self.assertNotIn("of-the-phone", result.stdout + result.stderr)
        # The request keeps only the job's line, which the next manager follows.
        self.assertEqual((self.dir / "settings-started").read_text(), "job-id\n")
        # Removing it has nothing to hide.
        result, _ = self.apply("NOTIFY_URL=")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("  NOTIFY_URL=\n", result.stdout)
        self.assertTrue(self.env_file.read_text().endswith("\nNOTIFY_URL=\n"))

    def test_a_refused_request_keeps_no_value_either(self):
        result, calls = self.apply("NOTIFY_URL=https://ntfy.sh/topic-of-the-phone", "MCP_TOKEN=x")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.env_file.read_text(), self.ENV)
        self.assertEqual((self.dir / "settings-started").read_text(), "job-id\n")
        self.assertNotIn("of-the-phone", result.stdout + result.stderr)
        self.assertFalse(any(call.startswith("docker compose up") for call in calls))

    def test_a_setting_the_dashboard_may_not_change_changes_nothing(self):
        for lines in (
            ("ADMIN_PASSWORD=new-password",),
            ("SECURE_COOKIE=true", "MCP_TOKEN=x"),
            ("ROOT_MCP_ENABLED=true",),
            ("BIND_ADDRESS=0.0.0.0",),
            ("MANAGER_IMAGE=ghcr.io/someone/else:1.0.0",),
            ("SECURE_COOKIE",),
            ("secure_cookie=true",),
            ("SECURE_COOKIE=true", "SECURE_COOKIE=false"),
            (),
        ):
            with self.subTest(lines=lines):
                result, calls = self.apply(*lines)
                self.assertEqual(result.returncode, 1)
                self.assertIn("nothing was changed", result.stderr.lower())
                self.assertEqual(self.env_file.read_text(), self.ENV)
                self.assertFalse((self.install / ".env.previous").exists())
                self.assertFalse(any(call.startswith("docker compose up") for call in calls))

    def test_a_value_env_would_read_otherwise_changes_nothing(self):
        for value in ("$HOME", "${ADMIN_PASSWORD}", "a #b", "'quoted'", '"quoted"', "a`id`", "a\\b", " x", "x ",
                      "tab\there", "carriage\r", "café", "x" * 2049):
            with self.subTest(value=value):
                result, calls = self.apply(f"GPU_ADAPTER={value}")
                self.assertEqual(result.returncode, 1)
                self.assertIn("cannot be written to .env", result.stderr)
                self.assertEqual(self.env_file.read_text(), self.ENV)
                self.assertFalse(any(call.startswith("docker compose up") for call in calls))

    def test_the_previous_settings_come_back_when_the_manager_does_not_answer(self):
        result, calls = self.apply("WSL_GPU=false", "SECURE_COOKIE=true", BREAKS="WSL_GPU=false")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.env_file.read_text(), self.ENV)
        self.assertEqual(stat.S_IMODE(self.env_file.stat().st_mode), 0o640)
        self.assertEqual(calls.count("docker compose up -d --force-recreate manager"), 2)
        self.assertIn(".env is back as it was", result.stdout)
        self.assertIn("The manager answers with the previous settings again.", result.stdout)

    def test_nothing_changes_while_an_update_runs(self):
        result, calls = self.apply("SECURE_COOKIE=true", UPDATING="1")
        self.assertEqual(result.returncode, 1)
        self.assertIn("An update is running", result.stderr)
        self.assertEqual(self.env_file.read_text(), self.ENV)
        self.assertFalse(any(call.startswith("docker compose up") for call in calls))


class PackageTest(unittest.TestCase):
    def test_the_package_pins_the_image_the_cli_updates(self):
        # packaging/wsl/Dockerfile rewrites this line to the release the package is built for, and
        # the build fails when the line is missing; an update recognises only that repository as a
        # pin.
        lines = [line for line in (ROOT / ".env.example").read_text().splitlines() if line.startswith("MANAGER_IMAGE=")]
        self.assertEqual(len(lines), 1)
        self.assertTrue(lines[0].startswith(f"MANAGER_IMAGE={REPOSITORY}:"), lines[0])
        self.assertIn(f"\nMANAGER_REPOSITORY={REPOSITORY}\n", CLI.read_text())
        self.assertIn(f"MANAGER_IMAGE={REPOSITORY}:$THORIC_VERSION", (ROOT / "packaging/wsl/Dockerfile").read_text())

    def test_the_manager_image_carries_the_release_label_the_cli_reads(self):
        # The manager image is built from the same argument the Rust build reads for its desktops.
        self.assertIn('LABEL thoric.release="${THORIC_RELEASE}"', (ROOT / "core/manager/Dockerfile").read_text())
        self.assertIn('"thoric.release"', CLI.read_text())

    def test_the_package_and_the_manager_bundle_the_files_an_update_copies(self):
        # An update copies these from /app/install in the manager image over the package's copies, and
        # compares them with the checksums the package records.
        match = re.search(r"^INSTALL_FILES=\((.*)\)$", CLI.read_text(), re.M)
        self.assertIsNotNone(match)
        self.assertEqual(tuple(match.group(1).split()), FILES)
        manager = (ROOT / "core/manager/Dockerfile").read_text()
        package = (ROOT / "packaging/wsl/Dockerfile").read_text()
        for name in FILES:
            source = copy_source(manager, f"/app/install/{name}")
            self.assertIsNotNone(source, name)
            self.assertEqual(source, copy_source(package, f"/opt/thoric/{name}"))
        self.assertIn(f"sha256sum {' '.join(FILES)} > .shipped.sha256", package)

    def test_the_package_and_the_manager_bundle_the_update_and_settings_units(self):
        # The package installs them, and an update brings them from /app/install/systemd in the
        # manager image to installations whose package came before them.
        script = CLI.read_text()
        self.assertEqual(tuple(re.search(r"^UPDATE_UNITS=\((.*)\)$", script, re.M).group(1).split()), UNITS)
        self.assertEqual(tuple(re.search(r"^SETTINGS_UNITS=\((.*)\)$", script, re.M).group(1).split()), SETTINGS_UNITS)
        unit_dir = re.search(r"^UNIT_DIR=(\S+)$", script, re.M).group(1)
        manager = (ROOT / "core/manager/Dockerfile").read_text()
        package = (ROOT / "packaging/wsl/Dockerfile").read_text()
        for name in UNITS + SETTINGS_UNITS:
            self.assertEqual(copy_source(manager, f"/app/install/systemd/{name}"), f"packaging/wsl/{name}")
            self.assertEqual(copy_source(package, f"{unit_dir}/{name}"), f"packaging/wsl/{name}")
        self.assertRegex(package, r"systemctl enable [^\n]*\bthoric-update\.path\b")
        self.assertRegex(package, r"systemctl enable [^\n]*\bthoric-settings\.path\b")

    def test_the_settings_units_use_the_names_the_manager_reads(self):
        script = CLI.read_text()
        install = re.search(r"^INSTALL=(\S+)$", script, re.M).group(1)
        folder = f"{install}/update"
        names = dict(re.findall(r'^pub\(crate\) const (\w+): &str = "([^"]*)";$', (ROOT / "core/manager/src/config/changes.rs").read_text(), re.M))
        self.assertIn(f"\nPathExists={folder}/{names['SETTINGS_REQUEST']}\n", (ROOT / "packaging/wsl/thoric-settings.path").read_text())
        service = (ROOT / "packaging/wsl/thoric-settings.service").read_text()
        cli = re.search(r"^CLI=(\S+)$", script, re.M).group(1)
        self.assertIn(f"ExecStartPre=/bin/mv -f {folder}/{names['SETTINGS_REQUEST']} {folder}/{names['SETTINGS_STARTED']}\n", service)
        self.assertIn(f"'exec {cli} apply-settings {folder}/{names['SETTINGS_STARTED']} >{folder}/{names['SETTINGS_LOG']} 2>&1'", service)
        self.assertIn(f"mv -f {folder}/{names['SETTINGS_EXIT']}.new {folder}/{names['SETTINGS_EXIT']}'", service)
        self.assertIn(f'install_units "$image" {names["SETTINGS_WATCHED"]} ', script)
        self.assertIn(f"{folder}/{names['SETTINGS_WATCHED']}", (ROOT / "packaging/wsl/Dockerfile").read_text())

    def test_the_command_accepts_exactly_the_settings_the_dashboard_edits(self):
        # EDITABLE in core/manager/src/config.rs is what the Settings page offers; the command takes
        # only those.
        config = (ROOT / "core/manager/src/config.rs").read_text()
        block = re.search(r"^pub\(crate\) const EDITABLE: &\[&str\] = &\[\n(.*?)^\];$", config, re.M | re.S)
        self.assertIsNotNone(block)
        editable = re.findall(r'^    "([A-Z_]+)",$', block.group(1), re.M)
        accepted = re.search(r"^DASHBOARD_SETTINGS=\((.*)\)$", CLI.read_text(), re.M).group(1).split()
        self.assertGreater(len(editable), 10)
        self.assertEqual(accepted, editable)
        # The values the command keeps out of its log are those of the secrets the page sets.
        secrets = re.search(r'^pub\(crate\) const EDITABLE_SECRETS: &\[&str\] = &\[(.*)\];$', config, re.M)
        self.assertIsNotNone(secrets)
        hidden = re.search(r"^DASHBOARD_SECRETS=\((.*)\)$", CLI.read_text(), re.M).group(1).split()
        self.assertEqual(hidden, re.findall(r'"([A-Z_]+)"', secrets.group(1)))
        self.assertTrue(set(hidden) <= set(accepted))
        # Every one of them is an installation setting the manager reads from .env.
        example = (ROOT / ".env.example").read_text()
        for name in accepted:
            self.assertIn(f"\n{name}=", example)

    def test_the_units_use_the_folder_and_the_names_the_manager_reads(self):
        # compose.ghcr.yml mounts the installation's update folder where
        # core/manager/src/releases.rs looks, and the manager reads what the service writes there by
        # the names that file gives them.
        script = CLI.read_text()
        install = re.search(r"^INSTALL=(\S+)$", script, re.M).group(1)
        folder = f"{install}/update"
        names = dict(re.findall(r'^pub\(crate\) const (\w+): &str = "([^"]*)";$', (ROOT / "core/manager/src/releases.rs").read_text(), re.M))
        self.assertIn(f"      - ./update:{names['UPDATE_DIR']}\n", (ROOT / "compose.ghcr.yml").read_text())
        self.assertIn(f"\nPathExists={folder}/{names['REQUEST']}\n", (ROOT / "packaging/wsl/thoric-update.path").read_text())
        service = (ROOT / "packaging/wsl/thoric-update.service").read_text()
        self.assertIn(f"ExecStartPre=/bin/mv -f {folder}/{names['REQUEST']} {folder}/{names['STARTED']}\n", service)
        cli = re.search(r"^CLI=(\S+)$", script, re.M).group(1)
        self.assertIn(f"'exec {cli} update >{folder}/{names['LOG']} 2>&1'", service)
        self.assertIn(f"mv -f {folder}/{names['EXIT']}.new {folder}/{names['EXIT']}'", service)
        self.assertIn(f'install_units "$image" {names["WATCHED"]} ', script)
        self.assertIn(f"touch {folder}/{names['WATCHED']}", (ROOT / "packaging/wsl/Dockerfile").read_text())

    def test_the_manager_bundles_the_command_the_package_installs(self):
        script = CLI.read_text()
        target = re.search(r"^CLI=(\S+)$", script, re.M).group(1)
        self.assertEqual(copy_source((ROOT / "packaging/wsl/Dockerfile").read_text(), target), "packaging/wsl/thoric")
        self.assertEqual(copy_source((ROOT / "core/manager/Dockerfile").read_text(), "/app/install/thoric"), "packaging/wsl/thoric")
        # Commands already installed read these two lines from a newer one before they install it, and
        # the package records its release in the first. Without one, any release counts as newer.
        self.assertEqual(re.findall(r"^CLI_VERSION=.*$", script, re.M), ["CLI_VERSION="])
        self.assertIn("CLI_VERSION=$THORIC_VERSION", (ROOT / "packaging/wsl/Dockerfile").read_text())
        needs = re.findall(r"^NEEDS_PACKAGE=(\d+)$", script, re.M)
        self.assertEqual(len(needs), 1)
        # A package runs the command it ships, so it has at least the level that command needs.
        level = package_level()
        self.assertIsNotNone(level)
        self.assertEqual(f"PACKAGE_LEVEL={level[1]}", re.search(r"^PACKAGE_LEVEL=.*$", script, re.M).group(0))
        self.assertLessEqual(int(needs[0]), level[0])

    def test_the_package_and_the_manager_bundle_the_system_files_an_update_keeps(self):
        # The package installs each system file and records its checksum; the manager image bundles the
        # same file under /app/install/system, from where an update brings it to older packages.
        manager = (ROOT / "core/manager/Dockerfile").read_text()
        package = (ROOT / "packaging/wsl/Dockerfile").read_text()
        record = re.search(r"^SYSTEM_RECORD=(\S+)$", SCRIPT, re.M).group(1)
        self.assertIn(f"sha256sum {' '.join(SYSTEM_FILES)} > {record}", package)
        for path in SYSTEM_FILES:
            source = copy_source(package, path)
            self.assertIsNotNone(source, path)
            self.assertEqual(copy_source(manager, f"/app/install/system{path}"), source, path)
        # An update raises the package level to what a package built now records.
        self.assertEqual(SYSTEM_LEVEL, package_level()[0])
        # The versions of older packages name only these files, and the record stays a separate file.
        self.assertGreater(len(SHIPPED_BEFORE_RECORD), len(SYSTEM_FILES) - 2)
        self.assertTrue({line.split()[1] for line in SHIPPED_BEFORE_RECORD} <= set(SYSTEM_FILES))
        self.assertNotIn(record, SYSTEM_FILES)

    def test_the_package_pins_docker_engine_before_it_installs_it(self):
        # apt takes the pin into account only once it is there, in the package's build as in an update.
        package = (ROOT / "packaging/wsl/Dockerfile").read_text()
        pin = package.index(f" {PIN}\n")
        self.assertLess(pin, package.index("docker-ce docker-ce-cli containerd.io"))
        preferences = (ROOT / copy_source(package, PIN)).read_text()
        self.assertRegex(preferences, r"(?m)^Package: docker-ce docker-ce-cli$")
        self.assertRegex(preferences, r"(?m)^Pin: version 5:\d+\.\*$")
        # Below 1000, so a newer Docker Engine installed by hand is never taken back.
        priority = int(re.search(r"(?m)^Pin-Priority: (\d+)$", preferences).group(1))
        self.assertTrue(990 <= priority < 1000, priority)


COMPLETION = ROOT / "packaging/wsl/thoric-completion.bash"


@unittest.skipUnless(os.name == "posix" and shutil.which("bash"), "needs bash on a POSIX system")
class CompletionTest(unittest.TestCase):
    """Tab completion of the command's first word, with the commands the command itself lists."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.dir)
        shutil.copy(CLI, self.dir / "thoric")
        (self.dir / "thoric").chmod(0o755)

    def complete(self, *words):
        """What Tab offers after the words typed so far, the last one being the word under the cursor."""
        script = (
            f'source "{COMPLETION}"\n'
            'COMP_WORDS=("$@"); COMP_CWORD=$(($# - 1))\n'
            '_thoric "$1" "${COMP_WORDS[COMP_CWORD]}" "${COMP_WORDS[COMP_CWORD - 1]}"\n'
            'printf "%s\\n" "${COMPREPLY[@]}"\n'
        )
        env = {**os.environ, "PATH": f"{self.dir}:{os.environ['PATH']}"}
        result = subprocess.run(["bash", "-c", script, "bash", *words], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.split()

    def test_a_prefix_completes_to_its_command(self):
        self.assertEqual(self.complete("thoric", "re"), ["restart"])
        self.assertEqual(sorted(self.complete("thoric", "st")), ["start", "status", "stop"])
        self.assertEqual(self.complete("thoric", "x"), [])

    def test_every_command_is_offered_and_only_the_first_word_completes(self):
        # Each branch of the command's case is offered, except help, which is what the bare command shows.
        branches = re.findall(r"^    ([a-z][a-z|-]*)\)$", CLI.read_text(), re.M)
        commands = sorted(branch for branch in branches if not branch.startswith("help"))
        self.assertIn("restart", commands)
        self.assertEqual(sorted(self.complete("thoric", "")), commands)
        self.assertEqual(self.complete("thoric", "restart", ""), [])

    def test_the_package_installs_the_completion_where_bash_completion_loads_it(self):
        # bash-completion loads completions/<command> the first time that command is completed.
        package = (ROOT / "packaging/wsl/Dockerfile").read_text()
        command = Path(re.search(r"^CLI=(\S+)$", CLI.read_text(), re.M).group(1)).name
        target = f"/usr/share/bash-completion/completions/{command}"
        self.assertEqual(copy_source(package, target), "packaging/wsl/thoric-completion.bash")
        self.assertRegex(package, r"apt-get install [^&]*\bbash-completion\b")

    def test_the_short_name_completes_like_the_command(self):
        # thor is a link to the command, so its help lists the same commands.
        (self.dir / "thor").symlink_to("thoric")
        self.assertEqual(self.complete("thor", "re"), ["restart"])
        registered = subprocess.run(
            ["bash", "-c", f'source "{COMPLETION}"; complete -p thor'], capture_output=True, text=True
        )
        self.assertEqual(registered.stdout.strip(), "complete -F _thoric thor")

    def test_the_package_links_the_short_name_and_its_completion(self):
        # Completing thor loads completions/thor, which is the command's completion file and registers both names.
        package = (ROOT / "packaging/wsl/Dockerfile").read_text()
        cli = Path(re.search(r"^CLI=(\S+)$", CLI.read_text(), re.M).group(1))
        completions = "/usr/share/bash-completion/completions"
        self.assertEqual(copy_source(package, f"{completions}/{cli.name}"), "packaging/wsl/thoric-completion.bash")
        self.assertRegex(package, rf"\bln -s {cli.name} {cli.parent}/thor\b")
        self.assertRegex(package, rf"\bln -s {cli.name} {completions}/thor\b")


if __name__ == "__main__":
    unittest.main()


@unittest.skipUnless(os.name == "posix" and shutil.which("bash"), "needs bash on a POSIX system")
class PortTest(unittest.TestCase):
    """The dashboard's address follows the PORT setting in .env, not the shell's environment."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.dir)
        self.install = self.dir / "install"
        self.install.mkdir()
        self.cli = self.dir / "thoric"
        self.cli.write_text(CLI.read_text().replace("\nINSTALL=/opt/thoric\n", f"\nINSTALL={self.install}\n"))
        # Without explorer.exe, `open` prints the address instead of opening it.
        bin_dir = self.dir / "bin"
        bin_dir.mkdir()
        self.env = {"PATH": f"{bin_dir}:/usr/bin:/bin", "PORT": "7777", "HOME": str(self.dir)}

    def address(self):
        result = subprocess.run(["bash", str(self.cli), "open"], env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip()

    def test_the_address_follows_the_port_setting(self):
        (self.install / ".env").write_text("AUTH_MODE=none\nPORT='9090'\n")
        self.assertEqual(self.address(), "http://localhost:9090")

    def test_the_default_port_is_used_without_the_setting_or_the_file(self):
        self.assertEqual(self.address(), "http://localhost:8080")
        (self.install / ".env").write_text("AUTH_MODE=none\n")
        self.assertEqual(self.address(), "http://localhost:8080")


@unittest.skipUnless(os.name == "posix" and shutil.which("bash"), "needs bash on a POSIX system")
class McpTest(unittest.TestCase):
    """`thoric mcp` follows ROOT_MCP_ENABLED, and MCP_ENABLED, its name before 0.6.3, while the new
    one is missing, as the manager does."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.dir)
        self.install = self.dir / "install"
        self.install.mkdir()
        self.cli = self.dir / "thoric"
        self.cli.write_text(CLI.read_text().replace("\nINSTALL=/opt/thoric\n", f"\nINSTALL={self.install}\n"))
        self.env = {"PATH": "/usr/bin:/bin", "HOME": str(self.dir)}

    def mcp(self, settings):
        (self.install / ".env").write_text(settings)
        return subprocess.run(["bash", str(self.cli), "mcp"], env=self.env, capture_output=True, text=True)

    def test_the_root_endpoint_is_shown_under_either_name(self):
        for settings in ("ROOT_MCP_ENABLED=true\nMCP_TOKEN='secret-token'\n", "MCP_ENABLED=true\nMCP_TOKEN=secret-token\n"):
            with self.subTest(settings=settings):
                result = self.mcp(settings)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("URL:     http://localhost:8080/mcp", result.stdout)
                self.assertIn("Authorization: Bearer secret-token", result.stdout)
                self.assertIn('"docker", "exec", "-i", "thoric-manager", "python3"', result.stdout)

    def test_the_bridge_runs_in_the_container_compose_names(self):
        result = self.mcp("ROOT_MCP_ENABLED=true\nMCP_TOKEN=x\nMANAGER_CONTAINER='desk-manager'\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('"docker", "exec", "-i", "desk-manager", "python3"', result.stdout)

    def test_without_the_root_endpoint_it_points_at_the_dashboards_endpoints(self):
        for settings in ("ROOT_MCP_ENABLED=false\nMCP_ENABLED=true\nMCP_TOKEN=x\n", "MCP_TOKEN=x\n"):
            with self.subTest(settings=settings):
                result = self.mcp(settings)
                self.assertEqual(result.returncode, 1)
                self.assertIn("MCP endpoints made on the dashboard's MCP page work without it", result.stderr)
                self.assertIn("ROOT_MCP_ENABLED=true", result.stderr)


@unittest.skipUnless(os.name == "posix" and shutil.which("bash"), "needs bash on a POSIX system")
class AccessTest(unittest.TestCase):
    """`thoric status` tells who can reach the dashboard from BIND_ADDRESS, which the first start
    sets and Compose publishes the port on, with 127.0.0.1 when it is missing or empty."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.dir)
        self.install = self.dir / "install"
        self.install.mkdir()
        self.cli = self.dir / "thoric"
        self.cli.write_text(CLI.read_text().replace("\nINSTALL=/opt/thoric\n", f"\nINSTALL={self.install}\n"))
        # A running Docker and a manager that answers its health check.
        bin_dir = self.dir / "bin"
        bin_dir.mkdir()
        for name in ("docker", "curl"):
            (bin_dir / name).write_text("#!/bin/sh\nexit 0\n")
            (bin_dir / name).chmod(0o755)
        self.env = {"PATH": f"{bin_dir}:/usr/bin:/bin", "HOME": str(self.dir), "BIND_ADDRESS": "0.0.0.0"}

    def dashboard(self, settings):
        (self.install / ".env").write_text(settings)
        result = subprocess.run(["bash", str(self.cli), "status"], env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return [line for line in result.stdout.splitlines() if line.startswith("Dashboard:")]

    def test_the_status_names_who_can_reach_the_dashboard(self):
        for settings, reach in (
            ("AUTH_MODE=none\n", "this computer only (127.0.0.1:8080)"),
            ("BIND_ADDRESS=\nPORT=9090\n", "this computer only (127.0.0.1:9090)"),
            ("BIND_ADDRESS=127.0.0.1\n", "this computer only (127.0.0.1:8080)"),
            ("BIND_ADDRESS=0.0.0.0\nPORT='9090'\n", "other devices too (0.0.0.0:9090)"),
            ("BIND_ADDRESS='192.168.1.20'\n", "other devices too (192.168.1.20:8080)"),
        ):
            with self.subTest(settings=settings):
                port = "9090" if "9090" in settings else "8080"
                self.assertEqual(self.dashboard(settings), [f"Dashboard: http://localhost:{port}, reachable from {reach}"])


# docker and curl for `thoric doctor`: Docker's proxy is DOCKER_PROXY, and the Engine API answers
# ANSWER with the status CODE; curl records the address it was given. Everything else fails.
DOCTOR_STUB = """#!/usr/bin/env bash
case "$(basename "$0") $*" in
    "docker info --format {{.HTTPSProxy}}") printf '%s\n' "${DOCKER_PROXY:-}" ;;
    "docker info "*) echo "engine 29.0.0" ;;
    "curl "*) printf '%s\n' "${*: -1}" >> "$CALLS"; printf '%s\n%s' "$ANSWER" "$CODE" ;;
    *) exit 1 ;;
esac
"""


@unittest.skipUnless(os.name == "posix" and shutil.which("bash"), "needs bash on a POSIX system")
class DoctorTest(unittest.TestCase):
    """`thoric doctor` asks Docker itself whether it reaches the registry, since Docker downloads
    images as a service with a proxy of its own, never the shell's."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.dir)
        self.install = self.dir / "install"
        self.install.mkdir()
        (self.install / ".env").write_text(f"MANAGER_IMAGE='{REPOSITORY}:1.2.3'\n")
        self.cli = self.dir / "thoric"
        self.cli.write_text(CLI.read_text().replace("\nINSTALL=/opt/thoric\n", f"\nINSTALL={self.install}\n"))
        bin_dir = self.dir / "bin"
        bin_dir.mkdir()
        for name in ("docker", "curl", "sudo", "systemctl"):
            (bin_dir / name).write_text(DOCTOR_STUB)
            (bin_dir / name).chmod(0o755)
        self.calls = self.dir / "calls"
        self.env = {"PATH": f"{bin_dir}:/usr/bin:/bin", "HOME": str(self.dir), "CALLS": str(self.calls)}

    def registry(self, code, answer="", **env):
        result = subprocess.run(["bash", str(self.cli), "doctor"], env={**self.env, "CODE": code, "ANSWER": answer, **env},
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return [line for line in result.stdout.splitlines() if line.startswith("registry:")]

    def test_docker_looks_up_the_managers_image_in_the_registry(self):
        self.assertEqual(self.registry("200", '{"Descriptor":{}}'), ["registry:  Docker reaches ghcr.io (no proxy)"])
        # The Engine API's lookup of the manager in .env, which downloads nothing.
        self.assertEqual(self.calls.read_text(), f"http://localhost/distribution/{REPOSITORY}:1.2.3/json\n")
        # A registry that answers with an error is reachable, through the proxy Docker reports.
        proxy = "http://xxxxx:xxxxx@proxy.example:3128"
        self.assertEqual(
            self.registry("404", '{"message":"manifest unknown"}', DOCKER_PROXY=proxy),
            [f"registry:  Docker reaches ghcr.io (proxy {proxy}), which answers about {REPOSITORY}:1.2.3: manifest unknown"],
        )

    def test_a_docker_without_the_shells_proxy_is_named(self):
        # As Docker answers, with the quotes in its message escaped.
        answer = '{"message":"Get \\"https://ghcr.io/v2/\\": dial tcp: lookup ghcr.io: no such host"}'
        self.assertEqual(
            self.registry("500", answer, HTTPS_PROXY="http://proxy.example:3128"),
            ['registry:  Docker does not reach ghcr.io (no proxy): Get "https://ghcr.io/v2/": dial tcp: lookup ghcr.io: '
             "no such host. This shell has a proxy and Docker none; see 'Docker behind a proxy' in docs/WSL_PACKAGE.md"],
        )
        self.assertEqual(
            self.registry("000"),
            ["registry:  Docker does not reach ghcr.io (no proxy): Docker gave no answer; see 'Docker behind a proxy' in "
             "docs/WSL_PACKAGE.md"],
        )
