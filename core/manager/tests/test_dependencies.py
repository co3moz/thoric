"""The dependency list of core/manager/dependencies.py, which the manager image's build writes for
the Dependencies page: how it reads the interface's lockfile, cargo metadata, Dockerfiles, Compose
manifests and workflows, and that it still covers everything this repository builds from.
"""

import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("dependencies", ROOT / "core/manager/dependencies.py")
dependencies = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dependencies)


def lock(packages, dependencies_=None, dev=None):
    return {
        "workspaces": {"": {"dependencies": dependencies_ or {}, "devDependencies": dev or {}}},
        "packages": packages,
    }


def by_name(found):
    return {(item["name"], item.get("version")): item for item in found}


class InterfaceTest(unittest.TestCase):
    def test_reads_the_json_bun_writes(self):
        text = '{"a": ["x, }", "y",], "b": {"c": 1,},\n}'
        self.assertEqual(
            json.loads(dependencies.without_trailing_commas(text)), {"a": ["x, }", "y"], "b": {"c": 1}}
        )

    def test_nested_keys_name_packages_inside_packages(self):
        self.assertEqual(dependencies.package_path("a/@b/c/d"), ["a", "@b/c", "d"])
        self.assertEqual(dependencies.package_path("@b/c"), ["@b/c"])

    def test_scope_follows_what_the_interface_reaches(self):
        packages = {
            "app": ["app@1.0.0", "", {"dependencies": {"shared": "^1", "@x/lib": "^2"}}, "sha"],
            "@x/lib": ["@x/lib@2.0.0", "", {"peerDependencies": {"types": "*"}, "optionalPeers": ["types"]}, "sha"],
            "shared": ["shared@1.0.0", "", {}, "sha"],
            "tool": ["tool@3.0.0", "", {"dependencies": {"shared": "^2"}, "optionalDependencies": {"bin-linux": "3", "bin-win": "3"}}, "sha"],
            # tool needs another shared than the app; Bun installs it inside tool.
            "tool/shared": ["shared@2.0.0", "", {}, "sha"],
            "bin-linux": ["bin-linux@3.0.0", "", {"os": "linux", "cpu": "x64"}, "sha"],
            "bin-win": ["bin-win@3.0.0", "", {"os": "win32", "cpu": "x64"}, "sha"],
            "types": ["types@1.0.0", "", {}, "sha"],
        }
        with tempfile.TemporaryDirectory() as folder:
            modules = Path(folder)
            for path, manifest in {
                "app": {"version": "1.0.0", "license": "MIT", "description": "The  app\n"},
                "tool/node_modules/shared": {"version": "2.0.0", "license": {"type": "ISC"}},
                "shared": {"version": "0.9.0", "license": "MIT"},
            }.items():
                (modules / path).mkdir(parents=True)
                (modules / path / "package.json").write_text(json.dumps(manifest))
            found = by_name(
                dependencies.ui_dependencies(
                    lock(packages, {"app": "1.0.0"}, {"tool": "3.0.0", "types": "1.0.0"}), modules
                )
            )

        self.assertNotIn(("bin-win", "3.0.0"), found)
        self.assertEqual(found[("app", "1.0.0")]["scope"], "runtime")
        self.assertTrue(found[("app", "1.0.0")]["direct"])
        self.assertEqual(found[("app", "1.0.0")]["license"], "MIT")
        self.assertEqual(found[("app", "1.0.0")]["description"], "The app")
        self.assertEqual(found[("shared", "1.0.0")]["scope"], "runtime")
        self.assertFalse(found[("shared", "1.0.0")]["direct"])
        # A package.json of another release says nothing about this one.
        self.assertNotIn("license", found[("shared", "1.0.0")])
        self.assertEqual(found[("shared", "2.0.0")]["scope"], "development")
        self.assertEqual(found[("shared", "2.0.0")]["license"], "ISC")
        self.assertEqual(found[("bin-linux", "3.0.0")]["scope"], "development")
        # An optional peer of a runtime package stays what package.json makes it.
        self.assertEqual(found[("types", "1.0.0")]["scope"], "development")
        self.assertTrue(found[("types", "1.0.0")]["direct"])

    def test_each_platform_lists_its_own_native_packages(self):
        packages = {
            "tool": ["tool@1.0.0", "", {"optionalDependencies": {"tool-x64": "1", "tool-arm64": "1"}}, "sha"],
            "tool-x64": ["tool-x64@1.0.0", "", {"os": "linux", "cpu": "x64"}, "sha"],
            "tool-arm64": ["tool-arm64@1.0.0", "", {"os": "linux", "cpu": "arm64"}, "sha"],
        }
        missing = Path(tempfile.gettempdir()) / "thoric-missing-node-modules"
        for platform, native in (("linux/amd64", "tool-x64"), ("linux/arm64", "tool-arm64")):
            npm, _ = dependencies.PLATFORMS[platform]
            found = dependencies.ui_dependencies(lock(packages, dev={"tool": "1.0.0"}), missing, npm)
            self.assertEqual({item["name"] for item in found}, {"tool", native}, platform)

    def test_every_package_of_the_lockfile_is_reached(self):
        """A package the walk misses would be left off the page without a word. Across every
        platform, since some packages are only needed by another platform's."""
        data = json.loads(dependencies.without_trailing_commas((ROOT / "core/web/bun.lock").read_text(encoding="utf-8")))
        expected = {dependencies.split_ident(value[0]) for value in data["packages"].values()}
        with patch.dict(dependencies.NPM_PLATFORM, clear=True):
            found = dependencies.ui_dependencies(data, ROOT / "core/web/missing-node-modules")
        self.assertEqual({(item["name"], item["version"]) for item in found}, expected)


def crate(name, version, kind="lib", **fields):
    return {
        "id": f"{name}@{version}",
        "name": name,
        "version": version,
        "source": dependencies.CRATES_IO,
        "targets": [{"kind": [kind]}],
        **fields,
    }


def node(package, *deps):
    return {
        "id": package,
        "deps": [{"pkg": pkg, "dep_kinds": [{"kind": kind, "target": None}]} for pkg, kind in deps],
    }


class ManagerTest(unittest.TestCase):
    def test_scope_follows_how_the_manager_uses_a_crate(self):
        metadata = {
            "packages": [
                crate("thoric", "1.0.0", "bin", source=None),
                crate("web", "1.0.0", license="MIT"),
                crate("derive", "1.0.0", "proc-macro", license="MIT"),
                crate("syntax", "2.0.0", license="MIT OR Apache-2.0"),
                crate("cc", "1.0.0", license="MIT"),
                crate("mock", "0.1.0", license_file="LICENSE.txt"),
                crate("forked", "0.2.0", source="git+https://example.org/forked", repository="https://example.org/forked"),
            ],
            "resolve": {
                "root": "thoric@1.0.0",
                "nodes": [
                    node("thoric@1.0.0", ("web@1.0.0", None), ("derive@1.0.0", None), ("mock@0.1.0", "dev")),
                    node("web@1.0.0", ("cc@1.0.0", "build"), ("forked@0.2.0", None)),
                    node("derive@1.0.0", ("syntax@2.0.0", None)),
                    node("syntax@2.0.0"),
                    node("cc@1.0.0"),
                    node("mock@0.1.0", ("web@1.0.0", None)),
                    node("forked@0.2.0"),
                ],
            },
        }
        found = {item["name"]: item for item in dependencies.backend_dependencies(metadata)}

        self.assertNotIn("thoric", found)
        self.assertEqual(found["web"]["scope"], "runtime")
        self.assertTrue(found["web"]["direct"])
        self.assertEqual(found["web"]["url"], "https://crates.io/crates/web/1.0.0")
        self.assertEqual(found["forked"]["scope"], "runtime")
        self.assertEqual(found["forked"]["url"], "https://example.org/forked")
        # Procedural macros and build scripts run while compiling and are not in the binary.
        self.assertEqual(found["derive"]["scope"], "build")
        self.assertTrue(found["derive"]["direct"])
        self.assertEqual(found["syntax"]["scope"], "build")
        self.assertEqual(found["cc"]["scope"], "build")
        self.assertEqual(found["mock"]["scope"], "development")
        self.assertEqual(found["mock"]["license"], "SEE LICENSE IN LICENSE.txt")

    def test_shipped_packages_need_a_license(self):
        listing = {
            "dependencies": [
                {"group": "backend", "name": "a", "version": "1", "scope": "runtime"},
                {"group": "backend", "name": "b", "version": "1", "scope": "build"},
                {"group": "ui", "name": "c", "version": "1", "scope": "runtime", "license": "MIT"},
                {"group": "infrastructure", "name": "d", "kind": "package", "used_in": ["wsl"]},
            ]
        }
        self.assertEqual(dependencies.unlicensed(listing), ["backend a 1"])


class InfrastructureTest(unittest.TestCase):
    def test_dockerfile_images_packages_tools_and_downloads(self):
        text = """
# syntax comment
ARG GL=3.1.5
FROM --platform=linux/amd64 rust:1-bookworm AS chef
RUN cargo install cargo-chef --locked --version 0.1.78
FROM chef AS build
RUN apt-get update && apt-get install -y --no-install-recommends \\
    # a comment between continued lines
    gcc libc6-dev=2.39-0ubuntu8 \\
    && curl -fL "https://github.com/VirtualGL/virtualgl/releases/download/${GL}/virtualgl_${GL}_amd64.deb" -o /tmp/gl.deb \\
    && apt-get install -y /tmp/gl.deb
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y curl
"""
        found = by_name(dependencies.dockerfile_dependencies(text, "manager"))
        self.assertEqual(found[("rust", "1-bookworm")]["kind"], "image")
        self.assertEqual(found[("rust", "1-bookworm")]["url"], "https://hub.docker.com/_/rust")
        self.assertEqual(found[("cargo-chef", "0.1.78")]["kind"], "tool")
        # A stage built from another one installs onto the image that one started from.
        self.assertEqual(found[("gcc", None)]["source"], "rust:1-bookworm")
        self.assertEqual(found[("libc6-dev", "2.39-0ubuntu8")]["kind"], "package")
        self.assertEqual(found[("virtualgl", "3.1.5")]["url"], "https://github.com/VirtualGL/virtualgl")
        self.assertEqual(found[("curl", None)]["source"], "debian:bookworm-slim")
        self.assertEqual(found[("curl", None)]["used_in"], ["manager"])
        self.assertNotIn(("chef", "latest"), found)
        self.assertFalse(any(name.endswith(".deb") or name.startswith("/") for name, _ in found))

    def test_compose_names_images_but_thoric_own(self):
        text = """
services:
  manager:
    image: ${MANAGER_IMAGE:-ghcr.io/co3moz/thoric:latest}
  local:
    image: thoric/manager:local
  xfce:
    image: ghcr.io/co3moz/thoric/ubuntu-xfce:24.04
  https:
    image: "caddy:2-alpine"
  proxy:
    image: ghcr.io/example/proxy@sha256:abc
"""
        found = dependencies.compose_dependencies(text)
        self.assertEqual(
            [(item["name"], item["version"], item.get("url")) for item in found],
            [("caddy", "2-alpine", "https://hub.docker.com/_/caddy"), ("ghcr.io/example/proxy", "sha256:abc", None)],
        )

    def test_workflow_actions_tools_and_runners(self):
        text = """
jobs:
  checks:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
      - uses: oven-sh/setup-bun@v2
        with:
          bun-version: 1.4.3
      - uses: github/codeql-action/init@abc # v3
      - uses: ./.github/actions/local
      - run: rustup toolchain install stable --profile minimal
  build:
    strategy:
      matrix:
        include:
          - arch: arm64
            runner: ubuntu-24.04-arm
    runs-on: ${{ matrix.runner }}
"""
        found = by_name(dependencies.workflow_dependencies(text))
        # A runner the matrix names, and nothing for the expression that picks it.
        runners = {name for (name, _), item in found.items() if item["kind"] == "runner"}
        self.assertEqual(runners, {"ubuntu-24.04", "ubuntu-24.04-arm"})
        checkout = found[("actions/checkout", "v7.0.1")]
        self.assertEqual(checkout["pin"], "3d3c42e5aac5ba805825da76410c181273ba90b1")
        self.assertEqual(found[("oven-sh/setup-bun", "v2")].get("pin"), None)
        self.assertEqual(found[("github/codeql-action/init", "v3")]["url"], "https://github.com/github/codeql-action")
        self.assertEqual(found[("bun", "1.4.3")]["kind"], "tool")
        self.assertEqual(found[("rust", "stable")]["kind"], "tool")
        self.assertEqual(found[("ubuntu-24.04", None)]["kind"], "runner")
        self.assertFalse(any(name.startswith("./") for name, _ in found))

    def test_one_entry_per_dependency_with_every_place(self):
        found = dependencies.merged(
            dependencies.dockerfile_dependencies("FROM ubuntu:24.04\nRUN apt-get install -y curl", "wsl")
            + dependencies.dockerfile_dependencies("FROM ubuntu:24.04\nRUN apt-get install -y curl", "desktop")
        )
        self.assertEqual([(item["name"], item["used_in"]) for item in found], [("ubuntu", ["desktop", "wsl"]), ("curl", ["desktop", "wsl"])])


class RepositoryTest(unittest.TestCase):
    def test_every_dockerfile_is_listed(self):
        # Generated, installed and local data folders, and other worktrees, are not the repository.
        skipped = {"node_modules", "target", "dist", "data", ".git", ".claude"}
        found = set()
        for folder, folders, files in os.walk(ROOT):
            folders[:] = [name for name in folders if name not in skipped]
            if "Dockerfile" in files:
                found.add((Path(folder) / "Dockerfile").relative_to(ROOT).as_posix())
        self.assertEqual(found, set(dependencies.DOCKERFILES))

    def test_every_workflow_action_is_pinned_to_a_release(self):
        actions = [
            item for item in dependencies.infrastructure_dependencies(ROOT) if item["kind"] == "action"
        ]
        self.assertTrue(actions)
        for action in actions:
            self.assertRegex(action.get("pin", ""), r"^[0-9a-f]{40}$", action["name"])
            self.assertRegex(action["version"], r"^v\d", action["name"])

    def test_every_place_and_kind_has_a_label(self):
        """The page names places and kinds from the interface's catalog, which must know each."""
        catalog = (ROOT / "core/web/src/locales/en.ts").read_text(encoding="utf-8")
        listed = dependencies.infrastructure_dependencies(ROOT)
        self.assertLessEqual({place for item in listed for place in item["used_in"]}, set(dependencies.PLACES))
        self.assertLessEqual({item["kind"] for item in listed}, set(dependencies.KINDS))
        for place in dependencies.PLACES:
            self.assertTrue(f'"dependencies.place.{place}":' in catalog, f"en.ts has no label for {place}")
        for kind in dependencies.KINDS:
            self.assertTrue(f'"dependencies.kind.{kind}":' in catalog, f"en.ts has no label for {kind}")


if __name__ == "__main__":
    unittest.main()
