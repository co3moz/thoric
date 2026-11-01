# Using Thoric from an agent

[Back to the README](../README.md)

Thoric includes an MCP server at `/mcp`, on the same port as the dashboard. An agent can create a container, install software, read and write files, inspect its screen, interact with applications and save the result as an image. You do not need a separate MCP container.

The server uses the [official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk) and Streamable HTTP. The same operations are available through the [HTTP API](API.md) for scripts and other applications.

## Enable agent access

The root MCP server at `/mcp`, which reaches every container, is off by default. Generate a separate random token, for example with `openssl rand -hex 32`, and set these values in the manager's Compose environment:

```yaml
ROOT_MCP_ENABLED: "true"
MCP_TOKEN: "your-random-token"
```

With the full installation manifests, set `ROOT_MCP_ENABLED=true` and `MCP_TOKEN` in `.env` instead. Recreate the manager after changing its settings. The [WSL distribution package](WSL_PACKAGE.md) asks at first start whether to enable root agent access, how agents authenticate and whether other devices may reach the endpoint; run `thoric mcp` inside the distribution to see the endpoint together with the client settings below. An enabled root server requires a token of 1 to 256 visible ASCII characters without spaces, even if dashboard login is disabled, unless `MCP_AUTH_MODE=none` is set. The length is your choice, but a short token is easy to guess.

The setting was called `MCP_ENABLED` before 0.6.3. An `.env` that still has the old name keeps working while `ROOT_MCP_ENABLED` is missing, and the manager logs that it should be renamed; it refuses to start when the two names disagree. `thoric update` renames it in the WSL package.

[MCP endpoints](#mcp-endpoints-for-one-agent) made on the dashboard, or by an agent on `/mcp`, are not affected by this setting: each answers with its own token, so an agent can be given chosen containers while `/mcp` stays off.

Configure your MCP client with:

| Setting | Value |
| --- | --- |
| Transport | Streamable HTTP |
| URL | `http://localhost:8080/mcp` |
| Authorization header | `Bearer YOUR_MCP_TOKEN` |

Use the hostname and HTTPS address of your installation for LAN access. The token grants administrator access to the managed containers, including root script execution and deletion. It is separate from the dashboard password and does not expire with a browser session. Rotate it by changing `MCP_TOKEN` and recreating the manager.

`MCP_AUTH_MODE=none` accepts agent requests without a token; leave the authorization header out of the client settings, or keep it, since the manager then ignores it. The Host and Origin checks still apply, so a web page opened in a browser cannot call the endpoint, but any program that can reach the manager's address gets the same administrator access. Use it only while the manager listens on localhost. Desktops then do not count: the manager shares a network with each of them, and refuses every request that reaches it there. With `BIND_ADDRESS=0.0.0.0`, though, a desktop with internet access reaches the published port through the host, as a device on the network does, and the manager cannot tell it from your own browser, so every program in such a desktop gets administrator access too. The manager warns about this at startup and on the Management page.

Clients configure HTTP headers in different ways. Add the URL and bearer header using your client's MCP server settings; the server does not implement an OAuth sign-in flow. **Add /mcp to a client** on the dashboard's **Endpoints** page, under **MCP**, shows the command or settings entry for Claude Code, Codex, VS Code, Cursor, OpenCode, Gemini CLI and other clients, filled in with the address and a token you paste there. It uses stateless requests, so there is no session ID to save. The SDK handles protocol negotiation, including clients using the 2025 Streamable HTTP revisions.

## Connect through stdio on Windows

If your agent uses stdio, or something on Windows interferes with HTTP on localhost (see [broken responses from localhost](#broken-responses-from-localhost-on-windows)), use the bridge bundled in the manager image. It connects to the same `/mcp` service from inside the container and relays MCP messages over stdin/stdout.

For clients that use an `mcpServers` configuration:

```json
{
  "mcpServers": {
    "thoric": {
      "command": "wsl.exe",
      "args": ["-d", "Thoric", "--", "docker", "exec", "-i", "thoric-manager", "python3", "/app/mcp_bridge.py"]
    }
  }
}
```

Use your WSL distribution and manager container names; `Thoric` is the distribution installed from the package, and a manual installation uses its own distribution name such as `Ubuntu-24.04`. The local development preview uses `thoric-preview`. For an agent already inside WSL, use `docker` as the command and start the argument list at `exec`.

The root `/mcp` must be on (`ROOT_MCP_ENABLED`) for the bridge's default address. The bridge uses the manager's existing `MCP_TOKEN`, or none with `MCP_AUTH_MODE=none`, so you do not put a token in the client command. Anyone able to run Docker exec already has access to that Docker Engine. The bridge negotiates MCP `2025-11-25`, which the connected client must support. HTTP clients can continue to use the normal endpoint directly. The bridge also reaches a [scoped endpoint](#mcp-endpoints-for-one-agent), which answers while `/mcp` is off too: give it that endpoint's path and token, `docker exec -i -e MCP_BRIDGE_URL=http://127.0.0.1:8080/mcp/NAME -e MCP_TOKEN=ENDPOINT_TOKEN thoric-manager python3 /app/mcp_bridge.py`, and it names the session the endpoint assigns on every request after `initialize`. Once the client sends `notifications/initialized`, the bridge also opens that session's own stream, which a stdio client cannot open itself, and relays what the endpoint sends there, such as `notifications/tools/list_changed` when a desktop server is added, removed or changes its tools. Listing the tools again is still up to the client. A lost stream is opened again after 20 seconds, and the endpoint then repeats what it kept of it, so a notification can arrive twice. When the endpoint answers `404` because it no longer knows the session, after five idle minutes or a restart of the manager, the bridge opens a new one by itself: it sends the client's `initialize` again and keeps that answer, which the client already has, sends `notifications/initialized`, opens the new session's stream and sends the refused request once more. A request refused again, or one whose session cannot be opened again, fails as before, and the next request tries once more. After a new session the bridge also sends the client `notifications/tools/list_changed`, since no change to the tools could reach it while there was no session. Without `-e MCP_TOKEN` it sends the manager's own token, which opens every endpoint too; the dashboard's settings for Claude Desktop use that form, so no token is written into them.

### Broken responses from localhost on Windows

Some Windows security products scan HTTP traffic on localhost and rewrite responses on the way. The web protection in Norton does this for requests whose path is `/mcp` (and `GET /sse`), on any local server: it replaces a `Content-Length` header with `Transfer-Encoding: chunked` but leaves the body unchunked, so clients drop the connection with errors such as `Invalid character in chunk size` (Node.js) or `Malformed encoding found in chunked-encoding` (curl). A response that is already chunked passes through unchanged.

The manager therefore sends every `/mcp` response chunked, including the `401` for a missing token. Managers up to 0.2.0 still used `Content-Length` and are affected: update them (`thoric update` in the WSL package), or exclude `localhost` or the manager's port from the product's web or HTTP scanning, or use the stdio bridge above, which reaches the manager without the Windows loopback connection.

To check what reaches Windows, send the same request from Windows and from inside the WSL distribution, where the security product does not see the traffic. In `token` mode both should print `401` with `transfer-encoding: chunked` and the JSON error body:

```bash
curl -si -X POST http://127.0.0.1:8080/mcp
```

## A useful first workflow

1. Call `get_settings` and `list_images` to see the available images and defaults.
2. Call `create_container` with a name and an image: a catalog image's `reference` or a saved image's `image_id`. A feature it leaves out, such as `gpu` or `internet`, gets the installation's default, which `get_settings` reports in `defaults`; one the installation switched off, such as `internet_enabled: false`, is refused. `gpu_enabled` is false also where the host has none of the GPUs the settings allow, and `gpu` in `get_settings` says which routes GPU containers get and why not the others. A catalog image that is not downloaded yet is pulled first, which can take several minutes; `pull_image` fetches it ahead of time.
3. Call `get_job` with the returned `job_id` and `wait_seconds` until it completes, then use the result's `container_id`. The job completes once the desktop is ready, so the desktop takes input as soon as it does; a desktop that stops, keeps restarting or is not ready within five minutes fails the job with the exit code and last log lines ([desktop readiness](API.md#desktop-readiness)). The same holds for a `start` or `restart` of `container_action`, `retry_container` and `reset_container`, and for an `update_container` that recreates the container, which restores the previous one instead.
4. Call `exec_container` to install applications or prepare files. Use `root=true` explicitly when package installation needs root. For a common application or toolchain, such as Node.js, [build the image from a template](#build-from-a-template) first and create the container from that image instead.
5. Call `launch_application` to start a graphical program, then `act_and_observe` or `screenshot` and `desktop_input` to use it. Inspect the returned screenshot before the next step.
6. Call `snapshot` to save the desktop's filesystem changes. After that job completes, its image can start another container.
7. Delete a container only when its remaining container files are no longer needed. Saved images and named volumes are preserved.

[Examples](EXAMPLES.md) follows three complete tasks with these tools: building an image from a template, testing a web app in a browser with a recording, and asking a person for a CAPTCHA.

An accepted job is not a completed operation. `get_job` reports `queued`, `running`, `completed`, `failed` or `interrupted`, with the log and result. Give it `wait_seconds`, at most 300, to wait for the job to end instead of polling: it answers as soon as the job ends, or with the job as it is once the time is up, reporting progress every ten seconds to a client that asked for it; a client that ends calls after a minute without resetting on progress needs a shorter wait. Its `log` is the last 4096 bytes unless `log_tail_bytes` asks for more, up to 262144, and `log_start` and `log_end` say where that part lies in the whole output, so `log_offset` set to the last answer's `log_end` returns only what was added since. A job that creates a container or an image also reports the `name` it gives it. While an image downloads, it also reports `progress` with `current` and `total` bytes and `layers_done` of `layers`. A `build_image` job lists its `stages` (`base`, `script`, `layer`, `image`, `cleanup`) with `started_at` and `finished_at` seconds, so a long build shows whether the script is still running or Docker is saving the image layer; the base image download and the script have a `progress`, as described in [the API guide](API.md). A build script, and a compose setup step, can report its steps with lines of output such as `THORIC_STAGE=3/10 Installing Ghidra` when the third of ten begins and `THORIC_SKIP=4/10 Wine` for one it leaves out; without them Thoric follows the line the script reached. While the scripts run, `progress.script` has the running `part` of `parts` and its `name`, the last `stage` of `stages` with its `label` and whether it was `skipped`, or else the `line` of `lines` and the command there as its `label`, and the `percent` of all parts done. The job keeps a `script` report with each part's times, whether it was `cached`, and the `steps` it named, so after a build `get_job` still shows which optional steps were skipped ([script progress](API.md#script-progress)). A `snapshot` job lists `check`, `commit` and `record`, and an `update_container` job that recreates the container lists `stop`, `commit`, `create`, `start` and `cleanup`, with `stop` and `start` skipped for a stopped container and `start` lasting until the new desktop is ready. Docker reports no progress while it commits the container layer, so `commit` only shows how long it has been running. A backup of the installation lists `plan`, `images`, `files` and `volumes`, and a restore `verify`, `images`, `files`, `volumes`, `records` and `containers`; only the dashboard and the [HTTP API](API.md#backup-and-restore) start them, and no tool does, since a restore creates containers and images at once and a backup carries the installation's secrets. Inspect a failed or interrupted job before trying again. A lost connection does not mean a queued operation was cancelled. `cancel_job` cancels a queued job, or stops a running `exec_container` script, image build or image download, which then ends as failed with `Cancelled by request`; a running job that changes a container, an image or a volume is refused and finishes.

## Tools

The server currently exposes 79 tools:

| Work | Tools |
| --- | --- |
| Installation | `get_settings`, `get_system`, `get_usage` |
| Container lifecycle | `list_containers`, `get_container`, `create_container`, `update_container`, `container_action`, `set_internet`, `retry_container`, `reset_container`, `delete_container` |
| Work inside a container | `exec_container`, `read_file`, `write_file`, `upload_file`, `download_file`, `list_files`, `move_file`, `delete_file`, `screenshot`, `save_screenshot`, `desktop_input`, `act_and_observe`, `wait_for_condition`, `inspect_ui`, `find_text`, `gamepad_input`, `capture_audio`, `inject_audio`, `container_logs`, `container_diagnostics` |
| Interactive terminals | `open_terminal`, `list_terminals`, `terminal_input`, `terminal_output`, `close_terminal` |
| Applications and windows | `launch_application`, `list_applications`, `get_application`, `application_logs`, `application_diagnostics`, `sample_metrics`, `stop_application`, `list_windows`, `focus_window`, `set_window_geometry` |
| Recordings and evidence | `start_recording`, `stop_recording`, `list_recordings`, `export_artifacts` |
| A person's help | `request_human` |
| Images | `list_images`, `get_image_layers`, `pull_image`, `build_catalog_image`, `import_image`, `build_image`, `get_image_compose`, `set_image_compose`, `snapshot`, `delete_image` |
| Persistent storage | `list_volumes`, `create_volume`, `copy_volume`, `delete_volume` |
| Large files and archives | `create_transfer` |
| MCP servers in desktops and endpoints | `list_mcp_servers`, `call_mcp_server_tool`, `register_mcp_server`, `remove_mcp_server`, `list_mcp_endpoints`, `create_mcp_endpoint`, `update_mcp_endpoint`, `delete_mcp_endpoint`, `create_mcp_endpoint_token` |
| Operation results | `list_jobs`, `get_job`, `cancel_job` |

Each tool advertises its arguments and description through `tools/list`. Read operations are marked read-only; deletion, script execution, file replacement and desktop interaction carry destructive-operation hints for clients that show approvals. Every tool has a `title` for clients to show a person. `idempotentHint` is true for the read-only tools and for those that change nothing more when called again with the same arguments, such as `set_internet`, `update_container`, `set_window_geometry`, the deletes and the creates refused for a name in use; `container_action` is not, since a second `restart` restarts again.

Successful data results are returned as `structuredContent`, with text content for clients that do not use structured results. Lists are wrapped in an `items` object. A result that carries data sends it once, in a content block of its own before the text: `download_file` returns a PNG, JPEG, GIF or WebP picture as an image block and any other file as an embedded resource whose `blob` is the file in base64, while `structuredContent` and the text describe the file without its bytes. `screenshot` and `act_and_observe` return a JPEG image block and `capture_audio` a WAV audio block the same way, with the image's size and place on the desktop, or the sound's levels, as text and in `structuredContent`. Tool failures return `isError=true` and an explanation. So does a missing or unknown top-level argument, with a text such as `Unknown argument: root` that the model can correct its call from, as MCP `2025-11-25` asks of input validation; only an unknown tool name is a protocol error (`-32602`).

## Build from a template

The dashboard's **Templates** page lists ready-made images, applications such as Unity Hub and environments such as Node.js, and an agent can build the same images. Prefer a template to a build script of your own: its script has been tried, it may bring a compose and [MCP servers](#mcp-servers-inside-a-desktop), and the page then shows it as installed. The server instructions and the `build_image` description tell agents the same steps:

1. Call `list_images`. An image built from templates names them in `template_ids`; when the one you need is already there, create the container from that image.
2. Read `templates_url` from `get_settings`, the address of the list. It is `null` when the installation turned the page off, and then there are no templates.
3. Fetch the list with your own HTTP client. The manager never fetches a template, so the list has to be reachable from the machine the agent runs on. Each entry of its `templates` has an `id`, a `name`, a `description`, `tags`, a `kind` (`application`, or `environment` for a toolchain) and possibly `requires`, the oldest release that can build it. Skip an entry whose `requires` is newer than `version` in `get_settings`, since that release would refuse the build.
4. Fetch `<id>.json` from the same folder as the list. It is a build request, the JSON **Export configuration** copies.
5. Call `build_image` with that JSON unchanged and `template_id` set to the entry's `id`, then poll `get_job` as for any build.

`<id>.png` in the same folder is the icon the page shows; pass it as `icon`, a `data:image/png;base64,` URL, to show it beside the image. Several templates go into one image through `parts`, as [composes](#ask-for-values-with-composes) describe. A template can also be added to an image built earlier: give that image's `image_id` as `base_image`, and the new image keeps the base's compose before the template's own, unless `inherit_compose` is `false` ([API](API.md#a-base-images-compose)). [The development guide](DEVELOPMENT.md#add-a-template) describes the list format in full.

## Run commands and install software

`exec_container` takes `container_id`, `script`, optional `root` and optional `timeout_seconds`. It runs bash with fail-on-error. The default user is UID/GID 1001:1001 and the working directory is `/home/desktop`, or `/root` with `root`.

For example:

```json
{
  "container_id": "CONTAINER_ID",
  "root": true,
  "timeout_seconds": 600,
  "script": "apt-get update\napt-get install -y --no-install-recommends git curl"
}
```

The container needs internet access for package downloads. Its CPU, memory and networking settings still apply. `root=true` is an administrator operation and does not depend on whether desktop-user sudo was enabled.

Output goes to the job log. A nonzero exit or timeout fails the job. Either way the job's `result` holds `exit_code`, `0` on success and `null` when a signal or the manager's own limit ended the script, `timed_out`, and `output_tail`, the last 8 KiB of what the script printed on stdout and stderr without the manager's own lines, beside `container_id` and `user`. The default time limit is 300 seconds; the allowed range is 1 to 3500. The script may contain at most 1,000,000 bytes. Scripts are copied into the desktop before they run, so large scripts do not rely on Docker command-line argument limits. The script's standard input is closed: a command that reads input receives end-of-file instead of consuming the rest of the script.

A script shares the container with desktop input, screenshots, file access, terminals and applications, which go on while it runs; a reset, a snapshot, a settings change, a stop or a delete waits for it to finish. That wait is fair, so once one of those waits behind a long script, new input, file access, uploads, terminals, applications, `start_recording` and `export_artifacts` wait behind it in turn and answer 409 after five seconds, until the script ends or `cancel_job` stops it; screenshots and waits go on. Still, do not start a graphical application in the foreground of a script, since the job ends only when it exits: use [`launch_application`](#launch-and-observe-applications) instead, which returns immediately.

## Ask for values with composes

A compose makes a saved image ask for values, such as an account, when a container is created from it, and run a setup script with them once the desktop is ready. `set_image_compose` takes the record's `image_id` from `list_images` and a `compose`, or `null` to remove it; `get_image_compose` returns it with its script, and `list_images` shows the parameters without it. `build_image` accepts a `compose` for the image it builds, and a `description`, an `icon` (a PNG of at most 256 by 256 pixels as a `data:image/png;base64,` URL), a `template_id` and `mcp`, the [MCP servers](#mcp-servers-inside-a-desktop) the image declares, so an exported build configuration or [a template](#build-from-a-template) is a request it takes as it is. Instead of one `script` it takes `parts`, the scripts of several templates merged into one image, each `{"id", "name", "script"}`, run in order and each in a layer of its own, with their ids in `template_ids`; such an image's compose has a step for each template, `{"steps": [...]}`, whose scripts receive only their own parameters, as [the HTTP API](API.md#merged-templates) describes. `list_images` names an image's icon by a version instead of carrying the picture.

```json
{
  "image_id": "IMAGE_RECORD_ID",
  "compose": {
    "parameters": [
      {"name": "UNITY_EMAIL", "label": "Unity email", "required": true},
      {"name": "UNITY_PASSWORD", "label": "Unity password", "required": true, "secret": true}
    ],
    "script": "set -eu\nprintf '%s\\n' \"$UNITY_EMAIL\" > ~/.unity-account\n"
  }
}
```

`create_container` then takes the values in `parameters`, such as `{"UNITY_EMAIL":"dev@example.com","UNITY_PASSWORD":"..."}`; with a compose of several steps a value is `STEP.NAME`, such as `"vscode.GIT_EMAIL"`, or the name alone when only one step asks for it. Its job completes after every script succeeded, and `get_container` reports the result in `setup`, with each step's `status` in `setup.steps`. A secret value is never returned and appears as `***` in the job log. A parameter's `type` is `text`, `number`, `boolean`, `choice`, `choices` or `volume`, and its `display` picks the dashboard control; the values an agent sends are text in the form the script reads, such as `"4"`, `"true"` or `"linux\nwebgl"`. A `text` parameter can have a `pattern`, a regular expression in the syntax of Rust's regex crate that a value must match as a whole, and a `pattern_message` describing the expected format, which the error repeats when a value does not match; an empty value is left to `required`. A `volume` parameter has a `path`, such as `/opt/unity`, and takes the name of a volume from `list_volumes`, other than one marked `manager`, or `create_volume`, or `""` for none: the volume is mounted at the path, and the script reads the path from the parameter's variable and the volume from the same name with `_VOLUME` appended. With `"new_volume": true`, `""` or no value gives the new container a volume of its own, `thoric-CONTAINER_ID-name`, which its creation job creates, so there is no `create_volume` to call first and nothing to clean up when the request is refused; `"-"` gives it none instead, unless the parameter is required. `list_volumes` reports such a volume's `owner`: its `container_id`, `parameter` and the container's `name`, or `deleted: true` once that container was deleted. `delete_container` keeps it unless `delete_volumes` is `true`, and then removes only the volumes created for that container that no other container mounts, listing them in the job result as `volumes_deleted` and `volumes_kept`. The rules for names, values, volumes, reruns and snapshots are in [the HTTP API](API.md#composes).

## See what takes disk space

`list_images` gives every image Docker has three sizes in bytes: `size` as Docker reports it, `layers_size` unpacked and `own_size`, the part no other listed image brought. Images share the layers they have in common, so a snapshot's `size` includes its base desktop while its `own_size` is only what the container changed. `get_image_layers` takes an image's `id` from `list_images` and lists the steps of its history, newest first, each with the bytes it wrote, its `instruction` and `from`, the image it came from, so an agent can tell which step of a build script made an image large. The fields are described in [the HTTP API](API.md#image-sizes-and-layers).

`delete_image` takes an image's `id` from `list_images` and deletes it. It cannot be undone, so check the name first. A build, a snapshot or an archive loses its record and Docker deletes the image, unless another tag or another image still holds it; `image_removed` in the result tells which. An `import` was on the Docker host before it was registered, so only its record goes. A catalog image loses only its download and stays listed, so `pull_image` or a new container downloads it again. It answers at once, without a job, and deletes nothing when it refuses: for an image a container was created from, whose name the error gives, since `reset_container` creates the container from that image again, unless another saved image keeps the same Docker image, and for one Docker still uses elsewhere. Snapshots made from a deleted image keep working, because Docker keeps the layers they share with it.

## Read and write files

`read_file` takes a canonical absolute path and reads as the desktop user. Its default encoding is `utf8`; use `base64` for binary data. Each call reads up to 512 KiB, with a default of 64 KiB. Results include `bytes`, `size`, `next_offset` and `eof`; use `offset` and `limit` to page through larger files. Offsets count bytes. A UTF-8 slice that cuts a character or contains binary bytes needs a different boundary or base64 encoding.

`write_file` takes `path` and `data`, with the same encoding choices. It creates missing parent directories and saves the complete file atomically. Existing files require `replace=true`. Writes are limited to 512 KiB and use mode 0600. There is no root file-write option; destination permissions still apply. The result gives the `path`, `bytes` and `download_url`, a link for the person, as [Hand files to the user](#hand-files-to-the-user) describes.

Both tools reject path traversal and symlinks. Reads accept regular files only, so devices, sockets and FIFOs cannot turn a bounded read into an endless stream.

`list_files` returns a directory's entries with type, size, modification time and mode. `move_file` renames or moves within the same volume and needs `replace=true` to overwrite. `delete_file` removes a file, a symlink itself or, with `recursive=true`, a directory with its contents; it refuses top-level directories, the desktop home and mounted volume roots. `download_file` returns a whole regular file of at most `max_bytes`, 1 MiB by default and 2 MiB at most, with its size and a guessed MIME type: a picture as an image block a model can look at, any other file as an embedded resource, so its bytes travel once and the reply stays within its 8 MiB. `create_transfer` and the HTTP download route move larger files, and a folder as a `.tar.gz`. Its result also carries `download_url`, described under [Hand files to the user](#hand-files-to-the-user).

`upload_file` is the smaller-file counterpart of the browser upload feature. It accepts standard base64 data and a destination folder anywhere in the desktop home, in a writable mounted volume or in one of the `places` of the container's `setup`. Its MCP limit is 512 KiB; for larger files, `create_transfer` issues a link to the streaming HTTP upload route that needs no dashboard session, as [move large files](#move-large-files) describes. See [files in the HTTP API](API.md#files).

## Launch and observe applications

`launch_application` starts a program in the background as the desktop user and returns an `application_id`. It takes `command`, optional `args`, `cwd` (default `/home/desktop`), `env` and `name`. Set `gpu=true` to run the program through `gpu-run` for hardware OpenGL; this needs a GPU-enabled container. A Chromium-based browser needs no flags of its own, as [running graphics applications](USAGE.md#running-graphics-applications) explains. The call returns after a short check, so a missing program or working directory is reported as `failed` immediately.

`get_application` and `list_applications` report `state` (`starting`, `running`, `exited`, `failed` or `unknown`), the process ID, `exit_code` or the terminating `signal`, and the log size. `application_logs` returns the last `tail_bytes` of the combined stdout/stderr log, which keeps about the latest 1 MiB. `stop_application` sends SIGTERM to the application's process group and SIGKILL after `grace_seconds`.

`application_diagnostics` adds the application's process tree with memory use, open file and thread counts and the latest log lines. `sample_metrics` measures CPU and resident memory of those processes over `duration_ms` and reports container memory from cgroups; a frame rate is not available, so judge rendering from screenshots.

`get_usage` reads the history the manager keeps of every running container, one sample a second for 30 minutes, and returns the latest, average and peak over the last `seconds` (default 60, at most 1800) for one `container_id` or for all of them. A deleted container is still reported, with `removed: true` and the name it had, until its samples are 30 minutes old. `cpu_cores` counts host CPU cores in use, not a share of the container's `cpu_limit`; disk and network are bytes per second. Its `gpu` is the whole host GPU from `nvidia-smi` while a GPU container runs: neither WSL nor the driver attributes GPU use to a container, so never read it as one container's share. See [resource usage](API.md#resource-usage) for the figures and their limits.

`wait_for_condition` replaces fixed delays: it polls until a window title appears, a launched application reaches `application_state` (default `exited`) or logs `log_contains`, the screen has not changed for `screen_stable_ms`, the screen differs from how it looked when the wait began (`screen_changed`), or `text` is read on the screen (`text_state` `visible`, the default) or no longer is (`gone`). `region` limits the screen and text conditions to `x,y,width,height`. A change counts pixels that turned noticeably brighter or darker, and `screen_change_percent` (default 0.5, from 0.01 to 100) is the share of them it takes, so a blinking cursor or the panel clock does not end the wait; to notice a small change, such as a label or a button, wait on its region. `text` is read with OCR as `find_text` reads it and matched the same way, within one line and ignoring case. All supplied conditions must hold. After `timeout_ms` it returns `satisfied=false` with the last observation instead of an error: `screen_changed_percent`, `text_found` and `text_match`, the box of the first place the text was read, among them. A check that could not look, because both of the installation's screenshot slots stayed taken or a step ran out of time, does not end the wait: it goes on, and when the last check could not look the answer says why in `last_error`. `screen_changed` compares with the first screen the wait could capture, which comes later while the slots stay taken. No check runs past `timeout_ms`, except that the first one may take up to five seconds, so a shorter wait still looks once.

Supervisor files live under `/tmp` in the desktop, so they are not part of snapshots and do not survive a desktop restart; afterwards the state is `unknown`. Application records are removed with their container. A launched application does not hold the container lock, so scripts, file access and desktop input keep working while it runs.

`list_windows` returns visible X11 windows with their IDs, titles, PIDs, geometry and the active window. `focus_window` raises a window so keyboard input reaches it, and `set_window_geometry` moves or resizes it so that screenshots and coordinates stay predictable. These tools, window screenshots and relative pointer movement use `xdotool` and `wmctrl` in the desktop image; desktops built from images that predate them need an updated image.

## Record and export evidence

`start_recording` captures the desktop as JPEG frames at `fps` (1 to 5) until `stop_recording`, `max_seconds` (up to 600), 3000 frames or 256 MiB. `max_width` scales frames (default 960) and `region` limits them to part of the desktop. `list_recordings` shows each recording's state, frame count and size; frames are regular files under `/tmp/thoric-recordings/<recording_id>` and can be read with `read_file`.

Stopping a recording turns its frames into `recording.mp4` beside them, an H.264 video in MP4 that every browser plays, in which each frame shows from the moment it was captured, so the video runs at the speed the screen changed. The video scales a frame larger than 3840x2160 in area down to it, and the frames themselves stay as they are. The stop result describes the video under `video`: its `path`, a dashboard `download_url`, `codec`, `container`, `mime_type`, `width`, `height`, `frames`, `duration_ms` and `bytes`. The desktop encodes it as the desktop user with GStreamer's `x264enc` at its `veryfast` preset on two threads and a lowered priority, for at most 90 seconds. A desktop image from Thoric 0.8.4 or earlier has no encoder, so it gets `recording.avi`, an MJPEG video that VLC, mpv and ffmpeg play at the recording's `fps`; so does a recording whose encoding runs out of time. Then `video.fallback` says why. A recording that already ended at `max_seconds` or a limit gets its video when it is stopped or exported. `export_artifacts` packages recordings with their frames and video, and application logs, into a tar.gz with a `manifest.json` whose `videos` names each recording's video, saved in the desktop home, a writable mounted volume or a compose folder so the files outlive the desktop, and returns the archive's `path` and `download_url`. Recording and application files live in tmpfs and disappear when the desktop restarts, so export them before resetting or restarting a container.

## Listen, talk, type and press buttons

`capture_audio` records what the desktop plays for up to 10 seconds and returns a WAV audio block plus `peak`, `rms` and `silent`, so an agent can tell whether sound is produced even without listening. With `source=microphone` it records what reaches the desktop's microphone. `inject_audio` plays 16-bit PCM WAV data into the microphone or the speakers to test voice input or playback. Both need audio enabled for the container; the microphone side also needs the microphone option.

`open_terminal` starts bash on a pseudo-terminal and returns a `terminal_id`; `terminal_input` types text (with Enter by default) and `terminal_output` reads from a byte offset, returning `next_offset` for incremental reads and stripping escape sequences. Use it for installers that ask questions, debuggers and other interactive programs; `close_terminal` hangs up, kills a shell that ignores the hangup, and forgets the terminal with its output. `root=true` opens a root shell and is an explicit administrator choice. Terminal state lives in tmpfs and ends with a desktop restart; `list_terminals` then leaves such terminals out. A shell that exits by itself stays listed, with its output, until it is closed.

`inspect_ui` reads the AT-SPI accessibility tree of applications that expose one, such as GTK and Electron applications or Qt with `QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1`. Nodes carry roles, names, states, screen extents, text and action names, which lets an agent find a button by name instead of guessing pixels. Games and Unity players usually expose no tree; use `find_text` or screenshots for those.

`find_text` reads the text on the screen itself with Tesseract OCR, so it works where `inspect_ui` sees nothing: games, Unity players, many Electron applications, a remote session or an image. It returns `lines`, or every word with `level=word`, each with its `text`, a box `x`, `y`, `width`, `height` in desktop pixels that `desktop_input` takes as they are, and a `confidence` from 0 to 100; words Tesseract is less sure of than `min_confidence` (default 50) are left out. With `text` it also returns `found` and `matches`, every place that text is read within one line, ignoring case unless `case_sensitive`, with its box and `center_x`, `center_y` to click. `region` reads only `x,y,width,height`, which is faster and keeps other text out. Tesseract runs in the manager, so it reads every desktop image; the published manager image includes it with English data, and a manager without it answers that it is missing. OCR reads English and misreads small, faint, stylized or low-contrast text, and a word it splits or joins differently is not found; look for a distinctive word and check a screenshot when nothing is found.

`gamepad_input` drives a virtual Xbox-style controller in containers created with `gamepad=true`. Buttons stay pressed and axes keep their value between requests until `reset`, so holding a stick while pressing a button is a two-action request. The device appears as `/dev/input/eventN` inside the desktop; SDL applications normally detect it, and `SDL_JOYSTICK_DISABLE_UDEV=1` in `launch_application` env helps those that rely on udev.

## Use the mouse and keyboard

`desktop_input` sends actions through RFB, the VNC protocol, to the same desktop used by the browser. It opens a shared connection, so another viewer can stay connected. Coordinates are desktop pixels; they match a screenshot taken at full resolution. For a scaled or cropped screenshot, the `scale` and origin it returned map a point of the image back: `x = origin_x + image_x / scale` and `y = origin_y + image_y / scale`.

```json
{
  "container_id": "CONTAINER_ID",
  "actions": [
    {"type": "click", "x": 400, "y": 250},
    {"type": "key_press", "keys": ["Control", "a"]},
    {"type": "type_text", "text": "Hello from the agent"},
    {"type": "key_press", "keys": ["Enter"]}
  ]
}
```

Supported actions are `move`, `move_relative`, `click`, `button_down`, `button_up`, `drag`, `scroll`, `key_press`, `key_down`, `key_up`, `hold_keys`, `type_text`, `wait` and `release_all`. Clicks support left/middle/right buttons and up to three clicks. Drags have start/end coordinates and a duration of 100 to 2000 ms. Scroll accepts up/down/left/right and 1 to 50 steps.

Key chords press the supplied keys together, then release them in reverse order; a chord has 1 to 8 distinct keys. Special names include Control, Alt, Shift, Super, Enter, Tab, Escape, Backspace, Delete, Insert, Home, End, PageUp, PageDown, Space, arrows and F1 to F12. Text supports Unicode; a newline presses Enter and a tab presses Tab.

`key_down` and `button_down` keep keys or buttons pressed for the following actions in the same request, so an agent can walk with W held while turning with the mouse, or drag with a held button. `hold_keys` presses keys for a given time. Everything still held is released when the request ends, and the response lists it under `released`; use `key_up`, `button_up` or `release_all` to release earlier. Held input does not carry over to the next request.

`move_relative` moves the pointer by `dx`,`dy` from its current position, optionally over several `steps` and a `duration_ms`. RFB, the VNC protocol, only carries positions, so the manager sends this motion through XTest inside the desktop instead, with `xdotool` run as the desktop user, in order with the rest of the request: applications that grab the pointer and read raw motion, such as first-person cameras built on SDL, receive the deltas like those of a physical mouse, also at the desktop's edge, where the pointer itself stops. Buttons pressed or released afterwards without coordinates act where the motion left the pointer, and held keys and buttons stay held through it. The response's `pointer` is where the pointer ended and `relative_motion` says how it moved: `{"method":"xtest"}`, or `{"method":"emulated","reason":"..."}` when the desktop image has no `xdotool`, in which case the motion is a series of absolute positions that stops at the edge and that such applications may not follow.

Requests allow 1 to 100 actions, up to 10,000 typed characters, single waits or holds of at most 10 seconds and at most 30 seconds of combined waits, holds and gesture delays. The whole action batch is validated against the connected desktop's dimensions before input is sent.

`screenshot` captures the desktop at full resolution by default. `max_width` scales the image down, `region` crops `x,y,width,height` and `window_id` captures one window from `list_windows`. The result's text and `structuredContent` hold `width`, `height`, `captured_at` and where the image sits on the desktop: `scale`, image pixels per desktop pixel, `origin_x` and `origin_y`, the desktop pixel at its top left, and `desktop_width` and `desktop_height`. A part the desktop image cannot tell, such as a window's place without `xwininfo`, is `null`. `desktop_input` takes desktop pixels only, so a point of the image is converted with the formula above rather than sent as it is. MCP clients and models often scale a large image down before the model sees it, which shifts every coordinate read off it; pass `max_width`, such as 1280, so the image arrives at the size the model sees, and the returned `scale` maps it back. `act_and_observe` sends actions, waits `settle_ms` (default 500) and returns the screenshot together with the input result, which is the shortest loop for driving an application. The installation takes two screenshots at a time: another one waits up to five seconds for its turn and is then refused as busy, so try again, and a capture the desktop has not finished in 20 seconds fails.

`save_screenshot` takes the same options under `capture` and writes the JPEG inside the desktop at `path`, an absolute file path in the desktop home, a writable mounted volume or a compose folder, as the desktop user and under the upload destination rules. It returns the file's `path`, `bytes`, `width`, `height`, `captured_at` and `download_url`. An existing file needs `replace=true`. `screenshot` stays a read-only tool; the image it returns reaches the agent, not necessarily the person.

## Ask a person for help

Some steps are for a person only: a CAPTCHA, a second sign-in factor, a license or consent window, a sign-in screen. Instead of trying to get past one, an agent calls `request_human` with `container_id` and a `message` saying what is needed and where, and waits:

```json
{"container_id": "CONTAINER_ID", "message": "Solve the CAPTCHA on the sign-in page in Chrome, then press Done."}
```

The dashboard shows the request on every page, with the container's name, the message, how long ago it came and **Open desktop**, and as a strip above that container's desktop view, with a toast when it arrives. Whoever is there answers **Done** or **Decline**, with an optional reply, and the call returns at once with `status` `done` or `declined`, the `reply` and the `request_id`. Take a screenshot before going on: the answer says what the person did, not what the screen shows now.

MCP clients give up on a call that runs too long, often after a minute, so one call waits at most `wait_seconds`: 45 by default, up to 300, or 0 to return at once. Without an answer by then it returns `status` `pending` with the `request_id`; call `request_human` again with that `request_id` instead of `message` to keep waiting, as often as needed. Asking again with the same `message` while the request is pending joins it rather than asking twice, which helps an agent whose call was cut off before it learned the id. A client that gives up on a call does not withdraw the request: it stays on the dashboard until someone answers, and an answer can be read with its `request_id` for an hour. Both work only through the channel that asked, `/mcp` or the same endpoint: an agent on another endpoint asks a request of its own and cannot read this one. Deleting the container declines its open requests, and deleting an endpoint those asked through it, with a `reply` that says so. While a call waits, a client that sent a `progressToken` gets a progress notification every 10 seconds, which keeps a call open in clients that reset their timeout on progress.

Requests live in the manager's memory, at most 64 waiting at once. A manager restart ends them, and waiting on one afterwards fails with an error that says so; ask again if a person is still needed. On an [MCP endpoint](#mcp-endpoints-for-one-agent) the tool comes with the `input` scope and asks for the endpoint's containers only, and the dashboard names the endpoint beside the request. The [HTTP API](API.md#requests-for-a-person) lists and answers requests too.

## Hand files to the user

Tool results reach the agent, not necessarily the person reading the conversation: many clients never show an image returned by a tool, and relaying a file as base64 through the model is expensive and bounded. The results of `write_file`, `upload_file`, `save_screenshot`, `download_file` and `export_artifacts` therefore include `download_url`, a link such as `/api/containers/ID/files/download?path=/home/desktop/Downloads/clock.jpg`. It is relative to the manager's address, the same host and port that serves `/mcp`, so it is valid wherever the manager is reachable: from a browser on the same machine, over the LAN with the HTTPS address, or from another operating system. The link opens in the dashboard session and streams the file with the limits of the HTTP download route; the same link with a folder's path downloads that folder as a `.tar.gz`. The MCP token does not authorize it, and an agent that fetches a file for itself uses `create_transfer` instead. Share the link, or the saved path, instead of looking for `docker`, `wsl` or a host shell: the server instructions tell agents that containers are reachable only through the tools and the HTTP API, because the client may run on a different machine than the manager.

The bundled TigerVNC server acknowledges input with RFB Fence. This confirms that the server processed the events; an application may still need time to update. Inspect the next screenshot before making a decision. If a connection fails after input starts, check the screen before repeating an action that could submit, purchase or delete something. A custom desktop VNC server must support RFB 3.8, shared connections and Fence.

## Move large files

The file tools carry at most 512 KiB per call as base64 through the conversation, and `download_file` 2 MiB, and the HTTP routes that stream files and archives of any size take the dashboard's session cookie, not the MCP token. `create_transfer` connects the two: it issues a link for one HTTP request, which the agent sends with curl or another HTTP client from the machine it runs on.

| `operation` | Arguments | Request |
| --- | --- | --- |
| `container_upload` | `container_id`, `destination`, `name`, optional `replace` | `PUT` or `POST` the file as the body |
| `container_download` | `container_id`, `path` | `GET` the file, or a folder as a `.tar.gz` |
| `volume_upload` | `volume`, `destination`, `name`, optional `replace` | `PUT` or `POST` the file |
| `volume_download` | `volume`, `path` | `GET` the file, or a folder as a `.tar.gz` |
| `volume_import` | `name` of a new volume | `PUT` or `POST` a `.tar.gz` or `.tar` |
| `volume_extract` | `volume`, `path` of a new folder in it | `PUT` or `POST` a `.tar.gz` or `.tar` |
| `volume_export` | `volume` | `GET` a `.tar.gz` |
| `image_import` | optional `name` and `description` | `PUT` or `POST` an image archive |
| `image_export` | `image_id` | `GET` an image archive |

The result holds `url`, relative to the manager's address like `download_url`, the `method`, `expires_in_seconds` and `expires_at`:

```bash
curl --fail-with-body --upload-file project.tar.gz http://thoric.lan:8080/api/transfers/TOKEN
curl --fail --output build.zip http://thoric.lan:8080/api/transfers/TOKEN
```

The request runs the HTTP route the dashboard uses, with its limits, locks, job and progress, and answers as that route does: an upload with the saved path, an import with a `job_id` once the whole archive has arrived, an export or a folder download with the archive and its job in the `X-Job-Id` header. Follow jobs with `get_job`. An upload's size comes from `Content-Length`, which `--upload-file` sends for a file, or from a `size` query parameter for a body sent without one. An import also takes an archive of unknown size, such as `tar -c -C project . | curl --upload-file - URL`, with the checks the [HTTP API](API.md#volume-archives) describes. Nothing else in the request changes what the link does.

A link is a credential of its own:

- Its token is random, 244 bits, kept in the manager's memory only, so a restart forgets it.
- It expires after five minutes, and the first request that presents it spends it, whatever that request's outcome. A transfer that has begun runs to its end; signing out of the dashboard does not stop it.
- It runs only the operation and arguments it was issued for, and it is neither a session cookie nor a bearer token for any other route.
- It needs no cookie, no `X-Thoric-Request` header and no `Authorization` header, so a shell can use it without the administrator password or the MCP token. If the request carries an `Origin` header, it must match the manager's address.
- The manager keeps the token out of job logs, error messages and its request tracing. Treat the URL as a secret until it has been used: whoever holds it can run that one request.

Links come only from `create_transfer`, so with MCP disabled the route answers `404`. Accepting the MCP token on the HTTP API itself would have been simpler, but it would put a long-lived administrator credential into curl commands and shell history, where a leak reaches every route for as long as the token is configured.

## MCP servers inside a desktop

A desktop can run MCP servers of its own: a browser automation server attached to its Chrome, an editor's server, or one you write for a project, such as a Chrome extension you are developing. Each is declared in a JSON file named after it, in one of two folders:

| Folder | Who writes it | Source |
| --- | --- | --- |
| `/etc/thoric/mcp.d/NAME.json` | The image: a template's or build's `mcp` declarations, or its script. | `image` |
| `~/.config/thoric/mcp/NAME.json` | The desktop user, at any time: `thoric-mcp add`, the dashboard's **Desktop servers** page or `register_mcp_server`. | `container` |

The desktop user's declaration of a name replaces the image's. A declaration is either a program that speaks MCP on stdio or a Streamable HTTP server already listening on the desktop's loopback:

```json
{"description": "Playwright attached to the desktop's Chrome",
 "command": "playwright-mcp", "args": ["--cdp-endpoint", "http://127.0.0.1:9223"],
 "env": {"PLAYWRIGHT_BROWSERS_PATH": "0"}, "cwd": "/home/desktop"}
```

```json
{"url": "http://127.0.0.1:8931/mcp", "headers": {"Authorization": "Bearer ${API_TOKEN}"}}
```

Values in `args`, `env`, `url` and `headers` may name the container's [compose](API.md#composes) values as `${NAME}`, or `${STEP.NAME}` for one step of several; the manager fills them in when it connects, so a secret never has to be written into the image or the file. Secret values are filled in only for a declaration in the image's `/etc/thoric/mcp.d`, which a build writes and the manager reads itself as root, taking only files that root alone can change: root owns the file and every folder on its way from `/`, and neither another group nor anyone else can write them, a folder with the sticky bit excepted. A file there that a link, the desktop user or anyone else could change is reported as the server's error, and a folder on the way they could change hides every declaration in it. One in the desktop user's folder, as `thoric-mcp add` and `register_mcp_server` write it, names only values that are not secret, since that user writes the file and its server runs as that user. That keeps the desktop user from asking for a secret; it does not hide one an image's server is given, since that server runs as the desktop user too, who can read its environment. Give a secret only to a server whose desktop user may know it. A template's declaration names its values alone, and keeps working once its image is merged with another template or [built on](API.md#a-base-images-compose): `${NAME}` stands for the one step's value of that name, and is refused with the names to use when several steps have one. Inside the desktop, `thoric-mcp` writes the desktop user's files:

```bash
thoric-mcp add x --description "My extension's helper" -- node /home/desktop/ext/mcp.js
thoric-mcp add api --url http://127.0.0.1:8931/mcp --header 'Authorization: Bearer ${API_TOKEN}'
thoric-mcp list
thoric-mcp list --json
thoric-mcp show x
thoric-mcp remove x
```

`add` refuses what the manager would refuse anyway, such as an address that is not on the desktop's loopback, and replaces the desktop user's declaration of the same name. `list --json` prints both folders' declarations, each with its `spec`, or an `error` when its file cannot be read, and `active`, which is false for the image's declaration of a name the desktop user declares too. While the manager watches the desktop it adds `watched: true`, the `endpoints` that reach it, and each used declaration's `status` as the manager knows it: `state`, `error`, `tools` with the first line of each description, `tools_at` and the end of its stderr in `log`. `show NAME` prints the used declaration with its name, as a template's or a build's `mcp` list holds it.

The XFCE and GNOME images show the same in the panel's tray, through XFCE's Status Tray and GNOME's AppIndicator extension. The tray's menu lists the servers with their states, and its icon carries a red dot while one has an error; **Manage MCP servers**, a middle click on the icon or **MCP Servers** in the applications menu opens a window with each server's command, file, tools, error and output, and the endpoints that reach the desktop with their scopes and whether they have a token of their own. **Add server**, **Edit** and **Remove** run `thoric-mcp`, and the dialog shows the command line before it runs it, so whatever is done there can be repeated in a terminal or a script. An image's server is changed with **Change here**, which saves a copy of it in the desktop user's folder; **Use the image's** removes the copy. **Copy command** and **Copy for a template** give a server's `thoric-mcp add` command line and its declaration for a template. The tray starts with the session from `/etc/xdg/autostart`, where XFCE's Session and Startup settings can turn it off.

The manager watches both folders in every running desktop. A helper it runs as the desktop user reports that user's folder and when the image's folder changed; whatever it says, the servers it reports are the desktop user's, and the manager reads the image's folder itself as root when it starts watching, at most every five seconds after a change, and whenever a page or an agent lists a running container's servers. When that read fails, the manager's log says why, the desktop user's servers still change, and the image's stay as they were last read. A new or changed server is started once to read its tool list, which is kept, so its tools stay listed while the container is stopped. `list_mcp_servers` reports each server's `state`: `ready`, `pending` while its tools are read, `error` with the reason, or `stopped` while the container does not run, with the end of its stderr in `log`. A server that sends `notifications/tools/list_changed` has its list read again, once per burst of notifications. A list of more than 256 tools, larger than 1 MiB as JSON or spread over more than 16 pages is refused and reported as the server's error, since every endpoint sends the whole list in each `tools/list`.

The desktop hears what the manager knows of it through the same watch: whenever one of its servers or an endpoint that covers it changes, the manager sends a report on the watcher's stdin, and the watcher keeps the latest in `/run/desktop/thoric/mcp-status.json`, readable by the desktop user alone, until the manager stops watching. A report carries each server's state, tools, error and output, and for each endpoint that covers the container its name, path, scopes, servers, left-out tools and whether it has a token. It never carries a token or its hash, another container's grant or anything about another container, and the desktop never connects to the manager.

A server is never published on a port and never reached over the desktop's network. The manager runs it with `docker exec` as the desktop user, with `DISPLAY`, `HOME` and a `PATH` that includes `~/.local/bin`: a stdio program directly, an HTTP server through a small adapter inside the desktop that connects to its loopback address, so a server that listens on `127.0.0.1` only works, and its own `Authorization` header is sent. Root is never used. A declared variable's value, which may be a resolved compose value, reaches the server on stdin rather than on a command line: a small launcher in the desktop reads the variables from the start of its stdin, sets them and becomes the server's program, which reads the rest. So `ps` in the manager does not show a value, and the Docker CLI keeps the manager's own environment, which a variable such as `DOCKER_HOST` could otherwise redirect. An HTTP server's header values reach its adapter on stdin the same way. A declared argument has no such channel and is on the command line. A server whose declaration the image carries cannot be removed from a container, only left out of an endpoint.

An agent on `/mcp` calls a server's tool without an endpoint through `call_mcp_server_tool`, with the `container_id`, the `server`'s name, the `tool`'s own name and its `arguments`. `list_mcp_servers` with a `container_id` gives each tool its `input_schema`, the arguments it takes; the list of every container's servers leaves the schemas out. `/mcp` keeps a connection to each server of its own, apart from every endpoint's, so the pages it opens in a browser are not an endpoint agent's. The server's result is passed on as it is, as an endpoint passes it on, and a server in error or a tool it does not list is refused. The tool is on `/mcp` only, in no endpoint's scope, and the [activity log](#agent-activity) records it with the server and the tool and the tool's arguments by name and size, as it records an endpoint's call of a server's tool.

## MCP endpoints for one agent

`/mcp` is the administrator's channel: whoever holds its token can create, change and delete containers and run scripts as root. An agent that should only drive a desktop's browser, or a project's own MCP server, gets an endpoint instead, at `/mcp/NAME` on the same address. An endpoint covers one or several containers, and for each the scopes and desktop servers chosen for it:

| Scope | Tools, bound to that container |
| --- | --- |
| `view` | `get_container`, `container_logs`, `container_diagnostics`, `get_usage`, `screenshot`, `list_windows`, `inspect_ui`, `find_text`, `wait_for_condition`, all of them read-only |
| `input` | `desktop_input`, `act_and_observe`, `focus_window`, `set_window_geometry`, `gamepad_input`, and `request_human`, which changes nothing in the container but asks a person to take over its screen |
| `applications` | `launch_application`, `list_applications`, `get_application`, `application_logs`, `stop_application`, `application_diagnostics`, `sample_metrics` |
| `files_read` | `list_files`, `read_file`, `download_file` |
| `files_write` | `write_file`, `upload_file`, `move_file`, `delete_file`, and `save_screenshot`, which writes the image into the desktop |
| `commands` | `exec_container` and the terminal tools, as the desktop user; with `sudo` enabled for the container that is root inside it, and after `sudo` was turned off it can still be through what root left in the container, until a reset |
| `media` | `start_recording`, `stop_recording`, `list_recordings`, `export_artifacts`, `capture_audio`, `inject_audio` |
| `power` | `container_action` |

`get_job` comes with `commands` or `power`, for the jobs the endpoint's own calls started in its containers since the manager last started. The terminal tools list and reach only the terminals the endpoint opened, never one the administrator opened or a root shell. A deleted endpoint's terminals are the administrator's from then on, and its jobs are forgotten, so an endpoint made later with the same name starts with neither. No scope creates, changes, resets, snapshots or deletes a container, and none reaches images, volumes, other containers or root. The desktop servers are `all`, including those declared later, or a list of names.

A grant can also leave single tools out, with `excluded_tools`: the names the endpoint gives them for that container alone, such as `application_diagnostics` from the `applications` scope or `playwright__browser_run_code_unsafe` from the `playwright` server, which `list_mcp_servers` reports in each tool's `exposed`. An excluded tool is missing from `tools/list` and as unknown to `tools/call` as a tool outside the scopes. Without the list, the default, the endpoint offers everything its scopes and servers give; with it, a tool a server adds later is still offered unless it is named. A container tool's name must belong to the granted scopes, so a typo is refused instead of leaving the tool in; a server tool's is kept whether or not the server is known now, since its server may be stopped or declared later. On an endpoint of several containers a container tool left out for one container stays for the others and takes only their aliases, and `get_job` can be left out the same way. The dashboard's endpoint dialog lists the tools of each granted scope and each included server with a checkbox each, so nothing has to be typed.

Tools keep their names and lose `container_id`: an endpoint of one container is bound to it, and on an endpoint of several each tool takes `container`, one of the aliases of the containers granted its scope. Their descriptions leave out what an endpoint does not offer, such as `root=true` for `exec_container` and `open_terminal`, or `create_transfer` and the dashboard's download route for `download_file`, and say so instead. A desktop server's tools are named `SERVER__TOOL`, or `ALIAS__SERVER__TOOL` on an endpoint of several containers, at most 64 characters, and their descriptions start with the container and server; two tools whose names clean to the same exposed name both stay listed, the later one with a short hash tail. `get_usage` on an endpoint leaves out the host `gpu` figure, which one container's grant does not cover. Their annotations, such as `readOnlyHint`, come from inside the desktop and are left out, so a client asks before running them as it would for any tool. A result such a server returns without `resultType`, as the 2025 protocol revisions do, is passed on as `complete`, which a client of the `2026-07-28` revision requires in every result. When a server is added, removed or changes its tools, the endpoint sends `notifications/tools/list_changed`: on the stream a session of protocol `2025-11-25` or earlier opens, and on `subscriptions/listen` for `2026-07-28`. For a client that ignores the notification, a call to a tool the endpoint no longer offers is refused with a hint to list the tools again. Agents on one endpoint share each server's connection, and so its state, such as a browser's pages; another endpoint has its own. A connection unused for 10 minutes is closed.

Every new container gets an endpoint of its own unless `create_container` is given `"mcp_endpoint": false`, also one whose creation failed once it was saved, such as on a desktop that was not ready, so the endpoint works once `retry_container` or `reset_container` repairs it: named after the container, with every scope but `commands`, all its desktop servers and no token yet. Deleting the container deletes it. Other endpoints are made with `create_mcp_endpoint`, or on the dashboard's **Endpoints** page, and outlive their containers: a deleted container only leaves their grants.

An endpoint opens with its own bearer token or the administrator's MCP token. `create_mcp_endpoint` and `create_mcp_endpoint_token` return the token once; the manager keeps only its hash, and a new token ends the old one and its sessions. The endpoint's token opens nothing else: not `/mcp`, not the HTTP API, not the desktop ports. A request without a token that opens the endpoint, or the administrator's, is answered `401` whether or not the name exists, so endpoint names, which follow the containers' names, cannot be listed from the LAN; `404` for an unknown name is told to the administrator's token. The endpoints answer whether or not the root `/mcp` is on (`ROOT_MCP_ENABLED`), with the Host and Origin checks of `/mcp`. The administrator's `MCP_TOKEN`, when one is set, opens them while `/mcp` is off too, and with `MCP_AUTH_MODE=none` they need no token at all while `/mcp` is on. With `/mcp` off, `MCP_AUTH_MODE=none` opens nothing: an endpoint then needs its own token. A client connects to an endpoint as to `/mcp`:

```bash
claude mcp add --transport http chrome-dev http://localhost:8080/mcp/chrome-dev --header "Authorization: Bearer ENDPOINT_TOKEN"
```

The share button beside an endpoint's URL on the dashboard shows the same for the other common clients. The token is shown only when it is created, so the dialog asks for it and fills it in; it stays in the page.

A client of protocol `2025-11-25` or earlier gets a session from the endpoint on `initialize`. The endpoint forgets a session that carries no message for five minutes, an open stream not counting, and every session when the manager restarts. It then answers `404` to that session's ID, and MCP asks the client to start a new session with `initialize`, which the [stdio bridge](#connect-through-stdio-on-windows) does by itself.

## Agent activity

The manager records every tool call an agent makes, on `/mcp` and on every endpoint, and every transfer link that is used. The dashboard's **Activity** page shows them, and `GET /api/activity` lists them, as [the API guide](API.md#agent-activity) describes. A call is recorded when it answers, with:

- the time it began and how long it took;
- the caller: `/mcp` or the endpoint, the credential it showed (the endpoint's token, the MCP token, none with `MCP_AUTH_MODE=none`, the dashboard's inspector, or a transfer link, which names the caller whose `create_transfer` issued it), and the name and version its client gives itself;
- the tool, and for a desktop server's tool the server;
- the container it worked on, with the name it had then;
- its arguments, masked as below, and a one-line summary of them;
- how it ended: a result, an error the tool reported with the first 300 characters of its text, or a refusal before any tool ran, such as an unknown tool or an argument its schema does not take; and the job it started.

A result is never kept beyond that error text, so a token a tool returns, such as `create_mcp_endpoint_token`'s, never reaches the log. Each call also writes one line to the manager's own log (`docker compose logs manager`, or `thoric logs` in the WSL package) with its caller, tool, container, kind, outcome and duration, and none of its arguments.

A call's kind comes from its tool's annotations: `read` for `readOnlyHint`, `change` for a tool that says it is not destructive, and `destructive` otherwise, as MCP assumes of a tool that says nothing. Thoric marks the mouse, keyboard and window tools destructive, since input can change or delete an application's data, so a session of clicks and keys shows as destructive calls. A desktop server's annotations come from inside the desktop: they can mark its tool destructive but never a read, so a server cannot pass its calls off as reads.

What is kept of the arguments:

- Every Thoric tool has a list of the arguments whose values are kept: paths, names, IDs, options and limits, a text cut at 256 characters. `exec_container`'s `script`, the first thing to look at, is kept up to its first 4,096 characters, as is `build_image`'s.
- Every other argument is kept as its type and size only. That covers what `write_file` writes; the base64 of `upload_file`, `inject_audio` and an image icon; compose values in `create_container` and the composes of `build_image` and `set_image_compose`; the `proxy_password` of `create_container` and `update_container`; the arguments, environment and headers of `register_mcp_server` and of the servers `build_image` declares and the environment of `launch_application`; text typed with `desktop_input` or `act_and_observe`, and every key of a single character they press, since a password can be typed a key at a time; and what `terminal_input` types, since a terminal is where a password prompt waits. An argument a later release adds is kept the same way until its tool's list names it.
- A desktop server's tool is opaque to Thoric, so its arguments keep their names and sizes and never their values, whatever the tool is called.
- An error text loses every masked value it quotes before it is kept.

The newest 10,000 calls are kept in the manager's database, so they survive a restart, and older ones are removed as new ones arrive. Recording never fails or holds up a call: the row is written after the call has answered, and one that cannot be written is logged and lost.

Not recorded are other MCP messages, such as `tools/list`; requests through a container's [proxied ports](#reach-a-server-inside-the-desktop), which carry a service's own traffic rather than single actions, so audit a server reached that way by its own means; a transfer link that is unknown, spent or expired, since nothing ran; the dashboard's own HTTP requests, which a person makes; and the desktop servers the MCP inspector starts in a process of its own. The inspector's calls to `/mcp` and to an endpoint go through the same handlers and are recorded with `inspector` as their credential, apart from any agent's.

## Reach a server inside the desktop

For an MCP server, prefer a declaration and an endpoint, described above: they need no port and work with servers that listen on `127.0.0.1` only. A program in the desktop may serve its own API: an MCP server of the Unity editor, a test runner endpoint, a debug server. Its ports are not published, but the manager forwards HTTP and WebSocket requests to the ports a container's `ports` setting lists, at `/api/containers/CONTAINER_ID/ports/PORT/` followed by the service's own path. Set `ports` with `create_container` or `update_container`, such as `"ports": [8090]`; it is empty by default, and a change applies without restarting the desktop. The server must listen on the desktop's network, such as on `0.0.0.0`, not on `127.0.0.1` only, which the manager cannot reach.

The route takes the same bearer token as `/mcp`. An MCP server listening on port 8090 at `/mcp` inside the desktop is therefore a second MCP server for a remote agent:

| Setting | Value |
| --- | --- |
| Transport | Streamable HTTP |
| URL | `http://localhost:8080/api/containers/CONTAINER_ID/ports/8090/mcp` |
| Authorization header | `Bearer YOUR_MCP_TOKEN` |

Responses stream as the server sends them, so server-sent events work, and a WebSocket handshake joins the client to the service. The service sees a local client: `Host` is `localhost:PORT`, and the token, the dashboard cookie and `Origin` are removed, so a service that requires its own `Authorization` header cannot be reached this way. Treat what it answers as container data, like screenshots and command output. A browser cannot use the route with the dashboard session, and every response is sandboxed, so a page the desktop serves runs no script on the dashboard's address; [the API guide](API.md#desktop-ports) has the details.

## Keep agent work recoverable

MCP uses the same ownership checks, container locks, jobs and snapshot behavior as the dashboard. An operation on another container can proceed independently. A script running in one container shares it with input, files and applications, while a change that replaces or stops the container waits for the script; a launched application holds nothing.

Read/write and execution tools operate inside managed desktop containers. They do not expose host shell execution or give desktop containers the host Docker socket. Treat files, logs and screen contents as data from the container, especially when the desktop is browsing external sites.

`reset_container` gives a test a clean starting point: it discards the container's unsaved changes and recreates it from its saved source image, keeping the container ID, networks, named volumes and [machine identity](API.md#machine-identity): the hostname, MAC address and machine ID that a license bound to one machine sees. A pool of desktops reset between tasks therefore stays the same set of machines, and a new container, also one from a snapshot, is a new machine unless `create_container` is given the `hostname`, `mac_address` and `machine_id` of the container it replaces. Data inside mounted volumes is not touched; clean it explicitly with `delete_file` or a script when a test needs an empty volume. Take a snapshot first if the current state should be reusable.

`update_container` changes a container's settings after creation; pass only the fields to change. `name`, `internet`, `ports`, `groups`, `aliases` and a new or changed resource limit apply in place, and so does `resolution`: the running desktop's screen takes the new size while its applications keep running, and the desktop gets it again whenever it starts. Use `dry_run=true` first: its `restart` list names the changes that recreate the container, which restarts the desktop and closes running applications while keeping the files in its container layer.

Containers that share a name in `groups` reach each other on that [group's network](API.md#container-groups), by hostname, by name in lowercase with hyphens and by `aliases`; a test runner in one desktop can reach a server in another as `http://server:8080/`. The network has neither the internet nor the manager on it, and a name another member answers to already is refused. Members reach every port the others listen on; a container whose `list_containers` entry has `vnc_private` false also shows its screen to them, since its image predates the private VNC server.

Snapshots save filesystem changes; they do not save volumes or running application state. Use a named volume for project files that must survive container deletion, and save an image before deleting a container whose installation you want to reuse. `list_volumes` marks the manager's own volumes, such as its data volume, with `manager: true`; a container cannot mount them, so pick another one. It lists only the volumes that carry the installation's `VOLUME_FILTER` label, by default the ones Thoric created; a volume it leaves out cannot be mounted or opened, so create or copy one instead of naming a volume from elsewhere on the host.

`copy_volume` copies a named volume, `volume`, into a new one called `name`: a prepared project or cache that each task starts from, or a copy kept before a risky change. Poll `get_job` until it completes before mounting the copy. The copy keeps the tree and modes, and its files belong to the desktop user. It is a full copy, not copy-on-write, so it takes as much disk space as the volume. The rules are in [volume archives](API.md#volume-archives).

`delete_volume` deletes a named volume, `name`, with its files, such as one `list_volumes` shows with `owner.deleted: true` because its container was deleted without it. It cannot be undone, so check the name first. It answers at once, without a job, and deletes nothing when it refuses: for the manager's own volumes, for a volume a managed container mounts through its volumes or its compose, whose name the error gives, and for one another Docker container still uses. A volume `VOLUME_FILTER` leaves out is answered as missing, as `list_volumes` leaves it out.
