# Developing Thoric

[Back to the README](../README.md)

Thoric has a Rust/Axum backend and a React and TypeScript frontend built with Vite, Tailwind CSS and shadcn/ui components, with [Bun](https://bun.sh) as its package manager and script runner. The manager uses the Docker CLI through the host Engine socket. Desktop images contain Ubuntu, the desktop session, VNC and the graphics helpers.

Read [AGENTS.md](../AGENTS.md) for implementation constraints and commit rules before changing the project.

## Build and run from source

In a checkout inside WSL, copy the example settings and set `ADMIN_PASSWORD`, or use `AUTH_MODE=none` for a private local development instance:

```bash
cp .env.example .env
docker compose --profile images build desktop desktop-gnome manager
docker compose up -d --no-build manager
```

This builds both catalog images and the manager. The manager is tagged `thoric/manager:local`. Desktop builds use the GHCR references in the image catalog, so local builds and registry installations agree on image names.

Builds are for the host's own architecture, x86_64 or arm64, such as on a Raspberry Pi or a Mac. To build the other one on an x86_64 host, register QEMU's arm64 emulation with Docker first, which lasts until the next restart of the Docker host (in WSL, of WSL), and give the platform to the build; it takes several times as long:

```bash
docker run --privileged --rm tonistiigi/binfmt:qemu-v10.2.3-68 --install arm64
docker buildx build --platform linux/arm64 --target xfce -t thoric-arm64/ubuntu-xfce:local --load images/desktop
docker buildx build --platform linux/arm64 -f core/manager/Dockerfile -t thoric-arm64/manager:local --load .
```

When a catalog image is missing, its row under **Images** offers a download button, which pulls the published image, and a build button, which builds it from the manager's bundled Docker context. Creating a container or an image from a missing catalog image downloads it in that job. Pulls use the Engine API socket so the job can report byte progress; the socket follows `DOCKER_HOST`, and a daemon that is not on a Unix socket is pulled through `docker pull` without byte counts. Script builds, snapshots and imported images follow the catalog images in the same list.

## Find the code

The projects sit in three folders: `core/` holds the manager (`core/manager`, Rust with the Python helpers it runs in containers under `helpers/`), the dashboard (`core/web`) and the MCP stdio bridge (`core/bridge`); `images/desktop` builds the desktop images; `packaging/` holds the WSL distribution (`packaging/wsl`), its Windows launcher (`packaging/windows`) and the Linux and macOS helper (`packaging/unix`). The Cargo workspace, the Compose manifests, `.env.example`, `deploy/`, `docs/` and the acceptance suite in `tests/` stay at the root. Each project keeps its Python tests in a `tests/` folder of its own.

| Area | Files |
| --- | --- |
| HTTP API, settings and authentication | `core/manager/src/main.rs`, `core/manager/src/api.rs`, `core/manager/src/api/` (a container's routes and an image's), `core/manager/src/config.rs`, `core/manager/src/auth.rs`, `core/manager/src/auth/` (the sign-in limit and the HTTPS proxy's forwarded address) |
| OpenAPI document of the HTTP API | `core/manager/src/openapi.rs`, `core/manager/src/openapi/routes.rs` |
| Error statuses: a `bail!` is the request's (400), a failure of Docker, a helper or the manager is marked where it happens with `error::Fault`, or is an I/O or database error | `core/manager/src/error.rs` |
| Request log, request IDs, health checks and repeated failures | `core/manager/src/request_log.rs`, `core/manager/src/health.rs`, `core/manager/src/reported.rs` |
| Docker lifecycle and jobs | `core/manager/src/docker.rs`, `core/manager/src/docker/` (a container's lifecycle, its `docker create` arguments, its networks and the commands a job logs), `core/manager/src/jobs.rs`, `core/manager/src/process.rs` |
| Catalog image downloads and their progress | `core/manager/src/pulls.rs` |
| Settings changes from the Management page through the WSL package | `core/manager/src/config/changes.rs`, `core/manager/src/package_requests.rs`, `core/web/src/components/configuration-editor.tsx`, `core/web/src/lib/settings.ts`, `packaging/wsl/thoric-settings.path`, `packaging/wsl/thoric-settings.service` |
| Settings changes after creation | `core/manager/src/reconfigure.rs` |
| Composes and their setup runs | `core/manager/src/compose.rs`, `core/manager/helpers/setup_runner.py`, `core/manager/tests/test_setup_runner.py`, `core/web/src/lib/compose.ts`, `core/web/src/components/dialogs/compose-dialog.tsx` |
| Metadata and image catalog | `core/manager/src/db.rs`, `core/manager/src/model.rs`, `core/manager/src/catalog.rs`, `images/desktop/profiles.json` |
| VNC, audio and microphone proxies | `core/manager/src/media.rs`, `core/manager/helpers/audio_output.py` |
| MCP server, command execution and desktop input | `core/manager/src/mcp.rs`, `core/manager/src/mcp/` (the tool catalog and what each tool runs), `core/manager/src/exec.rs`, `core/manager/src/desktop_input.rs`, `core/manager/src/desktop_input/relative.rs` |
| Background applications, windows, screenshots and waits | `core/manager/src/apps.rs`, `core/manager/helpers/app_runner.py`, `core/manager/helpers/app_control.py`, `core/manager/src/windows.rs`, `core/manager/src/capture.rs`, `core/manager/src/waits.rs` |
| Text on the screen with Tesseract OCR | `core/manager/src/ocr.rs`, `core/manager/src/ocr/words.rs`, `core/manager/src/capture/pixels.rs` |
| Recordings, their video and artifact export | `core/manager/src/recordings.rs`, `core/manager/helpers/recorder.py`, `core/manager/helpers/recording_video.py`, `core/manager/helpers/export_artifacts.py` |
| Audio capture and injection | `core/manager/src/audio.rs`, `core/manager/helpers/audio_tools.py` |
| Interactive terminals | `core/manager/src/terminal.rs`, `core/manager/helpers/terminal_runner.py`, `core/manager/helpers/terminal_control.py` |
| Virtual gamepad | `core/manager/src/gamepad.rs`, `core/manager/helpers/gamepad_daemon.py`, `core/manager/helpers/gamepad_control.py` |
| Desktop proxies | `core/manager/src/outbound.rs`, `core/manager/src/outbound/`, `core/manager/helpers/outbound_proxy.py`, `core/manager/helpers/outbound_control.py`, `core/web/src/lib/proxy.ts` |
| Container groups | `core/manager/src/groups.rs`, `core/web/src/lib/groups.ts`, `core/web/tests/name-labels.json` |
| Accessibility inspection and file download | `core/manager/helpers/inspect_ui.py` (in `core/manager/src/windows.rs`), `core/manager/helpers/download_file.py` (in `core/manager/src/files.rs`) |
| Stdio bridge for local agent clients | `core/bridge/mcp_bridge.py`, `core/bridge/tests/test_mcp_bridge.py` |
| File read/write/list/move/delete API | `core/manager/src/files.rs`, `core/manager/helpers/read_file.py`, `core/manager/helpers/upload_file.py`, `core/manager/helpers/file_ops.py` |
| What changed in a container's files | `core/manager/src/changes.rs`, `core/manager/src/changes/source.rs`, `core/manager/src/changes/noise.rs`, `core/manager/helpers/file_ops.py` (`stats`), `core/manager/tests/test_file_ops.py`, `core/web/src/components/container-changes.tsx`, `core/web/src/lib/changes.ts` |
| Browser uploads and container delivery | `core/manager/src/uploads.rs`, `core/manager/helpers/upload_file.py`, `core/web/src/lib/uploads.ts` |
| Folder downloads as `.tar.gz` | `core/manager/src/folder_archive.rs`, `core/manager/helpers/archive_folder.py`, `core/web/src/components/dialogs/download-dialog.tsx`, `core/manager/tests/test_archive_folder.py` |
| Volume files through a helper container | `core/manager/src/volumes.rs`, `core/manager/src/volumes/`, `core/manager/helpers/volume_claim.py`, `core/web/src/components/dialogs/volume-browser.tsx`, `core/web/src/lib/volume-files.ts` |
| Dashboard layout, pages and dialogs | `core/web/src/App.tsx`, `core/web/src/pages/`, `core/web/src/components/`, `core/web/src/components/dialogs/` |
| shadcn/ui components and theme | `core/web/src/components/ui/`, `core/web/src/index.css`, `core/web/components.json` |
| API client, queries and pure helpers | `core/web/src/lib/` (`api.ts`, `queries.ts`, `format.ts`, `route.ts`) |
| Languages of the interface | `core/web/src/lib/i18n.ts`, `core/web/src/locales/`, `core/web/src/components/language-menu.tsx`, `core/web/tests/i18n.test.ts` |
| Remote desktop, audio and microphone | `core/web/src/pages/desktop.tsx`, `core/web/src/lib/media.ts`, `core/web/public/audio-worklet.js` |
| Desktop images and startup | `images/desktop/Dockerfile`, `images/desktop/desktop-*.sh`, `images/desktop/gpu-run.sh`, `images/desktop/chromium-gpu.c` |
| Dependency list of the Dependencies page | `core/manager/dependencies.py`, `core/manager/tests/test_dependencies.py`, `core/manager/src/dependencies.rs`, `core/web/src/lib/dependencies.ts`, `core/web/src/pages/dependencies.tsx` |
| Builds, tests and publishing | `core/manager/Dockerfile`, `tests/`, each project's `tests/`, `core/web/tests/`, `.github/workflows/publish.yml` |
| Toolchain, dependency updates and audits | `rust-toolchain.toml`, `.github/dependabot.yml`, `.cargo/audit.toml`, `tests/test_pinned_versions.py` |
| WSL distribution package | `packaging/wsl/`, `packaging/wsl/build.sh`, `packaging/windows/thoric.ps1` |
| Linux and macOS helper | `packaging/unix/thoric.sh`, `packaging/unix/tests/test_unix_helper.py` |

`core/web/dist/` is generated by the frontend build and is not committed. The manager's Dockerfile, `core/manager/Dockerfile`, builds from the repository root: the frontend in the `oven/bun` image and the Rust binary, then it packages them with the desktop build context and installation files. The Rust dependencies are compiled in a layer of their own through `cargo-chef`, so a source change or a version bump reuses them from the build cache; only a `Cargo.lock` change, a dependency change in `core/manager/Cargo.toml` or a new `rust` image, which moves with `rust-toolchain.toml`, compiles them again.

The `dependencies` stage writes the list of the dashboard's **Dependencies** page with `core/manager/dependencies.py`: the interface's packages from `core/web/bun.lock` with the licenses of the installed ones, the crates from `cargo metadata`, and the images, APT packages, tools, release downloads and workflow actions the Dockerfiles, Compose manifests and workflows name. It runs on the stage that compiled the dependencies, with the interface's build mounted, and the image keeps the result at `/usr/share/doc/thoric/dependencies.json`, which `GET /api/dependencies` serves. With `--strict` it fails when a package that ships in the interface or the manager names no license. The `checks` job runs it the same way, so a lockfile, Dockerfile or workflow it cannot read fails a pull request before an image builds. A new Dockerfile goes into `DOCKERFILES` in the script, which `core/manager/tests/test_dependencies.py` holds to the Dockerfiles in the repository, and a new place or kind gets its label in every catalog. To look at the list without building the image, run it from the repository root once `core/web/node_modules` is installed:

```bash
python3 core/manager/dependencies.py --strict --output dependencies.json
```

## Work on the interface

The Vite project lives in `core/web/`: `core/web/index.html`, the React sources under `core/web/src`, static files under `core/web/public`, which are served as they are, the unit tests under `core/web/tests` and the toolchain beside them (`package.json`, `vite.config.ts`, the TypeScript projects, ESLint and Prettier). Every `bun` command below runs in `core/web/`. Pages live in the URL fragment (`#/images`, `#/desktop/<id>`), so the manager serves one `index.html` for all of them. Vite names the files under `core/web/dist/assets/` after their content, and the manager lets browsers keep those for a year; `index.html` and the other files are revalidated on every load, so an upgraded manager never serves a page that points at assets it no longer has.

The build writes the license of every package in the bundle to `third-party-licenses.txt`, taking the text from `core/web/licenses/<SPDX>.txt` for a package that ships no license file and adding the license file of code a package vendors (noVNC's `vendor/pako`), and stops when a package's license is not one Thoric's GPL-3.0-or-later can carry, as listed in `GPL_COMPATIBLE` in `core/web/src/lib/licenses.ts`. That is why `@remixicon/react` stays at 4.8.0: later releases moved to the Remix Icon License, whose restrictions the GPL does not allow. Choose another release or another package rather than widening the list; a license goes into it only once it is known to be GPL-3.0 compatible.

The layout follows the shadcn `dashboard-01` block: an inset sidebar, a site header, summary cards, an interactive chart and data tables with a detail drawer. Add a shadcn component with `bunx shadcn@latest add <component>`; it reads `core/web/components.json` and writes to `core/web/src/components/ui/`, which is project code from then on. The CLI also tends to add the `shadcn` package, an `@import "shadcn/tailwind.css"` line and `core/web/src/lib/utils.ts`; drop them, since that stylesheet is vendored as `core/web/src/shadcn.css` and components import `cn` from the `cn` package. See [DESIGN.md](DESIGN.md) before changing colors or layout.

To work on the interface with hot reload, start a manager (for example with Compose, as above), then run the Vite server, which proxies `/api` including the WebSockets:

```bash
cd core/web
bun install
bun run dev
```

Open `http://localhost:5173`. Set `THORIC_URL` when the manager does not listen on `http://127.0.0.1:8080`.

### Translate the interface

Every visible string in `core/web/src` goes through `t()` from `core/web/src/lib/i18n.ts` with a key from `core/web/src/locales/en.ts`, the English catalog that the other languages follow: `de.ts`, `fr.ts`, `es.ts` and `tr.ts`. A key names where its text appears, `{name}` marks a value the code fills in, and a pair of keys ending in `.one` and `.other` holds the plural forms that `plural()` picks with the language's own rules. `rich()` fills placeholders with elements, such as a path in `<code>`, so each language keeps its own word order around them. Dates, numbers and lists follow the language through `formatDate`, `formatNumber` and `formatList`.

To add or change text, edit `en.ts` and give every other catalog the same key in the same change. The catalogs are typed against `en.ts`, so a missing or misspelled key fails `bun run build`, and `tests/i18n.test.ts` fails when a translation drops a placeholder, a plural form is missing its pair or a message uses a long dash. Call `t()` while rendering or handling an event, never at module scope: a change of language renders the interface again, which only reaches text looked up then.

The language is the browser's first one that has a catalog, English otherwise, until someone picks one in the account menu or on the sign-in page; that choice is kept in `localStorage`. To add a language, add its catalog, register it in `catalogs`, `localeNames` and `regions` in `i18n.ts`, and the tests check it like the others. Text that the manager returns, such as API errors and job logs, stays English.

## How the containers connect

The manager and desktop containers run on the same WSL Docker Engine. Only the manager receives `/var/run/docker.sock`.

Containers are created with `--security-opt seccomp=images/desktop/seccomp.json` unless `DESKTOP_USER_NAMESPACES=false`. The file is Docker's default profile from `github.com/moby/profiles/seccomp` v0.2.4, the version Docker Engine 29.9 vendors, with one rule appended that allows `clone` and `unshare`, so browser and Electron sandboxes can create user namespaces. To follow a newer Docker release, copy its `vendor/github.com/moby/profiles/seccomp/default.json` and append that rule again; the Rust tests check that the rule is present and that calls Docker reserves for `CAP_SYS_ADMIN` stay refused. Docker fixes the profile when it creates a container, so existing containers pick up a change only when they are recreated.

Containers also get `--pids-limit` from `DESKTOP_PIDS_LIMIT` and, while the daemon's logging driver is `json-file`, `--log-opt max-size=10m --log-opt max-file=3` (`core/manager/src/desktop_limits.rs`). The manager reads `docker info` once when it starts, for the driver and for the GPU routes, and leaves another driver as the daemon has it: `local` rotates by itself, and `journald` or `syslog` refuse `max-size`. Docker fixes log options at creation, but `docker update` changes the pids limit of a running container, so the 20-second reconciliation gives every desktop the current value. The volume and GPU helpers run with `--log-driver none`, since archives and samples stream through their output, and image builds run their steps in BuildKit, which keeps no container log. Desktops get neither `no-new-privileges` nor dropped capabilities: `sudo` inside them is a deliberate default. Not even one whose `sudo` is off, which would keep a setuid program root left behind from elevating: GNOME's system bus starts `accounts-daemon`, `polkitd` and `upowerd` through D-Bus's setuid activation helper, which refuses to run under the flag, and Chrome falls back to its setuid sandbox where `DESKTOP_USER_NAMESPACES=false`. Turning `sudo` off recreates the container from a commit of its files, so only a reset clears what root changed meanwhile.

The MCP server is mounted on the same Axum router at `/mcp`. Its bearer authentication is independent of browser sessions, but the outer Host policy and matching-Origin checks still apply. Tool calls reuse HTTP handlers and resource locks; do not implement separate Docker mutations for MCP. Mouse and keyboard control use shared RFB 3.8 connections and Fence acknowledgement; keys and buttons held within a request are released when it ends, and relative movement is emulated from the position reported by `xdotool`. Scripts run from an unlinked temporary file with stdin closed. Background applications are supervised by `app_runner.py` inside the desktop, which keeps a bounded log under `/tmp`; `app_control.py` reports, reads and stops them. Window tools use `xdotool` and `wmctrl`, and `desktop-screenshot` accepts a maximum width, a region and a window ID. Accessibility inspection uses `python3-pyatspi` and the session's AT-SPI bus. Terminals run bash on a pseudo-terminal supervised by `terminal_runner.py`. Containers created with `gamepad=true` get `/dev/uinput` and a device cgroup rule for input devices; `gamepad_daemon.py` runs as root inside the desktop only, creates the virtual controller and its `/dev/input` nodes, and never touches the host.

Each container has a private internal bridge to the manager. Browser desktop and media connections pass through the manager; VNC ports are not published on the host. A second bridge provides internet egress and is attached or detached by the container's internet setting. Each of its groups adds an internal bridge, `thoric-group-NAME`, shared with the group's other members and nothing else (`core/manager/src/groups.rs`). The container joins it under its hostname, its name in DNS form and its aliases, through the Engine API with its MAC address where it can, as the internet network. An endpoint keeps its aliases, so a new name connects it again. The network is created for the first member under a lock of the group, and removed once no container record lists the group: a stopped member has no endpoint the network shows, so the records decide. The startup reconciliation joins the groups again, also after a group network was removed by hand.

Docker gives an internal bridge an address on the host all the same, so the private and group networks are created with `--opt com.docker.network.bridge.gateway_mode_ipv4=isolated` where the Engine API is 1.48 (Docker Engine 28.0) or later, which an older engine refuses (`core/manager/src/docker/isolation.rs`). Containers on such a network still reach each other, and the manager reaches a desktop from its own container. It knows it runs there when Docker mounted its `/etc/hostname` from `MANAGER_CONTAINER`'s own folder (`/proc/self/mountinfo`); a manager run outside it reaches desktops through the host's address, so the networks it makes keep one, and it warns about a network made without one, whose desktop it cannot reach. A private network made again for an existing container, also one found missing, gets the subnet of the container's `DESKTOP_PRIVATE_SUBNET`, or is not made: in another subnet `desktop-private-address` finds nothing and VNC listens on every interface. A network made with an address is made again, with its subnet, only while nothing that runs is on it, which `docker ps --all --filter network=` tells, since `docker network inspect` leaves stopped containers out and `docker network rm` removes a network they are on: when its container is created again, and before a stopped desktop starts where the Engine API is 1.54 or later, since an older engine gives a container connected again a MAC address of its own.

VNC asks no password, so a desktop listens for it only on its address in the private network. The manager passes that network's IPv4 subnet as `DESKTOP_PRIVATE_SUBNET` when it creates the container, and `desktop-session` starts the VNC server with `-interface` set to the address `desktop-private-address` finds in it. Docker fixes the variable at creation, so a container created before the manager passed it listens on every interface until it is recreated. So does a desktop whose image, or the image its snapshot was taken from, predates the script, and one whose private network no longer has the address, which its log says. Desktop images carry `thoric.vnc=private` since groups exist, and a container created without the subnet gets `thoric.vnc=all` over it, so the container list can tell a desktop whose screen the members of its groups would reach.

Audio uses authenticated WebSockets carrying 48 kHz float32 PCM: stereo output at roughly 384 KB/s and mono microphone input at roughly 192 KB/s. The browser needs a secure context for microphone permission, so LAN microphone access uses HTTPS.

The database records immutable image IDs so later tag changes do not change an existing container's base. Operations on the same container are serialized, while unrelated containers and image builds can proceed independently.

Standard Docker CLI operations have a 120-second timeout; image builds and commits have a one-hour timeout, and a compose setup step its own `timeout_seconds` plus two minutes, at least an hour. Job logs retain their last 256 KiB, and the job list returns the latest 100 entries without loading their full logs.

Uploads stream through bounded staging before delivery as UID/GID 1001:1001. Destinations must remain within the desktop home, writable mounted volumes or the folders the container's compose names, and never under a read-only mount; `core/web/src/lib/desktop-paths.ts` mirrors that rule for the upload dialog. The receiver rejects symlink traversal and writes completed files atomically. Preserve these constraints when changing the upload path.

A volume's files are reached through a helper container started from the manager's own image with the volume at `/volume`, the same Python helpers as a running desktop, `--network none`, `--read-only`, `--cap-drop ALL` and `no-new-privileges`. Reads run as 1001:1001 on a read-only mount. Writes start as root with only `CHOWN`, `SETUID` and `SETGID` so `volume_claim.py` can give an empty, root-owned volume to the desktop user, then switch to 1001:1001 before running anything. A volume archive is written by tar as root on a read-only mount with only `DAC_READ_SEARCH` added, and an import creates a new volume and unpacks into it as 1001:1001 after the manager has gunzipped the upload against `IMAGE_ARCHIVE_MAX_MB`. Keep the Docker socket and host paths out of the helper, and keep the manager's own volumes refused.

## Add a catalog image

Add an entry to `images/desktop/profiles.json` with an ID, display name, image reference, session, description, `build_target` and `build_service`; every field is required. The file keeps its name because the WSL package's `thoric update` reads it from the manager. Then add the matching Dockerfile stage, startup session and desktop metadata labels; the `thoric.profile` label carries the catalog ID.

Update Compose and the publishing workflow if the new image needs its own build service or published image. Rebuild the manager so its catalog includes the image. Keep image references aligned across the catalog, configuration defaults and installation files.

## Add a template

The **Templates** page lists templates from the project's GitHub wiki. The wiki is a git repository of its own, `https://github.com/co3moz/thoric.wiki.git`, editable by the repository's collaborators. The page reads three kinds of files from its `templates/` folder through `raw.githubusercontent.com/wiki/co3moz/thoric/`:

```
templates/index.json   the list
templates/<id>.json    a template: a build configuration
templates/<id>.png     its icon, at most 256 by 256 pixels
```

`index.json` names each template once:

```json
{
  "templates": [
    {
      "id": "unity",
      "name": "Unity Hub",
      "description": "Unity Hub with a volume for editor installs.",
      "tags": ["games", "3d"],
      "requires": "0.5.0"
    },
    {
      "id": "node",
      "name": "Node.js",
      "description": "Node.js LTS with npm for building and testing JavaScript.",
      "tags": ["javascript"],
      "kind": "environment",
      "architectures": ["amd64", "arm64"]
    }
  ]
}
```

`id` is lowercase letters, digits and dashes, up to 64, and names the two files. `name` is 1 to 100 characters, `description` at most 500 and `tags` at most eight short texts. `kind` is `application`, the default, or `environment` for a language toolchain or another environment meant to be merged with an application; the page shows environments in a section of their own below the applications and refuses any other value. Releases before the field show every entry with the applications, so an environment needs no `requires` for it. `requires` is the oldest release that can build the template: set it when the template uses something newer than an installed release understands, such as a new compose parameter type, since that release would refuse the build. A release older than `requires` shows the template without **Install**.

`architectures` names the architectures of the Docker hosts the template builds on, as Docker names them: `amd64` for x86_64 and `arm64` for a Raspberry Pi or a Mac's Docker Desktop, at most eight. Without it the template counts as `amd64` only, since every template before the field was written and tried on x86_64. The page compares the list with `architecture` in `/api/settings` and shows a template that leaves the host's out in a folded section of its own, without **Install**, and **Merge and install** refuses it. Name `arm64` only once every download of the script has a build for it that the script picks, such as through `dpkg --print-architecture`, and the build has run on an arm64 host or under QEMU, as [building from source](#build-and-run-from-source) describes; a download made for x86_64 alone, such as Google Chrome's `.deb`, keeps a template `amd64` only. Releases before the field show every template on every host.

To make a template, build the image in the dashboard with **Build image**, give it its compose, description and icon, create a container from it and check that the setup works. Then **Export configuration** copies exactly the JSON `<id>.json` holds. Its `icon` line can be dropped, since the page takes the icon from `<id>.png`; the line is used only when that file is missing. Save the icon at 256 by 256 pixels.

An image can bring [MCP servers that run inside its desktops](MCP.md#mcp-servers-inside-a-desktop), such as a browser automation server attached to its Chrome. The build request declares them in `mcp`, which **Export configuration** includes, and the build writes each as `/etc/thoric/mcp.d/NAME.json` in the image, so every container created from it, and from its snapshots, has them. Each entry is the file's content with its `name`, a stdio command or a loopback URL, as the MCP guide describes; at most 16, each name once. A `${NAME}` in its arguments, variables, URL or headers is filled in from the container's compose values when the manager starts the server, so the template can let the user choose what the server gets:

```json
{
  "name": "Chrome with Playwright MCP",
  "base_image": "ghcr.io/co3moz/thoric/ubuntu-xfce:24.04",
  "script": "#!/usr/bin/env bash\n...",
  "compose": {"parameters": [{"name": "CAPS", "type": "text", "label": "Extra tool groups", "default": "network,storage"}]},
  "mcp": [
    {
      "name": "playwright",
      "description": "Playwright MCP attached to the agent's Chrome",
      "command": "chrome-mcp",
      "args": ["mcp"],
      "env": {"CHROME_MCP_CAPS": "${CAPS}"}
    }
  ]
}
```

The script installs the program the command names; the manager runs it as the desktop user only when an endpoint needs its tools, never on a port. **Install** shows the declarations under the script, where the user can leave one out before building. To say on the card which servers a template provides, list their names in its `index.json` entry, as `"mcp": ["playwright"]`.

A template can let the user choose part of what it installs with `options`, which **Install** lists as checkboxes. Each has a `name`, the shell variable its script reads: an uppercase letter followed by up to 31 uppercase letters, digits or underscores. It has a `label` of one line and at most 100 characters, an optional `description` of at most 500, its `default`, `true` or `false`, and optionally in `mcp` the names of the template's own servers that are left out of the image while it is off. A template has at most 16, each name once, and only beside a `script`, never `parts`:

```json
{
  "name": "Reverse engineering",
  "base_image": "ghcr.io/co3moz/thoric/ubuntu-xfce:24.04",
  "script": "#!/usr/bin/env bash\nset -euo pipefail\n: \"${RADARE2:=true}\"\n: \"${WINE:=false}\"\nif [ \"$RADARE2\" = true ]; then\n  apt-get install -y radare2\nfi\n...",
  "mcp": [{"name": "radare2", "command": "r2mcp"}],
  "options": [
    {"name": "RADARE2", "label": "radare2 and r2mcp", "description": "Command line reverse engineering with an MCP server.", "default": true, "mcp": ["radare2"]},
    {"name": "WINE", "label": "Wine", "description": "Runs Windows programs to watch them.", "default": false}
  ]
}
```

Let a template's scripts say where they are, so the build's panel shows its steps and which were left out. Print `THORIC_STAGE=<i>/<n> <label>` on a line of its own as the i-th of n steps begins, and `THORIC_SKIP=<i>/<n> <label>` for the step of an option that is off, keeping the numbers the same whichever options are chosen:

```bash
echo "THORIC_STAGE=1/3 Installing radare2"
if [ "$WINE" = true ]; then
  echo "THORIC_STAGE=2/3 Installing Wine"
  apt-get install -y wine
else
  echo "THORIC_SKIP=2/3 Wine"
fi
echo "THORIC_STAGE=3/3 Cleaning up"
```

Count each script's steps on its own: in a merged build every part, and every compose step, is followed apart, so a template's numbers never depend on what it is merged with. The compose's setup script can print them too. A script without them is followed by the line it reached, which says less, since one long `apt-get install` line is most of the time; [the API guide](API.md#script-progress) has the details.

The dashboard resolves options in the browser and never sends them. When the image builds, it writes the values at the top of the script, after its `#!` line, as `RADARE2=true` and `WINE=false`, and leaves out the servers of the options that are off. So the script sets its own defaults with `: "${NAME:=default}"`, each the same as its option's `default`: a release from before options, an agent calling `build_image` and a pasted configuration send the script as it is, and it then installs the default set. Give an option a name of the template's own, such as `RADARE2`, never a variable the script or its tools read, such as `PATH` or `HOME`. The image's record keeps the script with the values in it, so its **Export configuration** builds the same image again and has no `options`. In a merged build each template's options stay with its part, and its values go into its own script.

```bash
git clone https://github.com/co3moz/thoric.wiki.git
cd thoric.wiki
mkdir -p templates
# Add templates/<id>.json and templates/<id>.png, and the entry in templates/index.json.
git add templates
git commit -m "Add the Unity template"
git push
```

The wiki has no review or checks before a push reaches every installation, which reads the list within about five minutes. Open **Templates** after pushing: an entry the page cannot offer is listed under the cards with the reason, and **Install** reports a template file that does not fill the build form. Every installation's browser fetches these files, and a template's script runs as root in the build, so treat a change to the wiki as a release. The first page of the wiki has to be created on GitHub before the repository can be cloned.

**Merge and install** builds a template together with others, so write every template to work beside them. A merged build runs each template's build script in a layer of its own, in the order the user chose, and each compose as a step whose script receives only its own parameters; [the API guide](API.md#merged-templates) has the details. That leaves the files the scripts share:

- Start from `apt-get update` and end with `rm -rf /var/lib/apt/lists/*`, since another template may have cleaned up before yours runs.
- Change shared files by adding to them, and check before adding, so a second run or another template's copy of the same software finds them as it expects. Never remove or replace what another template may have put there.
- Give a panel launcher its own plugin id: the `vscode` template takes `plugin-23`, so a new one takes the next free number rather than reusing one a template already checks for.
- Keep the template's compose volume paths its own, such as `/home/desktop/workspace` only for the template that owns that workspace: two steps cannot mount volumes at one path.
- Name the template's base image as the others do. A merge builds on the first template's base and warns about any other.
- Give the template's MCP servers names of their own, such as `playwright` rather than `mcp`: a merged image has one `mcp.d` folder, so a merge of two templates that declare a server under the same name is refused, and the build refuses a duplicate name too.

A template exported from a merged image has `parts` and `template_ids`, and can be listed like any other; choosing it together with one of the templates it holds is refused.

## Add an HTTP route

Register the route in `router()` in `core/manager/src/main.rs`, then give it an entry in `core/manager/src/openapi/routes.rs`, which `GET /api/openapi.json` is made from: its method, its path as `main.rs` spells it, a tag, a one-line summary and what it takes and answers. `.json::<T>()` and `.optional_json::<T>()` name the type a JSON body is read into, `.query::<T>()` that of a query string, and `.upload(...)` a body read as a stream; `.returns::<T>()`, `.job()`, `.no_content()`, `.download(...)`, `.events()` and `.websocket()` say what comes back, and `.access(...)` what opens a route outside the session. A route that takes any method is an `any(...)` entry naming the methods the document shows.

A type a route reads or returns derives `schemars::JsonSchema` beside its serde traits, and so does every type in its fields; its doc comments become the schema's descriptions. A type whose serde is written by hand implements `JsonSchema` by hand next to that code, as `Compose`, `Icon` and `Servers` do, and a type whose name another one already has gets `#[schemars(rename = "...")]`.

`cargo test` holds the table to the router both ways. `every_served_route_is_documented_and_every_documented_route_is_served` in `core/manager/src/openapi_tests.rs` reads the paths from the router and asks each with a method nobody serves, so axum names the methods it has, and it lists every route without an entry, every entry for a route that is gone and every method on one side only. List the route in the table of `docs/API.md` as well, and describe its answer there.

## Check a change

For the frontend, with Bun 1.4 or later, in `core/web/`:

```bash
cd core/web
bun install --frozen-lockfile
bun run lint
bun run test
bun run build
```

`lint` runs ESLint and checks Prettier formatting (`bun run format` fixes it), `test` type-checks and runs the unit tests in `core/web/tests/*.test.ts` with Bun's test runner, and `build` type-checks the application and writes `core/web/dist/`.

For the backend, with [rustup](https://rustup.rs), which installs the Rust release `rust-toolchain.toml` names the first time cargo runs in the repository, the one the checks use (add `rust-src` to it with `rustup component add rust-src` for an editor's language server):

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Check the Python code, the bundled stdio bridge, the manager's helpers and dependency list generator, the desktop tray, and the WSL package's command and first start, with Python's standard library test runner. `tests/unit.py` runs the `tests/` folder at the root and the one of every project:

```bash
python3 tests/unit.py
```

`tests/test_static.py` reads what no other test does, since the unit tests import only the helpers they test and the acceptance suite runs at the end. It compiles every Python file git tracks, with the compiler's warnings, such as an invalid escape or an assert on a tuple, as errors; where git cannot read the checkout, such as a Windows worktree from WSL, it walks the folders outside the generated ones instead. It also refuses a name `tests/integration.py` or `tests/cleanup_acceptance.py` binds again, in any scope, after the script's top level gave it to a function, a class or a module: the suite is one flat script, and a phase that assigned `refused` once hid the suite's `refused()` from every phase after it. Give the variable another name, not the helper.

Check the shell scripts, those of the WSL package and the desktop image and the manager's script trace, with [ShellCheck](https://www.shellcheck.net). The script finds them by their name or their first line and runs the ShellCheck release it names in Docker, as the checks do:

```bash
bash tests/shellcheck.sh
```

A finding that is wrong for the code is turned off on the line it fires on, with the reason beside it, such as `# shellcheck disable=SC2086 # one argument per process ID`, never for a whole file. A script that bash reads without running it names its shell in a `# shellcheck shell=bash` line.

Validate both installation manifests:

```bash
docker compose --env-file .env.example --profile images config --quiet
docker compose -f compose.ghcr.yml --env-file .env.example --profile https config --quiet
```

Look for known advisories in the locked crates and interface packages, as the `audit` job does for a pull request, with [cargo-audit](https://crates.io/crates/cargo-audit) (`cargo install cargo-audit --locked`) from the repository root and Bun from `core/web/`, whose `audit` script carries the exceptions listed under [Keep dependencies current](#keep-dependencies-current):

```bash
cargo audit
(cd core/web && bun run audit)
```

For Docker or GPU lifecycle changes, build both catalog images and the manager, then run the acceptance suite inside WSL:

```bash
python3 tests/integration.py
```

The suite creates uniquely named resources and cleans only resources it owns. It covers both catalog images, hardware OpenGL, authentication, VNC and screenshot output, audio channels, uploads, image builds, snapshots, volumes, network changes, concurrent operations and recovery, plus the agent tools: background applications, windows, held input, relative pointer movement, waits, diagnostics, recordings and their H.264 video, artifact export, file tools, downloads, audio capture and injection, terminals, accessibility inspection, the virtual gamepad and container reset. It does not launch Unity or test a physical microphone.

### GPU on a Linux host

The suite asserts the renderer of the GPU routes the manager found, so it also reads right on a Linux host, but it has been run in WSL only. Whoever first has a Linux host with an NVIDIA, AMD or Intel GPU, check there, with the images built from the branch:

1. The manager's log names the routes GPU containers get, and `GET /api/settings` reports them in `gpu`: `["nvidia"]` with the NVIDIA Container Toolkit registered in Docker, `["dri"]` with an AMD or Intel GPU, and `WSL_GPU` among `unused` as `no_wsl_gpu`.
2. A GPU container starts, and its **Diagnostics** show the hardware renderer under `GPU / VirtualGL EGL`, not `llvmpipe`, with the devices it got and `eglinfo -e`.
3. For NVIDIA, both ways Docker hands over `--gpus`: CDI devices (Docker 29.2 and later with `nvidia-ctk cdi generate` or the toolkit's own spec), and the toolkit's hook, where the `graphics` capability adds `libEGL_nvidia`.
4. For AMD or Intel, the desktop user's groups include those of `/dev/dri`, and `GPU_ADAPTER=AMD` or `Intel` picks the card on a host with two.
5. With `NVIDIA_COMPUTE=false DRI_GPU=false`, a request for a GPU container fails with a message that names `WSL_GPU`, and new containers start without GPU access.

Then `python3 tests/integration.py` on that host, and record what ran, with the GPU and the Docker and toolkit versions, in the pull request and in [Running graphics applications](USAGE.md#running-graphics-applications).

Inspect interface changes in a live browser at desktop and narrow widths. Open saved screenshots to confirm their actual layout before adding them to the documentation. Do not run acceptance tests or rebuild images for a wording-only change.

`docs/screenshots/agent-demo.gif`, at the top of the README, is a recording of an agent at work, not a screenshot of the interface. It was made in a real installation with a container from the XFCE catalog image at 1280x720, driven only through `/mcp` (`desktop_input`, `wait_for_condition`, `set_window_geometry`, `screenshot`, `save_screenshot`) while `start_recording` kept frames at 5 per second, which `read_file` then read back. The frames are scaled to 960 pixels wide and share one palette, a bar under them names the tool call in progress, and a ring marks the click, since screenshots carry no pointer. Keep it under 5 MB, open it after saving, and record it again when the desktop or the tools it shows change.

## Keep dependencies current

Every version Thoric builds with moves only with a commit. The crates and interface packages are locked in `Cargo.lock` and `core/web/bun.lock`, the workflow's actions are pinned to commits, `rust-toolchain.toml` names the Rust release, and every image a Dockerfile or a Compose manifest starts from names an exact release, such as `caddy:2.11.7-alpine`, or a dated build, such as `ubuntu:noble-20261024.1`, Ubuntu 24.04 as published that day. `tests/test_pinned_versions.py` refuses a moving tag, and holds the `rust` image of `core/manager/Dockerfile` to `rust-toolchain.toml` and its `oven/bun` image to the `bun-version` of the workflow, so the manager image builds with the toolchains the checks ran.

Dependabot (`.github/dependabot.yml`) proposes newer versions once a month, grouped, because each pull request runs the checks and costs CI minutes:

| Pull request | What it raises |
| --- | --- |
| `actions` | The workflow's actions, at most one pull request open |
| `crates`, `crates-major` | The crates in `Cargo.lock`, the minor and patch releases apart from the major ones |
| `packages`, `packages-major` | The interface packages in `core/web/bun.lock`, the same way |
| `infrastructure` | The images of the Dockerfiles and the Compose manifests, and the release in `rust-toolchain.toml`, together |

Each image stays on its release line: Ubuntu and Debian by the codename of their dated builds, the `docker`, `oven/bun`, `rust` and `caddy` images by ignoring their next major release, which is a change of its own. Dependabot leaves Thoric's own images alone, and `@remixicon/react`, which stays at 4.8.0 for its license. It skips TypeScript 7.0, which ships no compiler API, so typescript-eslint refuses it; its support waits for TypeScript 7.1 ([typescript-eslint#10940](https://github.com/typescript-eslint/typescript-eslint/issues/10940)), and Dependabot proposes that release on its own. The checks build no image, so before merging the `infrastructure` pull request build the images it changes (`docker compose --profile images build desktop desktop-gnome manager`, and `packaging/wsl/build.sh` for the WSL package's base) and run the acceptance suite. When it raises `oven/bun`, raise every `bun-version` of `.github/workflows/publish.yml` on its branch too; when it raises Rust, fix what the new clippy finds there. Raising Rust by hand changes the same two lines in one commit, the `channel` of `rust-toolchain.toml` and the `rust:<release>-bookworm` image of `core/manager/Dockerfile`, once that image is published, and fixes the new lints with them.

Pull requests also run the `audit` job, beside the checks: `cargo audit` against the [RustSec](https://rustsec.org) advisories and `bun run audit`, Bun's `bun audit` with the exceptions below, against the npm registry's. A crate fails it with a vulnerability or an unsound advisory, and a package with any advisory; an unmaintained or yanked crate is a warning in the log. Fix one by updating the dependency, or the one that pulls it in. When no release fixes it yet and it cannot reach Thoric, ignore it with its reason and the date it was looked at: a crate's advisory in `.cargo/audit.toml`, a package's as `--ignore=<GHSA id>` in the `audit` script of `core/web/package.json`, with the reason in this list, since JSON holds no comments:

- `GHSA-p98j-92pf-mc4p`, `GHSA-6688-9rhm-gjv2`, `GHSA-mrv2-3pqr-j5jr`, `GHSA-cg2g-xh64-v4q7` and `GHSA-c6gr-qvv4-5v3f`, DOMPurify up to 3.4.16, looked at on 2026-10-29. `monaco-editor` 0.57.0, its newest release, bundles a copy of DOMPurify 3.4.15 of its own (`esm/vs/base/browser/dompurify/`) and never imports the `dompurify` package the lockfile lists for it, so raising that package would change nothing the dashboard ships. Its sanitizer hands DOMPurify strings and sets none of `IN_PLACE`, `SAFE_FOR_TEMPLATES` and the XHTML parser these advisories need. Drop them with the `monaco-editor` release that bundles DOMPurify 3.4.17 or later.
- `GHSA-g3hv-75wg-m249`, minimatch before 11.0.1, looked at on 2026-10-31. Only the lint tools pull it in: `eslint` 10.14.0, `@eslint/config-array` 0.23.5 and `@typescript-eslint/typescript-estree` 8.72.1, the newest releases of each, all ask for minimatch 10. It is not in the dashboard's bundle, and the glob patterns it gets come from the repository's own ESLint and TypeScript configuration, while the advisory needs a pattern an attacker supplies. Drop it once those packages take minimatch 11.0.1 or later.

The job runs for pull requests only, so a release runs both audits again on the day it is cut.

## Publish container images

The **Publish containers** workflow checks frontend and backend code, makes the dependency list and validates Compose before publishing three images, each for `linux/amd64` and `linux/arm64`, to GHCR and Docker Hub:

| GHCR | Docker Hub | Moving tag from `master` | Moving tags of the newest release |
| --- | --- | --- | --- |
| `ghcr.io/co3moz/thoric` | `co3moz/thoric` | `edge` | `latest` |
| `ghcr.io/co3moz/thoric/ubuntu-xfce` | `co3moz/thoric-ubuntu-xfce` | `edge` | `latest`, `24.04` |
| `ghcr.io/co3moz/thoric/ubuntu-gnome` | `co3moz/thoric-ubuntu-gnome` | `edge` | `latest`, `24.04` |

Both registries get the same image under the same tags, an index of the two platforms, from which Docker downloads the host's own. Docker Hub has no nested repositories, so a desktop image's name there starts with the repository's. The image catalog, the Compose manifests and the WSL package keep naming GHCR; Docker Hub is a second place to pull the same images from. Every publication also gets a `sha-<full-commit>` tag. A release tag such as `v1.2.3` publishes `1.2.3` and `1.2` for all three images, and moves `latest`, and the desktop images' `24.04`, only when no higher `v*` tag exists, so publishing an older tag again leaves them where they are; the `checks` job finds that with `git ls-remote`.

Each published manager downloads the desktop images published in the same run and tags them as their catalog references, so it never installs a desktop image made after it. The workflow builds it with `THORIC_RELEASE` set to their tag: `1.2.3` for a release, `sha-<full-commit>` for a build of `master`. The manager carries the value in its `thoric.release` label, which `thoric update` reads to download the same images, and an edge build shows its commit in the sidebar (`GET /api/update` gives it as `commit`). Only a build without the argument, such as one from the source, downloads the catalog references themselves. The manager's tags wait for the desktop images' tags, so none of them names desktop images that are not published yet; since `THORIC_RELEASE` changes with every commit, each build of `master` compiles the manager again. Publishing is allowed from `master` or `v*` tags; manual runs follow the same restriction. A pull request to `master` runs only the checks and the audit, and a newer commit on it cancels the run of the older one. A change to the documentation alone builds no image, but a pull request that changes `docs/` or the README runs the checks, since the Rust and interface tests hold `docs/API.md`, `docs/MCP.md` and the README to the routes and tools they describe. `tests/test_publish_triggers.py` holds the pull request's paths to every path a push publishes from and to every file of the repository that a string in the crate, the interface tests or the Python tests names, from the root or, when the string starts with `./` or `../`, from the file that holds it. A path a test joins from parts is not seen.

Each platform builds on a GitHub-hosted runner of its own architecture, `ubuntu-24.04` and `ubuntu-24.04-arm`, since an emulated build of the manager or a desktop takes hours. The `build-manager` and `build-desktops` jobs push each platform's image by digest alone, without a tag, and keep the digests as artifacts of the run. Then `publish-desktops`, and after it `publish`, give the tags to one index of both platforms with `docker buildx imagetools create` and check that both registries hold it with an image of each platform, so no tag ever names one platform without the other. Build caches are kept per image and platform. The desktop image downloads the VirtualGL package of the architecture it builds for, and each manager image lists the interface packages and crates of its own platform, which `--platform` of `core/manager/dependencies.py` names and the list's `platform` field records. The `checks` job runs on x86_64 only: the arm64 builds are the first to compile the manager for arm64, and no test runs there.

The workflow publishes to GHCR with `GITHUB_TOKEN` and its package write access, and to Docker Hub as the `co3moz` account (`DOCKER_HUB_USER` in the workflow) with the `DOCKER_HUB_TOKEN` repository secret, a Docker Hub personal access token with read and write access. Without that secret the publishing jobs stop at their Docker Hub sign-in, before they push to either registry. It uses BuildKit caches in the GitHub Actions cache and actions pinned to commit SHAs, and reads each published manifest back by digest from both registries to check access. The two desktop images of a platform build one after the other on the same builder, so they publish the same layers for the `common` stage of `images/desktop/Dockerfile` and a host downloads and stores that part once; the job fails when those layers differ. The `checks` job keeps its compiled Rust dependencies with `Swatinem/rust-cache`, saved only from `master` and read by tags and pull requests. Hosted CI has no GPU, so it verifies neither the WSL GPU nor a Linux host's.

## Build the WSL distribution package

`packaging/wsl/build.sh` builds `packaging/wsl/Dockerfile` (Ubuntu 24.04, Docker Engine, Compose, Buildx, the NVIDIA Container Toolkit and the installation files), exports the container filesystem and compresses it. The package is for the Docker host's platform unless `--platform` names `linux/amd64` or `linux/arm64`, and goes to `dist/thoric.wsl`, or for Windows on ARM to `dist/thoric-arm64.wsl`. On an x86_64 host the arm64 package builds under QEMU, registered as [Build and run from source](#build-and-run-from-source) shows. Test a package with `wsl --install --from-file dist\thoric.wsl --name Thoric-test`, run the first-start setup, then `wsl --unregister Thoric-test`; the arm64 package installs only on Windows on ARM. On a release tag the `wsl-package` jobs build each package on a runner of its architecture, and `wsl-release` attaches both, with `packaging/windows/thoric.ps1`, the Windows helper the README installs with, which downloads the one of Windows' architecture, and `packaging/unix/thoric.sh`, the Linux and macOS helper, to the GitHub release. The names are those `build.sh` gives without an output, and `packaging/windows/tests/test_windows_launcher.py` holds the workflow and the helper to them. Keep the files under `packaging/wsl/` in sync with `compose.ghcr.yml` and `.env.example`, which the Dockerfile copies.

The package pins `MANAGER_IMAGE` to the version in `core/manager/Cargo.toml`, or to `THORIC_VERSION` when it is set, so a package built before its tag is published cannot download its manager yet; set `THORIC_VERSION` to a published release to test a first start that downloads. The `checks` job refuses a `v*` tag that differs from the version in `core/manager/Cargo.toml`, and the package job waits until all three images of the tag are published.

The manager image bundles `packaging/wsl/thoric` under `/app/install`, and `thoric update` installs it from a newer release, so a change to the command reaches installed distributions with the next release. The command already installed decides from two lines of the new one, `CLI_VERSION=`, which the package build and the update fill in, and `NEEDS_PACKAGE=`, so both keep their form. `packaging/wsl/Dockerfile` records the level of the package's own files in `/usr/lib/wsl/package-level`. When the command starts relying on a package file that older packages lack or have in another form, raise that level and `NEEDS_PACKAGE` together; installations from older packages then keep their command until they install a newer package, so prefer a command that copes with the older files.

## Keep older installations working

The ownership labels (`thoric.*`), the `thoric-*` resource names, the `thoric_session` cookie and the `X-Thoric-Request` header are identifiers that existing containers, images, volumes and sign-ins depend on. Changing them requires a migration, not a naming edit.

A manager meets desktop images of other versions: containers, saved images and snapshots outlive updates, and an image can come back from an archive or a backup of a newer installation. Both final stages of `images/desktop/Dockerfile` label their image with `thoric.desktop-level`, which `DESKTOP_LEVEL` in `core/manager/src/catalog.rs` equals and every image built, committed or imported from it keeps; an image without the label is level 0. A manager refuses to create a container from, build on, record or import an image whose level is above `DESKTOP_LEVEL` or below `MIN_DESKTOP_LEVEL`, with a reason that says to update Thoric or to download the catalog image again. Prefer desktop changes an older manager copes with, and new manager code that copes with older images, such as the warning for a desktop from before `thoric.vnc=private`. When a desktop change would break an older manager, because a script, path, user or label it relies on goes away or changes its form, raise the label in both stages and `DESKTOP_LEVEL` together; tests on both sides hold them equal. Raise `MIN_DESKTOP_LEVEL` only when the manager stops handling the images from before such a change.

The manager's database outlives updates too. Its records are JSON by kind and id, and a field added with `#[serde(default)]` needs nothing more; anything that changes a table, or rewrites records an older manager would misread, is a numbered migration in `core/manager/src/db/migrations.rs`. The database keeps how many it has run in `PRAGMA user_version`, every start runs the missing ones in one transaction, and a manager refuses a database whose version is above the migrations it knows. A migration a release shipped never changes, since databases already ran it; the next change is the next migration, with a test that opens a database of the version before it. A record that does not read is left out of the lists and logged by kind and id, so one bad record does not stop the manager; a decision that must see every record, such as whether anything still uses an image or a volume, or a backup, takes `Db::list_complete` and stops instead. Job logs are in a table of their own, `job_log`, in the chunks they were written in: `App::append_log` writes only the new text, and the log a page or an agent reads comes from `Db::job`, which puts the chunks after the log an older manager left in the record and keeps the last 256 KiB as `Job::trim_log` does. Change a job record read with `Db::get`, never one from `Db::job`, which would save its log into the record. The dashboard's update copies the database before it starts ([Go back after an update](WSL_PACKAGE.md#go-back-after-an-update)).

Preserve user containers, images and volumes during development. Never use daemon-wide pruning as a test cleanup step. See [installation and recovery](INSTALLATION.md) for the data that must survive an upgrade.
