# Installation and maintenance

[Back to the README](../README.md)

The [WSL distribution package](WSL_PACKAGE.md) installs Ubuntu, Docker and the GPU runtime together on Windows; the [Linux and macOS helper](#the-linux-and-macos-helper) installs Thoric with Compose on Linux, a Raspberry Pi or a Mac, and the [README's Compose example](../README.md#custom-installation) sets it up by hand on a host you manage yourself, in WSL or not. This guide covers changes you may need after that: settings, access from another device, updates and recovery.

## The Linux and macOS helper

`thoric.sh` is published beside the Windows helper in every release and runs straight from there, in a terminal:

```bash
curl -fsSL https://github.com/co3moz/thoric/releases/latest/download/thoric.sh | bash
```

It needs a 64-bit system, x86_64 or ARM, and bash, which macOS has. It looks for Docker first. On Linux without Docker it offers Docker's own install script from get.docker.com, which installs Docker Engine and Compose on Debian, Raspberry Pi OS, Ubuntu, Fedora, CentOS and RHEL as root through sudo, and runs it only when you agree; on a Mac it names Docker Desktop, which you install yourself, and starts it when it is installed but not running. A user outside the `docker` group gets `sudo docker`, and the offer to join the group, which applies from the next sign-in; the group's members control Docker as root does.

Then it asks for:

- A folder for the installation, `~/thoric` unless you type another. It holds `compose.yml` (the release's `compose.ghcr.yml`), `.env` with the settings, `deploy/Caddyfile` for [HTTPS](#https-and-microphone-access-over-lan), and `.shipped.sha256`, the checksums of the files as installed. Images, containers and volumes stay in Docker. On a Mac, keep the folder in your home folder, which Docker Desktop shares with its virtual machine.
- A username and a password for the dashboard. An empty password opens the dashboard without signing in.
- Whether to enable root agent access at `/mcp`, and how agents authenticate: a random token by default, your own token or none.
- Whether only this computer or other devices too can reach the dashboard (`BIND_ADDRESS` 127.0.0.1 or 0.0.0.0), its port (8080 unless you type another, refused while something already answers there) and, for other devices, the names they open it by (`ALLOWED_HOSTS`). The host's firewall has to let them in.

It pins the manager to the newest release, `MANAGER_IMAGE=ghcr.io/co3moz/thoric:<version>`, as the WSL package does, downloads it and takes the installation files from it, writes `.env` readable by you alone and says what the host may lack: memory for several desktops, the memory controller on a [Raspberry Pi](#raspberry-pi) and the NVIDIA Container Toolkit for an [NVIDIA GPU](#gpu-on-a-linux-host). It leaves the GPU settings at their defaults, since the manager finds the host's GPU itself. Then it offers to start Thoric and ends with the agent endpoint and its client settings.

Run the same line again to manage the installation. The helper finds it by the manager's container, which Compose labels with its folder, or in `~/thoric`, and offers to start or stop Thoric, update it, show the agent settings or remove it. A command after `bash -s --` skips the menu, as in `curl -fsSL https://github.com/co3moz/thoric/releases/latest/download/thoric.sh | bash -s -- update`; the commands are `start`, `stop`, `status`, `update`, `mcp` and `remove`.

- **Stop** stops the running desktops and the manager. Start the desktops again from the dashboard once it runs.
- **Update** moves the manager to the newest release with the installation files of that release. A file you edited is kept as `<file>.bak` before the release's replaces it, and `.env` keeps every setting but `MANAGER_IMAGE`. When the new manager does not answer within three minutes, the helper puts `.env` and the files back and starts the previous manager again. A `MANAGER_IMAGE` you set by hand, such as `latest` or `edge`, is downloaded again as it is. Desktop images you have downloaded update from their row on the **Images** page, see [Update the images](#update-the-images). The update takes no copy of the database, so to be able to go back, make one first as [Go back to an earlier release](#go-back-to-an-earlier-release) shows.
- **Remove** removes the manager's container and, when you ask, the files the helper put in the folder. Desktop containers, images and volumes stay, and so does the manager's database in its volume, which a new installation takes up again. The helper names the commands that list them.

The helper manages only an installation it made. One set up by hand stays as it is, and in the WSL package the `thoric` command does all of this.

## Use the full installation files

If you have a source checkout, copy `.env.example` to `.env`, set `ADMIN_PASSWORD` and start the published images:

```bash
cp .env.example .env
docker compose -f compose.ghcr.yml up -d
```

For an installation without a checkout, the manager image includes the same files. Export them into your installation directory:

```bash
mkdir -p deploy
docker run --rm --entrypoint cat ghcr.io/co3moz/thoric:latest /app/install/compose.yml > compose.ghcr.yml
docker run --rm --entrypoint cat ghcr.io/co3moz/thoric:latest /app/install/.env.example > .env
docker run --rm --entrypoint cat ghcr.io/co3moz/thoric:latest /app/install/deploy/Caddyfile > deploy/Caddyfile
```

Edit the exported `.env` before starting the manager. Run these export commands in a new directory so you do not overwrite settings from an existing installation.

## Settings

The minimal README example keeps settings directly in Compose. The full manifests read them from `.env`. Recreate the manager with `docker compose up -d` after changing its environment; for the full registry manifest, use `docker compose -f compose.ghcr.yml up -d`.

The dashboard's **Management** page lists the settings below with the values the manager runs with, so you can see what a restart picked up. It never shows `ADMIN_PASSWORD`, `MCP_TOKEN`, `PROXY_PASSWORD` or `NOTIFY_URL`, only whether they are set. Of the Compose settings in the second table it shows `MANAGER_CONTAINER` and `DESKTOP_HOST`, since Compose keeps the others to itself and `RUST_LOG` goes to the logger.

In the [WSL package](WSL_PACKAGE.md#settings-from-the-dashboard) the page also changes `SECURE_COOKIE`, `ALLOWED_HOSTS`, `DESKTOP_IMAGE`, `DESKTOP_USER_NAMESPACES`, `DESKTOP_PIDS_LIMIT`, `WSL_GPU`, `NVIDIA_COMPUTE`, `GPU_ADAPTER`, `AUDIO_ENABLED`, `MICROPHONE_ENABLED`, the other container features' switches and defaults below, `UPLOAD_MAX_MB`, `IMAGE_ARCHIVE_MAX_MB`, `TEMPLATES_URL`, `VOLUME_FILTER`, `PROXY_URL`, `PROXY_USERNAME`, `PROXY_BYPASS`, `UPDATE_CHECK`, `NOTIFY_EVENTS` and `DASHBOARD_URL`: it checks the new values as the manager would read them when it starts, writes them to `.env` and recreates the manager. `.env` stays the only place they are kept. The others, sign-in, the root MCP and its token, `PROXY_PASSWORD`, `NOTIFY_URL`, `MANAGER_CONTAINER` and the Compose settings, change only in `.env`, with `thoric settings` in the package, and the page marks them **CLI only**. Elsewhere the page changes nothing.

| Setting | Default | What it changes |
| --- | --- | --- |
| `AUTH_MODE` | `password` | Use `none` to open the manager without a sign-in screen. Programs in the desktops do not reach it on their private network, which the manager refuses, but with `BIND_ADDRESS=0.0.0.0` a desktop with internet access reaches the published port through the host like any device on the network. Keep sign-in on then; the manager warns at startup and on the Management page when it is off. |
| `ADMIN_USERNAME` | `admin` | The single administrator's sign-in name. |
| `ADMIN_PASSWORD` | Empty | Set a password of at least eight characters when login is enabled. |
| `ROOT_MCP_ENABLED` | `false` | Enable the root agent endpoint at `/mcp`, which reaches every container. The [MCP endpoints](MCP.md#mcp-endpoints-for-one-agent) made on the dashboard answer with their own tokens whether or not it is on. Called `MCP_ENABLED` before 0.6.3: an `.env` that still has the old name keeps its value while `ROOT_MCP_ENABLED` is missing, the manager logs that it should be renamed, and refuses to start when the two names disagree. In the WSL package `thoric update` renames it. |
| `MCP_TOKEN` | Empty | The administrator's bearer token, required when `ROOT_MCP_ENABLED=true` in `token` mode; 1 to 256 visible ASCII characters without spaces. It opens every MCP endpoint too, also while `/mcp` is off. Wrap it in single quotes when it contains `$` or `#`. |
| `MCP_AUTH_MODE` | `token` | Use `none` to accept agent requests without a token, on `/mcp` and on every MCP endpoint, whether or not `/mcp` is on. Anyone who can reach the manager's address can then control every container that `/mcp` or an endpoint reaches, so keep it on localhost. |
| `WSL_GPU` | `true` | Gives GPU containers the WSL GPU (`/dev/dxg` and `/usr/lib/wsl`) where the host has it. See [GPU on a Linux host](#gpu-on-a-linux-host) for how the routes are chosen. |
| `NVIDIA_COMPUTE` | `true` | Gives GPU containers an NVIDIA GPU through the NVIDIA Container Toolkit: CUDA, and outside WSL OpenGL too. |
| `DRI_GPU` | `true` | Experimental. Gives GPU containers the render devices under `/dev/dri` of AMD and Intel GPUs, or of a board such as the Raspberry Pi, for Mesa's own drivers, on a host outside WSL. The Management page shows it but does not change it, since the WSL package never uses it. |
| `GPU_ADAPTER` | Empty | The GPU to render on: in WSL a Mesa D3D12 adapter by vendor or part of its model name, with `/dev/dri` the `AMD` or `Intel` card. Empty leaves the choice to Mesa. |
| `AUDIO_ENABLED` | `true` | Allows containers to use browser sound. |
| `MICROPHONE_ENABLED` | `true` | Allows microphone input when audio is also enabled. |
| `INTERNET_ENABLED` | `true` | Allows containers internet access. `false` refuses it for new containers, in setting changes and on the internet switch. |
| `SUDO_ENABLED` | `true` | Allows containers to give the desktop user `sudo`. `false` refuses it for new containers and in setting changes; a container that had it keeps what root changed meanwhile until a reset, as described below the table. |
| `GAMEPAD_ENABLED` | `true` | Allows the virtual gamepad. `false` refuses it for new containers and in setting changes, and the manager passes no gamepad input. |
| `VPN_ENABLED` | `true` | Allows VPN access, `NET_ADMIN` and `/dev/net/tun` for a VPN client inside a container. `false` refuses it for new containers and in setting changes. |
| `GPU_DEFAULT` | `true` | Whether a new container gets GPU access unless it asks otherwise, where the host has a GPU that `WSL_GPU`, `NVIDIA_COMPUTE` or `DRI_GPU` allows. |
| `INTERNET_DEFAULT` | `true` | Whether a new container gets internet access unless it asks otherwise. |
| `SUDO_DEFAULT` | `true` | Whether a new container gives the desktop user `sudo` unless it asks otherwise. |
| `AUDIO_DEFAULT` | `false` | Whether a new container gets sound unless it asks otherwise. |
| `MICROPHONE_DEFAULT` | `false` | Whether a new container with sound gets the microphone unless it asks otherwise. |
| `GAMEPAD_DEFAULT` | `false` | Whether a new container gets the virtual gamepad unless it asks otherwise. |
| `VPN_DEFAULT` | `false` | Whether a new container gets VPN access unless it asks otherwise. |
| `DESKTOP_USER_NAMESPACES` | `true` | Lets processes in a container create user namespaces, which Chrome, Edge, other Chromium-based browsers and Electron applications use for their sandbox. Containers get Docker's default seccomp profile with this one addition (`images/desktop/seccomp.json`); calls that need `CAP_SYS_ADMIN`, such as `mount`, stay blocked. Set `false` to keep Docker's default: those applications then start only with `--no-sandbox`, and opening a link in such a default browser fails. It applies when a container is created, so an existing container changes after a reset or a settings change that recreates it. |
| `DESKTOP_PIDS_LIMIT` | `16384` | How many processes and threads one container may run, so a runaway program or a fork bomb inside it stops there instead of using up the host's for the other containers and the manager. Threads count: a browser with many tabs, an IDE with its language servers or the Unity editor each run hundreds to thousands, so raise it when programs in a container fail with `Resource temporarily unavailable` as they start a process or a thread. At least 1024, or `0` for no limit. Docker changes it on a running container, so within 20 seconds of starting the manager gives every container the current value, also one created before the limit, without restarting it. |
| `UPLOAD_MAX_MB` | `1024` | Maximum file size in MiB, from 1 to 65536. |
| `IMAGE_ARCHIVE_MAX_MB` | `65536` | Largest image or volume archive an import accepts, in MiB after decompression, from 1 to 1048576. An image archive holds every layer of the image, so it is several gigabytes. |
| `DESKTOP_IMAGE` | XFCE GHCR image | The default image when creating a container. |
| `TEMPLATES_URL` | Empty | The template list the **Templates** page reads. Empty uses the list on the Thoric wiki, `https://raw.githubusercontent.com/wiki/co3moz/thoric/templates/index.json`. Set the `http` or `https` address of another `index.json` for your own list or a mirror, with each template's `<id>.json` and `<id>.png` beside it, or `none` to turn the page off. The browser reads the list, so it needs to reach that address; the manager does not. |
| `VOLUME_FILTER` | Empty | The Docker label a named volume needs to be listed, mounted, browsed, exported or copied, as `key` or `key=value`. Empty means `thoric.managed=true`, which Thoric puts on every volume it creates, imports or copies, so volumes made elsewhere on the same Docker Engine, such as the manager's own data volume or another Compose project's database, stay out of reach. A request that names such a volume is answered as if it did not exist. Thoric adds the label you set here to the volumes it creates. A container keeps a volume it mounted before the filter left it out, through resets and setting changes; only a new choice is held to the filter. Docker cannot add a label to an existing volume, so to bring one in, set `none` for a while and copy it from the **Volumes** page. `none` lists every volume. |
| `PROXY_URL` | Empty | The proxy that desktops' programs go through when their container follows the installation's, the default for a new container while it is set: `http://`, `https://` or `socks5://` with a host and a port, such as `http://proxy.example:3128`, without a user name or a path. Empty gives the installation none, and a container that follows it goes straight to every address. A container can have a proxy of its own, or none, instead; see [Proxy](USAGE.md#proxy). A change reaches running desktops once the manager has started again, without restarting them. |
| `PROXY_USERNAME` | Empty | The proxy's user name, when it asks for one. |
| `PROXY_PASSWORD` | Empty | The proxy's password. It is a secret: the **Management** page shows only whether it is set and cannot change it. Put a password that holds `$`, `#` or quotes in single quotes. |
| `PROXY_BYPASS` | Empty | Hosts that skip the proxy, separated by commas: a name with the names below it (`intranet.example`, `.intranet.example` or `*.intranet.example`), an address or a network such as `10.0.0.0/8`, at most 64. The desktop itself always does. |
| `UPDATE_CHECK` | `true` | Whether the manager asks GitHub for the newest release, when it starts, every six hours and when **Check now** on the **Management** page asks. The sidebar names a release newer than the manager and links to its notes; in the [WSL package](WSL_PACKAGE.md#update-from-the-dashboard) its **Update** button runs `thoric update`. `false` turns both off, and the manager makes no request to GitHub. |
| `NOTIFY_URL` | Empty | Where [notices](USAGE.md#get-notified-on-your-phone) go, such as an agent's request for a person while nobody has the dashboard open: one or several addresses separated by commas, at most eight. An ntfy topic (`https://ntfy.sh/<topic>`, or `ntfy+https://` for a server of your own), a Discord or Slack incoming webhook, a Telegram bot's `sendMessage` address with `?chat_id=`, or any other `http` or `https` address, which gets a JSON payload. The address holds its credentials, so it is a secret: the **Management** page shows only whether it is set and cannot change it, and the manager never logs it or puts it on a command line. Put an address that holds `$` in single quotes. |
| `NOTIFY_EVENTS` | Empty | What is sent, separated by commas: `human_request`, `job_failed` and `update_available`. Empty sends `human_request` alone; `none` sends nothing, while the **Management** page's test still reaches `NOTIFY_URL`. |
| `DASHBOARD_URL` | Empty | The address you reach the dashboard at, such as `https://thoric.lan:8443`, so a request for a person links to its desktop and a failed job to the **Jobs** page. Empty sends notices without a link to the dashboard. |
| `ALLOWED_HOSTS` | `localhost` in Compose | Extra DNS names accepted by the manager, separated by commas. `DESKTOP_HOST` is accepted as well; Compose passes it on its own. |
| `SECURE_COOKIE` | `false` | Require HTTPS for the sign-in cookie. |

The full manifests also accept these Compose settings:

| Setting | Default | What it changes |
| --- | --- | --- |
| `BIND_ADDRESS` | `127.0.0.1` | Which host interfaces expose the manager and optional HTTPS service. |
| `PORT` | `8080` | The manager's HTTP port on the host. |
| `MANAGER_CONTAINER` | `thoric-manager` | The manager container's name. |
| `MANAGER_DATA_VOLUME` | `thoric-data` | The volume holding the manager database and temporary uploads. |
| `MANAGER_IMAGE` | `ghcr.io/co3moz/thoric:latest` | The manager image in `compose.ghcr.yml`; accepts a version or digest. `latest` is the newest release and `edge` follows `master`, see [Edge builds](#edge-builds); the WSL package sets the release it belongs to. |
| `DESKTOP_HOST` | `localhost` | The hostname for the optional HTTPS service; also added to accepted hosts. |
| `HTTPS_PORT` | `8443` | The optional HTTPS service's port on the host. |
| `RUST_LOG` | `thoric=info,tower_http=info` | The manager's log filter, which the manifests pass to its logger: `thoric=debug` adds the reads repeated on a timer, see [If something does not start](#if-something-does-not-start). Empty keeps the default. |

Sessions last 12 hours. Signing out or reaching session expiry closes active desktop and media connections. Desktop containers continue running.

The container features of the create form have a switch and a default each. GPU access is on while the host has a GPU that `WSL_GPU`, `NVIDIA_COMPUTE` or `DRI_GPU` allows, which choose how a container gets it, so it has no switch of its own. The defaults are what the form starts with and what a request to `POST /api/containers` or the MCP `create_container` gets when it leaves a feature out; one the installation switched off is never a default, and the microphone is one only with sound. Before 0.6.3 such a request got no GPU access while the form chose it; `GPU_DEFAULT=true` gives requests what the form gives, so set it to `false` to keep the old answer. A switched-off feature is refused when a container is created or a setting change turns it on, and the form shows its chip turned off. A container that already has it keeps it in its settings, also when a reset or a settings change creates it again, as audio always worked: the manager only refuses what it carries itself while the feature is off, the audio and microphone channels and gamepad input. Turn internet or `sudo` off in such a container's settings to take it away. Turning `sudo` off removes only the desktop user's sudo rule, and the change keeps the container's files, so whatever root changed while it was on stays: a program given the setuid bit, another file in `/etc/sudoers.d`, a password for root, or a changed system file, such as one of the scripts that start the desktop as root. Only a reset, which creates the container again from its source image, clears that; a snapshot taken meanwhile carries it along. GPU access is the exception: while the host has no GPU for desktops, a reset, a retry or a settings change that creates a GPU container again is refused with the settings named, since its desktop would stop at the renderer check; turn GPU off in its settings to go on using it. Image builds keep their own internet setting, since templates install packages.

Containers created with the gamepad option receive the host's `/dev/uinput` device, which current WSL2 kernels provide; a root helper inside the desktop creates the virtual controller there.

Containers created with the VPN option receive the host's `/dev/net/tun` device, which current WSL2 kernels and most Linux hosts provide, and the `NET_ADMIN` capability. Both reach only the container's own network namespace: a VPN client there changes the desktop's routes, never the host's or another container's. A host without `/dev/net/tun` refuses such a container with the setting named.

A desktop's screen connection asks no password, so its VNC server listens only on the private network it shares with the manager. From Docker Engine 28.0 that network, and the network of every [container group](USAGE.md#container-groups), has no address on the Docker host either, so no program on the host reaches a desktop there; in WSL that matters most, since every distribution shares one network namespace. An older Docker Engine refuses such a network, so the manager makes it as before, with an address on the host from which any program on the host reaches every desktop's screen and keyboard, and logs a warning; update Docker. A desktop created by an earlier release keeps its network as long as something that runs is on it, since its VNC server listens on the address it had when its session started: the manager makes the network again, with the same subnet, when a reset or a retry creates the container again, and with Docker Engine 29 before a stopped desktop is started from the dashboard or by an agent, which keeps its MAC address. So stop such a desktop and start it again; **Restart** does not move it. A group's network moves when its first member joins it alone. A network that cannot be made in its old subnet, which the desktop's VNC server listens in, is not made in another: the start fails with the subnet named, and a reset gives the container a new one. A manager that does not run in its own container, `MANAGER_CONTAINER`, as in some development setups, reaches the desktops through the host's address, so the networks it makes keep one; it cannot reach a desktop whose network a manager in its container made without, and logs a warning for it.

Container logs rotate. Docker's default logging driver, `json-file`, keeps everything a container writes in one file that otherwise grows for as long as the container exists. While the Docker daemon logs through it, the manager creates every container with `max-size=10m` and `max-file=3`, so a desktop's log takes at most 30 MB, and **Logs** and the MCP `container_logs` still show its newest lines. The manager asks Docker for its driver once when it starts. Under `local`, which rotates by itself, and under a driver chosen for the daemon, such as `journald` or `syslog`, containers keep the daemon's own settings. Both Compose manifests give the manager and the HTTPS service the same rotation whatever the daemon's default; a Compose override file changes that. Docker fixes a container's log settings when it creates the container, so a container created by an older manager keeps a log without a size until a reset or a settings change recreates it, and the manager's own log rotates once Compose recreates it from the new manifest, as `thoric update` does.

Agent access uses a separate token rather than the dashboard session, unless `MCP_AUTH_MODE=none` turns it off. See the [MCP guide](MCP.md) to connect an agent, or the [HTTP API](API.md) to build another client.

### GPU on a Linux host

When it starts, the manager reads its host's `/sys`, which a container shares with the host, to see which GPUs desktops can have, and logs the routes GPU containers get. The Management page shows the same in its **GPU** section, and `GET /api/settings` reports it in `gpu`. The manager sees the host only while it talks to a local Docker daemon; with a remote `DOCKER_HOST`, `WSL_GPU` and `NVIDIA_COMPUTE` apply as written and `/dev/dri` is never passed. Each route its setting allows is used where the host has it:

- **WSL GPU** (`WSL_GPU`): a WSL2 distribution's `/dev/dxg`, on which Mesa's D3D12 driver renders. There `NVIDIA_COMPUTE` adds CUDA beside it.
- **NVIDIA on a Linux host** (`NVIDIA_COMPUTE`): an NVIDIA GPU that NVIDIA's own driver runs, while Docker lists the NVIDIA Container Toolkit, as its `nvidia` runtime or an `nvidia.com/gpu` CDI device. Install the driver and the [toolkit](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/latest/install-guide.html), then register it with `sudo nvidia-ctk runtime configure --runtime=docker` and restart Docker. GPU containers get the GPU with the toolkit's `graphics` capability, and the desktop image names NVIDIA's EGL library to glvnd, so OpenGL renders on NVIDIA's driver. Docker 29.2 and later hand `--gpus` to the toolkit's CDI devices where they exist, which carry the graphics libraries in any case.
- **AMD and Intel on a Linux host** (`DRI_GPU`, experimental): the render devices under `/dev/dri` of a GPU that NVIDIA's driver does not run. The desktop user joins groups with the devices' group IDs, and Mesa's own drivers render. `GPU_ADAPTER=AMD` or `Intel` picks that vendor's card on a host with both. Usage charts read `nvidia-smi`, so these GPUs have none. The same route serves a GPU that is part of its board rather than a PCI card, which the manager names by its kernel driver: the Raspberry Pi's V3D as `Broadcom`, Mali (`panfrost`, `panthor`, `lima`) as `Arm`, and Adreno (`msm`), Vivante (`etnaviv`) and Apple's (`asahi`).

A host without any of them gives desktops no GPU: new containers start without it, and a request for it is refused with each setting and what the host lacks. Only the WSL route has been verified on hardware, with an NVIDIA RTX 4080 in WSL2. The NVIDIA and `/dev/dri` routes on a Linux host follow the documented behavior of the toolkit, Mesa and VirtualGL but have not been run on such a host yet; the [development guide](DEVELOPMENT.md#gpu-on-a-linux-host) lists what to check there.

## Raspberry Pi and Mac

The manager and both desktop images are published for `linux/amd64` and `linux/arm64` under the same names, so on an ARM host Docker downloads the arm64 images by itself, and the [Linux and macOS helper](#the-linux-and-macos-helper) and the [README's Compose example](../README.md#custom-installation) work as they are. The manager logs the architecture of the images it runs when it starts, `Docker runs linux/arm64 images`. Windows on ARM has a WSL package of its own, `thoric-arm64.wsl`, see [Windows on ARM](WSL_PACKAGE.md#windows-on-arm). Releases up to 0.9.1 published `linux/amd64` alone, so an ARM host needs a later release or an `edge` build. The arm64 images have been checked under QEMU on an x86_64 host, where the arm64 manager created XFCE and GNOME desktops from them that came up ready, but not yet on a Raspberry Pi or a Mac.

What a desktop runs is the same on both architectures, Ubuntu 24.04 with its own packages, with two things to keep in mind:

- Software a template or a script downloads must exist for arm64. A template that installs a program built only for x86_64 fails to build on an ARM host, so each entry of the template list names the architectures its template builds on, and a template without them counts as x86_64 only. On an ARM host the **Templates** page offers only the templates that name `arm64` and folds the others away under **For other architectures**, without **Install** ([Install a template](USAGE.md#install-a-template)). Most of the wiki's templates download a vendor's x86_64 build, such as Google Chrome or an AppImage, and stay x86_64 only until their scripts pick the arm64 download. A script of your own says what it downloads.
- An image keeps the architecture it was built on. A saved image, a snapshot, an image archive and the images of a backup from an x86_64 installation do not run on an ARM host, or the other way round, unless the host runs other architectures under emulation, as Docker Desktop can, and then slowly.

### Raspberry Pi

Use a Raspberry Pi 4 or 5 with a 64-bit system, such as the 64-bit Raspberry Pi OS or Ubuntu Server; a 32-bit system cannot run the images. Every desktop takes memory of its own, so prefer a board with 8 GB, and XFCE, which needs less than GNOME. The [helper](#the-linux-and-macos-helper) offers to install Docker Engine and its Compose plugin; to install them yourself, use [Docker's Debian or Ubuntu repository](https://docs.docker.com/engine/install/) and add your user to the `docker` group.

- When `docker info` warns `No memory limit support`, the kernel's memory controller is off and Docker discards every container's memory limit. On Raspberry Pi OS, add `cgroup_enable=memory` to the single line of `/boot/firmware/cmdline.txt` and reboot.
- A GPU container renders on the Pi's V3D GPU through the `/dev/dri` route above, which is experimental and has not been run on a Pi yet. A program that needs more OpenGL than V3D offers falls back to software or does not start; turn GPU access off for such a container.
- The Pi 5's own kernel uses 16 KB memory pages. A program built for 4 KB pages can fail to start there; `kernel=kernel8.img` in `/boot/firmware/config.txt` starts the Pi's 4 KB kernel instead.

### Mac

On a Mac, Thoric runs in [Docker Desktop](https://docs.docker.com/desktop/setup/install/mac-install/), whose Linux virtual machine runs the manager and the desktops: arm64 images on Apple silicon, amd64 images on an Intel Mac. Install it, then run the [helper](#the-linux-and-macos-helper) or follow the README's Compose example; the Docker socket they mount is the virtual machine's. Desktops use the virtual machine's memory, CPUs and disk, which **Settings > Resources** sets, so give it enough for the desktops you run, such as 8 GB of memory or more. Docker Desktop passes no GPU into its virtual machine, so desktops render in software, and the GPU option is unavailable when creating one. The dashboard and the desktop screens open at [localhost:8080](http://localhost:8080) as on any host.

## Access from another device

For the minimal Compose example, change the port binding to `0.0.0.0:8080:8080`. For the full manifests, set `BIND_ADDRESS=0.0.0.0`. Open the manager using the host's LAN address.

The manager accepts localhost and private LAN IP addresses. If you use a DNS name, add it to `ALLOWED_HOSTS`. The host's firewall must also allow the connection, and under WSL so must the Windows firewall and the forwarding into WSL; Compose does not configure those host settings. The WSL package asks at first start whether other devices may connect, on which port and by which names, and [describes the Windows side](WSL_PACKAGE.md#access-from-other-devices).

Keep login enabled when other people can reach the manager, and when any desktop has internet access: such a desktop reaches the published address through the host as the LAN does, and with sign-in off, or `MCP_AUTH_MODE=none`, its programs control every container. The manager warns about that combination at startup and on the Management page. Its Docker socket connection gives it control of the host Engine. The current application has one administrator, with no separate accounts or per-user container permissions.

## HTTPS and microphone access over LAN

Use the full manifest for the optional Caddy HTTPS service. It creates a local certificate authority, which each client device needs to trust. This also lets a remote browser request microphone access.

Set these values in `.env`, using a hostname that points to your host (the Windows machine, under WSL):

```dotenv
BIND_ADDRESS=0.0.0.0
DESKTOP_HOST=desktop.lan
HTTPS_PORT=8443
SECURE_COOKIE=true
COMPOSE_PROFILES=https
```

`COMPOSE_PROFILES=https` has every Compose command of the installation include Caddy, so `up`, `stop` and `pull` start, stop and update it with the manager, and in the WSL package `thoric start`, `stop` and `update` do too. Without it, Caddy runs only while a command names `--profile https`. `DESKTOP_HOST` has to be a name every client device finds the host under, through your network's DNS or each device's hosts file; the manager accepts it without an entry in `ALLOWED_HOSTS`.

Start the manager and Caddy, then copy the CA certificate:

```bash
docker compose -f compose.ghcr.yml up -d
docker compose -f compose.ghcr.yml cp https:/data/caddy/pki/authorities/local/root.crt ./caddy-root.crt
```

Trust `caddy-root.crt` on every client device and open [desktop.lan:8443](https://desktop.lan:8443). On Windows, `Import-Certificate -FilePath .\caddy-root.crt -CertStoreLocation Cert:\CurrentUser\Root` in PowerShell trusts it for Edge and Chrome after Windows asks to confirm; Firefox keeps its own list, under **Certificates** in its settings. The WSL package's guide shows [how to bring the file to Windows](WSL_PACKAGE.md#https). Once secure cookies are enabled, sign in through HTTPS.

Every browser reaches the manager from Caddy's address, so the manager counts failed sign-ins by the address Caddy forwards in `X-Forwarded-For`. It believes that header only from the running `https` service of its own Compose project, whose addresses it reads from Docker every 20 seconds and logs when they change; from anyone else, such as a desktop or a LAN client that connects to the manager's port, the connection's own address counts. A proxy of your own in front of the manager is not trusted this way, so all sign-ins through it share one count of 20 failures a minute.

Source installations use the same HTTPS profile in `compose.yml`. Omit `-f compose.ghcr.yml` for that manifest, and in the WSL package, whose `/opt/thoric/compose.yml` is `compose.ghcr.yml` under the name Compose reads by itself.

## Update the images

For the minimal Compose installation:

```bash
docker compose pull
docker compose up -d
```

For the full registry installation, add `-f compose.ghcr.yml` to both commands.

The sidebar names a newer release when one is out, unless `UPDATE_CHECK=false`. Only the WSL package updates itself from there; an installation of the [Linux and macOS helper](#the-linux-and-macos-helper) updates with the helper's **Update**, and others with the commands above. `compose.ghcr.yml` mounts an `update` folder beside it into the manager for that, which stays empty elsewhere.

This updates the manager. Catalog images are not Compose services: the manager downloads one the first time a container is created from it. To bring a catalog image you already have to the one published with the updated manager, select the update button on its row under **Images**. A published manager downloads the catalog image published with it and tags it with the catalog reference (`ubuntu-xfce:24.04`): a release such as `ghcr.io/co3moz/thoric:1.2.0` downloads `ubuntu-xfce:1.2.0`, and an `edge` build of `master` the `ubuntu-xfce:sha-<commit>` of its own commit. A manager therefore never installs a desktop image published after it, however far `master` has gone since. Only a manager built from the source downloads the reference itself, which, like `docker pull` of the reference, gives the newest release. Docker would keep the image the reference named before without a name, 1 to 2 GB for each release, so the download removes it, without force, once nothing needs it: no other tag names it, no container record starts from it, since a reset creates the container from it again, and no saved image's record names it. Docker itself keeps it while a container, or on its classic image store an image built on it, uses it. The job's log says which. Nothing is pruned. In the [WSL package](WSL_PACKAGE.md#update), `thoric update` does both and moves the manager to the newest release, together with that release's `compose.yml` and `thoric` command.

Compose files from earlier releases list two short-lived `desktop` and `desktop-gnome` services that download both catalog images at every start. They keep working. To download them only when they are used, replace the file with the current `compose.ghcr.yml` (or remove those services and the manager's `depends_on`), then run `docker compose up -d --remove-orphans` to remove their exited containers. Images already downloaded stay in place.

New containers use the updated catalog images. Existing containers and snapshots keep their saved image IDs; updating the manager does not reinstall their applications or replace their files. Keep the manager data volume when updating. To stay on a particular build, pin image references to a published version, `sha-<full-commit>` tag or digest.

### Edge builds

`edge` is the newest build of `master`, published with every change before a release includes it. To follow it, set `MANAGER_IMAGE=ghcr.io/co3moz/thoric:edge` in `.env`, export the installation files from `ghcr.io/co3moz/thoric:edge` as [above](#use-the-full-installation-files), since a change on `master` may add a setting to them, and run the update commands. An edge manager downloads the desktop images published with it, those of its own commit, and the sidebar names that commit instead of a version, since an edge build still carries the version of the release before it. `ghcr.io/co3moz/thoric:sha-<full-commit>` stays on one build of `master` the same way. For the WSL package, see [Edge builds](WSL_PACKAGE.md#edge-builds) there.

To return to releases, wait for a release newer than the edge build you run, then set `MANAGER_IMAGE` to it, or to `latest`, and update. An older release is a step back that its version does not show: it refuses a database the newer manager moved to a later layout, as [below](#go-back-to-an-earlier-release), and it refuses the desktop images, and the images built on them, made for a newer Thoric.

### Go back to an earlier release

The database records the version of its layout. Each start brings it to the newest layout the manager knows, in one step that either completes or leaves it as it was, and a manager refuses a database a later release has moved further, rather than misread it: it does not start, and its log (`docker compose logs manager`) names both versions. Releases up to 0.8.4 know no layout version: they open any database, and may misread what a later release wrote, such as the jobs it ran, which they show without their logs.

To go back, put a copy of the database from before the update in its place and start the earlier release. The WSL package's **Update** on the dashboard writes that copy itself, see [Go back after an update](WSL_PACKAGE.md#go-back-after-an-update). A Compose update takes none, so make one before you pull, with the manager stopped so the database and its journal files are whole. Replace `1.2.0` with the release you run and `thoric-data` with your `MANAGER_DATA_VOLUME`:

```bash
docker compose stop manager
docker run --rm -v thoric-data:/data --entrypoint sh ghcr.io/co3moz/thoric:1.2.0 -c \
  'mkdir -p /data/before-update && cp -p /data/state.sqlite* /data/before-update/'
docker compose pull
docker compose up -d
```

To put it back, stop the manager, set the newer database aside in `replaced`, copy the files back, set `MANAGER_IMAGE` in `.env` to the earlier release and start it:

```bash
docker compose stop manager
docker run --rm -v thoric-data:/data --entrypoint sh ghcr.io/co3moz/thoric:1.2.0 -c \
  'cd /data && mkdir -p replaced && mv state.sqlite* replaced/ && cp -p before-update/state.sqlite* .'
docker compose up -d
```

The records then describe the installation as it was before the update: a container created or deleted since is missing from them or listed again, while its Docker container, images and volumes stay as they are.

## Know where your data lives

| Data | Location | Effect of deleting a container |
| --- | --- | --- |
| Container settings, image records and jobs | `/data/state.sqlite` in the manager data volume | The manager's database remains. |
| Copies of the database from the WSL package's last three updates | `/data/state-<release>-to-<release>.sqlite` beside it | Unaffected. |
| Installed software and unmounted home files | The desktop container's writable layer | Lost unless saved in an image or snapshot. |
| Files carried over by a settings change | An image tagged `thoric/container:JOB_ID` that the container runs | Removed, unless a snapshot builds on it. |
| Files in named-volume mounts | Separate Docker volumes | Preserved. |
| Saved image and snapshot layers | The host Docker Engine's image store | Preserved. |

A snapshot alone is not a backup of the whole installation: it excludes volumes and the manager database. A backup from **Management** is one. It carries the manager's records and, as you choose, the saved images, each container's files and the named volumes; [moving to another computer](#move-to-another-computer) describes it. The settings in `.env` and the catalog images stay out of it.

Uploads are temporarily staged in the manager data volume. The manager accepts two transfers at once, each with a one-hour limit. Manager-side temporary upload files are removed after completion or cancellation, and leftover staging files are cleaned on startup. Make room for staging as well as the destination files.

## When setup is interrupted

A manager restart marks unfinished jobs as interrupted. Build scripts are not replayed automatically. Inspect the job result before starting an operation again.

Use **Retry setup** for a container that failed during creation or startup. If its container still exists, Thoric keeps the writable layer and completes missing networks or startup steps. If the container is missing, it can recreate it from the saved source image ID.

A settings change that needs a new container saves its files to an image and renames the old one to `thoric-ID-previous` until its replacement has started. If the manager stops during that job, the old container and its files are still there under that name, and the container record names the saved image. **Retry setup** then creates the container from that image with the new settings and removes the one set aside, as does the next settings change; a reset or a delete removes it with the container. Only a record written by a manager before 0.5.0 lacks the saved image, and then Retry setup and settings changes refuse to run while `thoric-ID-previous` exists: rename it back with `docker rename thoric-ID-previous thoric-ID`, or remove it once its files are no longer needed.

Recreating a missing container cannot restore changes that were never saved as a snapshot. Missing images or volumes stop recovery so you can resolve them; Thoric does not silently replace a missing volume with an empty one.

The manager checks container network connections, and gives each container `DESKTOP_PIDS_LIMIT`, every 20 seconds. A problem with one container does not prevent checks for the others. A check that fails is logged as a warning once, again only when its reason changes, and once more when it works again. Changes made directly through Docker can still conflict with manager operations, so inspect affected resources before trying recovery.

## If something does not start

If the manager is not reachable, check its startup log:

```bash
docker compose logs --tail=100 manager
```

Use `-f compose.ghcr.yml` with the full registry manifest. A missing or too-short password prevents startup when `AUTH_MODE=password`. A registry error happens before startup; check that the host can reach GHCR.

The log has a line for every request with its method, path, status, latency in milliseconds and `request_id`, the ID the response carries in its `X-Request-Id` header. To find the request behind an error the dashboard or a script saw, read that header in the browser's network panel or the script's response and search the log for it, for example `docker compose logs manager | grep 6f1c0b2e`. The line never holds a query string, a transfer link's token or the path inside a desktop service reached through the manager. Reads that a page or a health check repeats on a timer while they succeed (`/healthz`, `/readyz`, the usage charts, a followed job, the Activity page, the desktop previews and the interface's own files) are logged at the debug level only, so the log keeps what people and agents do. Failed sign-ins and the first refused attempt of a locked-out address are logged as warnings with the client's address. The log filter is `RUST_LOG`, which the Compose manifests pass from `.env` to the logger: the default is `thoric=info,tower_http=info`, and `thoric=debug` adds the repeated reads.

The manager stops on SIGTERM, which `docker stop`, `thoric update` and a settings change send it, as it does on Ctrl+C: it takes no new connection, ends the dashboards' event streams and every agent's wait for a person with an error that says so, and gives the open requests five seconds to finish before it ends, well within the ten seconds Docker waits. A job that was still running is marked interrupted when it starts again.

`/healthz` answers as long as the manager serves requests, which the manager image's health check uses. `/readyz` also asks the database and Docker and answers `503` naming the failed check, so `curl -fsS http://127.0.0.1:8080/readyz` tells a manager that runs but cannot reach Docker apart from one that is ready.

If the dashboard opens but a container fails, read its job log and **Logs**; **Diagnostics** needs a running desktop. For a GPU error, the job log names the setting behind a device Docker refused, and a desktop that stops at its renderer check names the one to check in **Logs**; [GPU on a Linux host](#gpu-on-a-linux-host) explains each route. You can create a separate container with GPU disabled to check the basic desktop path without deleting the failed container's files.

If file uploads report a permission error, check that the chosen volume is writable by UID/GID 1001:1001. Raising the upload size limit will not fix destination permissions.

## Move to another computer

A backup moves an installation, A, to another computer or a fresh installation, B. What it carries and how it streams are described in [the API guide](API.md#backup-and-restore).

1. On A, open **Management** and, under **Backup and restore**, tick **Saved images**, **Containers' files** and **Volumes** to move everything. **Secret values** adds the values of the composes' secret parameters; without them, a reset on B runs those setups without the values. Select **Download backup**. Containers may keep running; Docker pauses each desktop briefly while it saves its files.
2. Copy the file to B. It is not encrypted. It holds your files, the hashes of the MCP endpoints' tokens and, when ticked, secret values in plain text, so keep it where only you can read it, encrypt it with a tool such as `gpg --symmetric` or 7-Zip before it crosses a network share, and delete it once B works. Nothing is staged on either manager's disk: A streams the backup into your download, and B loads it into Docker as it uploads, so B needs room for the images, files and volumes themselves.
3. Install Thoric on B with a new data volume. Settings do not travel in the backup, since they live in `.env` outside the manager. In the WSL package, run `thoric settings` on B and copy the values you changed in A's `/opt/thoric/.env`, such as `ADMIN_PASSWORD`, `MCP_TOKEN` and `BIND_ADDRESS`, then `thoric restart`; with Compose, copy `.env` and recreate the manager. A restore reads at most `IMAGE_ARCHIVE_MAX_MB` (64 GiB by default) after decompression, so raise it on B for a larger backup.
4. On B, choose the file under **Restore a backup**. The dashboard lists what it creates and anything B already has under the same name; a restore never replaces anything, so a conflict stops it before it starts. Select **Restore** and keep the tab open until the upload ends. The job then saves the records and creates the containers.
5. The containers come back stopped, with the same hostname, MAC address and machine ID, so node-locked licenses stay valid. Start them from **Containers**. A container restored without its files, or one that mounts a volume the backup did not carry, shows why. **Retry setup** creates the first from its image, downloading a catalog image when B has none yet, and runs its compose setup again, and the second once its volume is on B. Endpoints come back without their tokens, so a token of A opens nothing on B: give each agent a new one with **Create token** on **Endpoints** under **MCP**. What B disables, such as `VPN_ENABLED=false`, is switched off in the restored containers, and a volume B's `VOLUME_FILTER` does not list is left out of them; the restore's log names both.

## Move an older installation

Before replacing an older Compose deployment, set `MANAGER_DATA_VOLUME` to its existing manager data volume name. The current default is `thoric-data`. Remove the old manager deployment without deleting that volume, then start Thoric using it. Existing containers can keep running while the manager is replaced.

If locally built desktop images still use older tags, retag them rather than rebuilding:

```bash
docker image tag thoric/ubuntu-xfce:24.04 ghcr.io/co3moz/thoric/ubuntu-xfce:24.04
docker image tag thoric/ubuntu-gnome:24.04 ghcr.io/co3moz/thoric/ubuntu-gnome:24.04
```

Saved container image IDs do not change when you add a tag.
