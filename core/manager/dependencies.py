#!/usr/bin/env python3
"""Write every dependency Thoric is built from as JSON, the list the Dependencies page shows.

The interface's packages come from core/web/bun.lock, with their licenses from
core/web/node_modules; the manager's crates from `cargo metadata`; and the infrastructure (images,
system packages, tools, downloads and workflow actions) from the Dockerfiles, the Compose manifests
and the workflows. The manager image's build runs it, so the list is the one that image was built
from.

    python3 core/manager/dependencies.py [--strict] [--platform linux/arm64] [--output FILE]

With --strict it fails when a package that ships in the interface or the manager names no license.
--platform names the platform the image is built for, linux/amd64 by default, whose interface
packages and crates are listed.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

FORMAT = 1
ROOT = Path(__file__).resolve().parents[2]
# Every Dockerfile in the repository and the place its packages end up, as the page names it.
DOCKERFILES = {
    "core/manager/Dockerfile": "manager",
    "images/desktop/Dockerfile": "desktop",
    "packaging/wsl/Dockerfile": "wsl",
}
COMPOSE_FILES = ("compose.yml", "compose.ghcr.yml")
WORKFLOWS = ".github/workflows"
PLACES = ("manager", "desktop", "wsl", "compose", "ci")
KINDS = ("image", "package", "tool", "download", "action", "runner")
# The platforms the images are built for, each with the `os` and `cpu` its interface packages are
# installed for and the target cargo builds the manager for. Packages of other platforms are not
# used, and each platform's manager image lists its own.
PLATFORMS = {
    "linux/amd64": ({"os": "linux", "cpu": "x64"}, "x86_64-unknown-linux-gnu"),
    "linux/arm64": ({"os": "linux", "cpu": "arm64"}, "aarch64-unknown-linux-gnu"),
}
NPM_PLATFORM, CARGO_TARGET = PLATFORMS["linux/amd64"]
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
# The images Thoric publishes or builds itself are not its dependencies.
OWN_IMAGE = re.compile(r"^(?:(?:ghcr\.io|docker\.io)/)?(?:co3moz/)?thoric(?:[/:-]|$)")
VARIABLE = re.compile(r"\$\{(\w+)(?::-([^}]*))?\}|\$(\w+)")
RELEASE_DOWNLOAD = re.compile(r"https://github\.com/([\w.-]+)/([\w.-]+)/releases/download/([^/\s\"']+)/")
DESCRIPTION_LIMIT = 200


def entry(group, name, version, **fields):
    """One dependency, without the fields it has no value for."""
    found = {"group": group, "name": name, "version": version or None, **fields}
    return {key: value for key, value in found.items() if value is not None}


def short(text):
    text = " ".join(str(text or "").split())
    return text[: DESCRIPTION_LIMIT - 1] + "…" if len(text) > DESCRIPTION_LIMIT else text or None


# Interface


def without_trailing_commas(text):
    """bun.lock is JSON apart from the commas Bun writes after the last item of a list or object."""
    return re.sub(r'("(?:[^"\\]|\\.)*")|,(\s*[}\]])', lambda m: m.group(1) or m.group(2), text)


def package_path(key):
    """The packages a bun.lock key nests: 'a/@b/c/d' is d, installed inside @b/c inside a."""
    parts = key.split("/")
    names = []
    while parts:
        part = parts.pop(0)
        names.append(f"{part}/{parts.pop(0)}" if part.startswith("@") and parts else part)
    return names


def split_ident(ident):
    """'@scope/name@1.2.3' into its name and version."""
    at = ident.index("@", 1)
    return ident[:at], ident[at + 1 :]


def runs_on(meta, field, wanted):
    """Whether an `os` or `cpu` restriction of a package allows `wanted`."""
    value = meta.get(field)
    if value is None:
        return True
    values = [value] if isinstance(value, str) else list(value)
    if f"!{wanted}" in values:
        return False
    allowed = [item for item in values if not item.startswith("!")]
    return not allowed or wanted in allowed


def npm_license(manifest):
    value = manifest.get("license")
    if isinstance(value, dict):
        value = value.get("type")
    if not value and isinstance(manifest.get("licenses"), list):
        value = " OR ".join(
            str(item.get("type") if isinstance(item, dict) else item) for item in manifest["licenses"]
        )
    return value or None


def installed_manifest(node_modules, key, name, version):
    """The package.json Bun installed for a lockfile key, in a hoisted or an isolated layout."""
    hoisted = node_modules.joinpath("/node_modules/".join(package_path(key)))
    isolated = node_modules / ".bun" / f"{name.replace('/', '+')}@{version}" / "node_modules" / name
    for folder in (hoisted, isolated):
        try:
            manifest = json.loads((folder / "package.json").read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        if isinstance(manifest, dict) and manifest.get("version") == version:
            return manifest
    return None


def ui_dependencies(lock, node_modules, platform=NPM_PLATFORM):
    """Every package of the lockfile on the platform: runtime when the interface's own
    dependencies reach it, development when only the build tools do."""
    workspace = lock["workspaces"][""]
    packages = {}
    for key, value in lock["packages"].items():
        meta = next((item for item in value if isinstance(item, dict)), {})
        if all(runs_on(meta, field, wanted) for field, wanted in platform.items()):
            packages[key] = (value[0], meta)

    def resolve(parent, name):
        # Node's lookup: the nearest node_modules folder up from the package that needs it.
        for depth in range(len(parent), -1, -1):
            key = "/".join(parent[:depth] + [name])
            if key in packages:
                return key
        return None

    def reach(names):
        seen = set()
        todo = [key for key in (resolve([], name) for name in names) if key]
        while todo:
            key = todo.pop()
            if key in seen:
                continue
            seen.add(key)
            meta = packages[key][1]
            # A peer the package can do without, such as type definitions, is the user's choice.
            optional = set(meta.get("optionalPeers", []))
            for field in ("dependencies", "optionalDependencies", "peerDependencies"):
                for name in meta.get(field, {}):
                    if field == "peerDependencies" and name in optional:
                        continue
                    found = resolve(package_path(key), name)
                    if found and found not in seen:
                        todo.append(found)
        return seen

    shipped = {
        name
        for field in ("dependencies", "optionalDependencies", "peerDependencies")
        for name in workspace.get(field, {})
    }
    tools = set(workspace.get("devDependencies", {}))
    runtime = reach(shipped)
    found = {}
    for key in sorted(runtime | reach(tools)):
        name, version = split_ident(packages[key][0])
        direct = len(package_path(key)) == 1 and name in shipped | tools
        scope = "runtime" if key in runtime else "development"
        known = found.get((name, version))
        if known:
            # The same release nested under several packages is one dependency.
            known["direct"] = known["direct"] or direct
            if scope == "runtime":
                known["scope"] = scope
            continue
        manifest = installed_manifest(node_modules, key, name, version) or {}
        found[(name, version)] = entry(
            "ui",
            name,
            version,
            license=npm_license(manifest),
            description=short(manifest.get("description")),
            scope=scope,
            direct=direct,
            url=f"https://www.npmjs.com/package/{name}",
        )
    return list(found.values())


# Manager


def cargo_metadata(manifest, target=CARGO_TARGET):
    output = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", target,
         "--manifest-path", str(manifest)],
        check=True,
        stdout=subprocess.PIPE,
    ).stdout
    return json.loads(output)


def backend_dependencies(metadata):
    """Every crate the manager builds on linux/amd64: runtime when it is compiled into the binary,
    build when only build scripts and procedural macros use it, development when only tests do."""
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    root = metadata["resolve"]["root"]

    def proc_macro(package):
        return any("proc-macro" in target.get("kind", []) for target in packages[package]["targets"])

    def edges(node):
        for dep in nodes[node]["deps"]:
            for kind in {item.get("kind") or "normal" for item in dep["dep_kinds"]}:
                yield dep["pkg"], "build" if kind == "normal" and proc_macro(dep["pkg"]) else kind

    def reach(starts, follow):
        seen = set()
        todo = list(starts)
        while todo:
            node = todo.pop()
            if node in seen:
                continue
            seen.add(node)
            todo.extend(package for package, kind in edges(node) if kind in follow)
        return seen

    direct = list(edges(root))
    runtime = reach([package for package, kind in direct if kind == "normal"], {"normal"})
    build_starts = [package for package, kind in direct if kind == "build"]
    build_starts += [package for node in runtime for package, kind in edges(node) if kind == "build"]
    build = reach(build_starts, {"normal", "build"}) - runtime
    development = reach([package for package, kind in direct if kind == "dev"], {"normal", "build"})
    development -= runtime | build
    named = {package for package, _ in direct}

    found = []
    for scope, ids in (("runtime", runtime), ("build", build), ("development", development)):
        for package_id in ids:
            package = packages[package_id]
            license_file = package.get("license_file")
            found.append(
                entry(
                    "backend",
                    package["name"],
                    package["version"],
                    license=package.get("license")
                    or (f"SEE LICENSE IN {Path(license_file).name}" if license_file else None),
                    description=short(package.get("description")),
                    scope=scope,
                    direct=package_id in named,
                    url=f"https://crates.io/crates/{package['name']}/{package['version']}"
                    if package.get("source") == CRATES_IO
                    else package.get("repository"),
                )
            )
    return found


# Infrastructure


def expand(text, variables):
    """`$NAME`, `${NAME}` and `${NAME:-default}` from the values known, others as written."""

    def value(match):
        name = match.group(1) or match.group(3)
        if name in variables:
            return variables[name]
        return match.group(2) if match.group(2) is not None else match.group(0)

    return VARIABLE.sub(value, text)


def logical_lines(text):
    """A Dockerfile's instructions, with continued lines joined and comments left out."""
    lines, current = [], ""
    for raw in text.splitlines():
        stripped = raw.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if stripped.endswith("\\"):
            current += stripped[:-1] + " "
            continue
        lines.append(current + stripped)
        current = ""
    if current:
        lines.append(current)
    return lines


def split_image(reference):
    name, at, digest = reference.partition("@")
    if at:
        return name, digest
    slash, colon = name.rfind("/"), name.rfind(":")
    return (name[:colon], name[colon + 1 :]) if colon > slash else (name, "latest")


def image_entry(reference, place):
    name, version = split_image(reference)
    first = name.split("/")[0]
    if "." in first or ":" in first:
        url = None
    else:
        url = f"https://hub.docker.com/{'r/' + name if '/' in name else '_/' + name}"
    return entry("infrastructure", name, version, kind="image", used_in=[place], url=url)


def run_dependencies(command, base, place):
    """The system packages, tools and release downloads a RUN instruction installs."""
    found = []
    for segment in re.split(r"&&|\|\||;|\|", command):
        words = [word.strip("'\"") for word in segment.split()]
        apt = next((index for index, word in enumerate(words) if word in ("apt-get", "apt")), None)
        if apt is not None and "install" in words[apt + 1 :]:
            for word in words[words.index("install", apt) + 1 :]:
                # Options, and local .deb files, which a download below names.
                if word and not word.startswith("-") and "/" not in word:
                    name, _, version = word.partition("=")
                    found.append(
                        entry("infrastructure", name, version, kind="package", source=base, used_in=[place])
                    )
        if words[:2] == ["cargo", "install"]:
            rest = words[2:]
            version = rest[rest.index("--version") + 1] if "--version" in rest[:-1] else None
            skip = {version}
            for word in rest:
                if word not in skip and not word.startswith("-"):
                    found.append(
                        entry(
                            "infrastructure",
                            word,
                            version,
                            kind="tool",
                            used_in=[place],
                            url=f"https://crates.io/crates/{word}",
                        )
                    )
    for owner, repository, version in RELEASE_DOWNLOAD.findall(command):
        found.append(
            entry(
                "infrastructure",
                repository,
                version,
                kind="download",
                used_in=[place],
                url=f"https://github.com/{owner}/{repository}",
            )
        )
    return found


def dockerfile_dependencies(text, place):
    """The images a Dockerfile starts from and what its RUN instructions install, each system
    package with the image of the stage that installs it."""
    found = []
    variables = {}
    stages = {}
    base = None
    for line in logical_lines(text):
        instruction, _, rest = line.partition(" ")
        instruction = instruction.upper()
        if instruction == "ARG":
            name, assigned, default = rest.strip().partition("=")
            if assigned:
                variables[name.strip()] = default.strip().strip("\"'")
        elif instruction == "FROM":
            words = [word for word in expand(rest, variables).split() if not word.startswith("--")]
            image = words[0]
            if image.lower() in stages:
                base = stages[image.lower()]
            else:
                base = image
                found.append(image_entry(image, place))
            if len(words) >= 3 and words[1].lower() == "as":
                stages[words[2].lower()] = base
        elif instruction == "RUN":
            found.extend(run_dependencies(expand(rest, variables), base, place))
    return found


def compose_dependencies(text):
    found = []
    for match in re.finditer(r"^\s*image:\s*[\"']?([^\"'\s#]+)", text, re.M):
        reference = expand(match.group(1), {})
        if not OWN_IMAGE.match(reference):
            found.append(image_entry(reference, "compose"))
    return found


def workflow_dependencies(text):
    """The actions a workflow uses, at the release their pin's comment names, its runners and the
    toolchain versions it installs."""
    found = []
    uses = r"^\s*(?:-\s*)?uses:\s*[\"']?([^@\s\"']+)@([^\s\"'#]+)[\"']?(?:\s*#\s*(\S+))?"
    for name, ref, release in re.findall(uses, text, re.M):
        if name.startswith("./"):
            continue
        if name.startswith("docker://"):
            found.append(image_entry(f"{name[len('docker://'):]}@{ref}", "ci"))
            continue
        found.append(
            entry(
                "infrastructure",
                name,
                release or ref,
                kind="action",
                pin=ref if release else None,
                used_in=["ci"],
                url=f"https://github.com/{'/'.join(name.split('/')[:2])}",
            )
        )
    for tool, version in re.findall(r"^\s*([a-z][\w-]*?)-version:\s*[\"']?([\w.+-]+)", text, re.M):
        found.append(entry("infrastructure", tool, version, kind="tool", used_in=["ci"]))
    for version in re.findall(r"rustup toolchain install (\w[\w.-]*)", text):
        found.append(entry("infrastructure", "rust", version, kind="tool", used_in=["ci"]))
    # A job's runner, or each runner of its matrix, which names them as `runner`.
    for runner in re.findall(r"^\s*(?:runs-on|(?:-\s*)?runner):\s*[\"']?([\w.-]+)", text, re.M):
        found.append(entry("infrastructure", runner, None, kind="runner", used_in=["ci"]))
    return found


def merged(found):
    """One entry per dependency, with every place that uses it."""
    by_key = {}
    for item in found:
        key = (item["kind"], item["name"], item.get("version"), item.get("source"))
        known = by_key.setdefault(key, {**item, "used_in": []})
        known["used_in"] = sorted(set(known["used_in"]) | set(item["used_in"]), key=PLACES.index)
    return list(by_key.values())


def infrastructure_dependencies(root):
    found = []
    for path, place in DOCKERFILES.items():
        found += dockerfile_dependencies((root / path).read_text(encoding="utf-8"), place)
    for path in COMPOSE_FILES:
        found += compose_dependencies((root / path).read_text(encoding="utf-8"))
    for path in sorted((root / WORKFLOWS).glob("*.y*ml")):
        found += workflow_dependencies(path.read_text(encoding="utf-8"))
    return merged(found)


# The list


def dependency_list(root, metadata, platform="linux/amd64"):
    lock = json.loads(without_trailing_commas((root / "core/web/bun.lock").read_text(encoding="utf-8")))
    version = next(
        package["version"] for package in metadata["packages"] if package["id"] == metadata["resolve"]["root"]
    )
    groups = {"ui": 0, "backend": 1, "infrastructure": 2}
    found = (
        ui_dependencies(lock, root / "core/web/node_modules", PLATFORMS[platform][0])
        + backend_dependencies(metadata)
        + infrastructure_dependencies(root)
    )
    found.sort(key=lambda item: (groups[item["group"]], item["name"].lower(), item.get("version") or ""))
    return {"format": FORMAT, "version": version, "platform": platform, "dependencies": found}


def unlicensed(listing):
    """Packages that ship in the interface or the manager without a license."""
    return [
        f"{item['group']} {item['name']} {item.get('version')}"
        for item in listing["dependencies"]
        if item["group"] != "infrastructure" and item.get("scope") == "runtime" and not item.get("license")
    ]


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--output", type=Path, help="write the list to this file instead of stdout")
    parser.add_argument(
        "--strict", action="store_true", help="fail when a shipped package names no license"
    )
    parser.add_argument(
        "--platform", choices=sorted(PLATFORMS), default="linux/amd64",
        help="the platform the image is built for, whose packages are listed",
    )
    args = parser.parse_args(argv)
    metadata = cargo_metadata(ROOT / "core/manager/Cargo.toml", PLATFORMS[args.platform][1])
    listing = dependency_list(ROOT, metadata, args.platform)
    missing = unlicensed(listing)
    if missing:
        print(
            "No license found for: " + ", ".join(missing) + ". Is core/web/node_modules installed?",
            file=sys.stderr,
        )
        if args.strict:
            return 1
    text = json.dumps(listing, separators=(",", ":")) + "\n"
    if args.output:
        args.output.write_text(text, encoding="utf-8", newline="\n")
    else:
        sys.stdout.write(text)
    counts = {group: 0 for group in ("ui", "backend", "infrastructure")}
    for item in listing["dependencies"]:
        counts[item["group"]] += 1
    print(
        f"Listed {counts['ui']} interface packages, {counts['backend']} crates and "
        f"{counts['infrastructure']} infrastructure dependencies.",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
