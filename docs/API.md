# HTTP API

[Back to the README](../README.md) · [MCP guide](MCP.md)

The dashboard and MCP server share Thoric's backend operations. You can also call the HTTP API directly from a script or another application to manage containers and build your own interface.

All paths below are relative to your installation, for example `http://localhost:8080`. Request bodies and results are JSON unless noted otherwise. Errors return an HTTP error status and an `error` message; [errors](#errors) says what each status means. The manager also describes every route, with its parameters and request bodies, as an [OpenAPI document](#openapi-document) for client generators and API tools.

Every response carries an `X-Request-Id` header, and the manager's log line for the request names the same ID as `request_id`, so an error a script or the dashboard saw can be found in [the log](INSTALLATION.md#if-something-does-not-start). Send an `X-Request-Id` of your own, up to 128 letters, digits, `-`, `_`, `.` and `:`, to have it used instead; any other value is replaced with a new UUID.

## Sign in

The HTTP API uses the same administrator session as the dashboard. Sign in and keep the returned cookie:

```bash
curl --cookie-jar thoric.cookies \
  --header 'Content-Type: application/json' \
  --header 'X-Thoric-Request: 1' \
  --data '{"username":"admin","password":"YOUR_PASSWORD"}' \
  http://localhost:8080/api/login
```

Send that cookie with subsequent requests. POST and DELETE requests also need `X-Thoric-Request: 1`. If you send an Origin header, it must match the request's host and port. These request checks still apply when `AUTH_MODE=none`; only sign-in and the cookie become unnecessary. A GET that starts work, which a [backup](#backup-and-restore), an image or volume archive and a file or folder download do, is refused with 403 when the browser names another site in `Sec-Fetch-Site` (`cross-site` or `same-site`, as another port of the same host is), so a page's image or link cannot start one. The dashboard's own downloads, an address opened by hand and clients that are not browsers, such as curl with a [transfer link](#transfer-links), are unaffected. A request that reaches the manager on a desktop's private network is refused with 403 before any of this, on every route, `/mcp` and `/healthz` included: a desktop never needs the manager, which reaches it the other way round. A desktop with internet access still reaches a port published beyond loopback (`BIND_ADDRESS=0.0.0.0`) through the host, like any device on the network, so keep sign-in on then. `GET /api/configuration` reports such a port in `exposure`, with what answers there without a credential (`dashboard`, `mcp`), and the manager logs it at startup.

The MCP bearer token authenticates `/mcp`, not these API routes; a [transfer link](#transfer-links) that an agent gets from MCP authorizes one file or archive transfer without either. Sessions last 12 hours; `/api/logout` revokes the session.

## Errors

An error answers `{"error": "..."}` with a status that says whose failure it is. The message is the same whatever the status, and the dashboard shows it as it is.

| Status | Meaning |
| --- | --- |
| `400` | The request was refused: a value that a check does not accept, or something it names that Docker does not have, such as an image or a volume. Change the request before sending it again. |
| `401` | There is no session, or it ended. Sign in again. |
| `403` | Not allowed: a foreign `Origin`, the manager's own volumes, or a file the desktop user cannot open. |
| `404` | The record, or what the route names, does not exist. |
| `409` | It conflicts with the state of something: another operation on the same container, image or volume is still running after the 5 seconds a request waits for it, a name is in use, a container is not running, a container record mounts the volume, or a desktop's display is held by a program or not running (see [desktop health](#desktop-health)). Send it again once that has changed. |
| `500` | The manager itself failed: its database, its disk, or a task that ended unexpectedly. |
| `502` | Docker failed at what it was asked, or a helper that the manager runs in a desktop or a helper container crashed. The message carries their answer. |
| `503` | Docker cannot be reached: its daemon is stopped or its socket is missing. The same request works once Docker is back. |
| `504` | A Docker command did not finish within its time limit. |

Routes name the other statuses they use, such as `413` and `429` for transfers. A route that starts a job answers once the job is queued, and the job's failure is in its log rather than in a status. MCP tools return the same message as a tool error, without a status.

A transfer ends when its client stops. A download of an image, volume or folder archive or of a backup whose client takes nothing for five minutes, as when a browser pauses it, fails its job with `The download stopped: the client took nothing for 5 minutes`, ends with an error instead of a short file and frees its transfer slot. A file upload, an archive import or a restore that receives nothing for five minutes fails the same way and answers 400 with `The upload stopped: nothing arrived for 5 minutes`. The five minutes count from the last piece that moved, at most 256 KiB, so a transfer goes on for as long as it moves faster than about 1 KB/s. An image import waits up to an hour for the image's own first bytes, since `docker save` on Docker's classic image store prepares the whole image before it writes any, which an import of its piped output, or of a Thoric export after its first entry, has to wait for; the five minutes apply from there. A backup piped straight into a restore would pause the same way before each image, so restore one from a file. A session that ends stops its transfers at once; a [transfer link](#transfer-links) and an installation with sign-in off have no session, so for them a stopped client is what ends a transfer early. The manager logs each such stop once at warn, with the method, path and request ID of the request that began the transfer.

## Endpoints

| Method | Path | Result or purpose |
| --- | --- | --- |
| GET | `/api/session` | Sign-in mode and current authentication state; available before login. |
| POST | `/api/login` | Sign in with `username` and `password`. After 20 failed attempts from one address in a minute, that address gets `429` until its oldest failure is a minute old; a successful sign-in never counts and clears its address's failures. Behind the bundled [HTTPS proxy](INSTALLATION.md#https-and-microphone-access-over-lan) the address is the browser's, which the proxy forwards in `X-Forwarded-For`; that header counts only from the proxy. The manager logs each failed sign-in and each lockout with the address, never the name or password tried. |
| POST | `/api/logout` | Sign out. |
| GET | `/api/settings` | Defaults and supported features, without credentials, with the manager's `version`, `architecture`, the architecture of the images the Docker host runs as Docker names it, such as `amd64` or `arm64`, and the `templates_url` the [templates page](#templates) reads, `null` when it is turned off, and the `volume_filter` label a volume needs to be listed, `null` when every volume is, and `root_mcp_enabled`, whether the root `/mcp` answers (`ROOT_MCP_ENABLED`); the [MCP endpoints](#desktop-mcp-servers-and-endpoints) answer either way. Before 0.6.3 the field was `mcp_enabled`. The container features the installation allows are `gpu_enabled`, `internet_enabled`, `sudo_enabled`, `audio_enabled`, `microphone_enabled`, `gamepad_enabled` and `vpn_enabled`, `proxy` is the installation's proxy, `{"url", "username", "bypass"}` without its password or `null`, and `defaults` holds what a new container gets of each, `{"gpu", "internet", "sudo", "audio", "microphone", "gamepad", "vpn"}`; see [creating a container](#create-a-container-and-follow-the-job). `gpu_enabled` is true where the host has a GPU that `wsl_gpu`, `nvidia_compute` or `dri_gpu` allows, and `gpu` says which: `routes`, those GPU containers get (`wsl`, `nvidia`, `dri`), `unused`, each one its setting allows that they do not get with its `setting` and a `reason` (`no_wsl_gpu`, `no_nvidia_gpu`, `no_toolkit`, `no_render_device`, `beside_wsl` or `unseen`), and `found`, what the host showed when the manager started (`wsl`, `nvidia`, `toolkit`, `null` when Docker was not asked, and `render`, the vendors of its render devices), `null` where the manager cannot see the host; see [GPU on a Linux host](INSTALLATION.md#gpu-on-a-linux-host). |
| GET | `/api/configuration` | The installation settings the manager runs with, for the dashboard's **Management** page; secrets only say whether they are set. See [configuration](#configuration). |
| POST | `/api/configuration` | Check a change of the editable settings, or have the WSL package write it to `.env` and recreate the manager; returns a job. See [configuration](#configuration). |
| GET | `/api/system` | Docker connection and host summary. |
| GET | `/api/update` | This manager's release or `edge` commit, the newest release, a newer `edge` build for a manager that follows it and whether this installation can update; see [updates](#updates). |
| POST | `/api/update` | Ask the WSL package to run `thoric update`; returns a job. See [updates](#updates). |
| POST | `/api/update/check` | Ask GitHub for the newest release now; answers as `GET /api/update` does. See [updates](#updates). |
| POST | `/api/settings/notify/test` | Send a test notice to every address of `NOTIFY_URL` and say how each answered; `409` without one. See [notifications](#notifications). |
| GET | `/api/backup` | Download a [backup](#backup-and-restore) of the whole installation; `images`, `volumes`, `files` and `secrets` choose its optional parts. The transfer is a `backup` job. |
| POST | `/api/backup/check` | Compare a backup's manifest with this installation: the names a [restore](#backup-and-restore) would take that are in use, and the volumes it would lack. |
| POST | `/api/backup/restore` | Stream a backup with optional `size` and restore it; returns a `restore` job once the whole backup has arrived. |
| GET | `/api/usage` | CPU, memory, disk and network use of every container once a second for the last 30 minutes, with the host's GPU; `since` returns only newer samples. See [resource usage](#resource-usage). |
| GET | `/api/containers` | Managed containers, state and health, with `health_detail`, why an unhealthy desktop is; see [desktop health](#desktop-health). |
| POST | `/api/containers` | Create a container, with values for its image's [compose](#composes) in `parameters`; returns a job. `mcp_endpoint: false` leaves out its own [MCP endpoint](#desktop-mcp-servers-and-endpoints). |
| GET | `/api/containers/{id}` | One container's configuration, state and health. |
| POST | `/api/containers/{id}/action` | Start, stop or restart; returns a job, which a start or restart ends once the [desktop is ready](#desktop-readiness). |
| POST | `/api/containers/{id}/retry` | Retry partial setup or a compose setup that did not finish; returns a job, which ends once the [desktop is ready](#desktop-readiness). |
| POST | `/api/containers/{id}/internet` | Set internet egress; returns a job. |
| POST | `/api/containers/{id}/settings` | Change settings after creation; `dry_run` reports what recreates the container. Returns a job. |
| DELETE | `/api/containers/{id}` | Delete the container and networks; returns a job. `?delete_volumes=true` also deletes the volumes created for it. |
| POST | `/api/containers/{id}/exec` | Run a script; returns a job. |
| POST | `/api/containers/{id}/snapshot` | Save filesystem changes as an image; returns a job. |
| POST | `/api/containers/{id}/duplicate` | Make a new container with the container's files and settings under a new `name` ([duplicate](#duplicate-a-container)); returns a job. |
| GET | `/api/scale-sets` | The [scale sets](#scalable-containers) with their instances. |
| POST | `/api/scale-sets` | Make an existing container scalable: `container_id` becomes instance 1 of a new set; `scaling` as below, `mcp_endpoint: false` leaves out the set's endpoint. |
| GET | `/api/scale-sets/{name}` | A set with its instances and `slots`, the template's volumes with their policies. |
| PUT | `/api/scale-sets/{name}` | Change `min`, `max`, `idle_delete_minutes`, `volumes` and `servers` of a set; the template stays. |
| DELETE | `/api/scale-sets/{name}` | Delete a set with its instances, the volumes made for them and its endpoint; returns a job. The set answers `deleting` until the job ends, and nothing creates, deletes or changes its instances meanwhile. |
| POST | `/api/scale-sets/{name}/instances` | Create an instance, with a `note` and `volumes` that override the set's policy for it; returns a job with the instance's number. |
| DELETE | `/api/scale-sets/{name}/instances/{number}` | Delete an instance with the volumes made for it; returns a job. Refused below the set's minimum. |
| GET | `/api/containers/{id}/logs` | Bounded container log output. |
| GET | `/api/containers/{id}/diagnostics` | Desktop and graphics diagnostics. |
| GET | `/api/containers/{id}/usage` | One container's [resource usage](#resource-usage) samples; accepts `since`. |
| GET | `/api/containers/{id}/screenshot` | JPEG at desktop resolution; accepts `max_width`, `region` and `window_id`. |
| POST | `/api/containers/{id}/input` | Mouse/keyboard actions through VNC. |
| POST | `/api/containers/{id}/act-and-observe` | Input actions followed by a screenshot, as JSON. |
| POST | `/api/containers/{id}/screenshots` | Save a screenshot inside the desktop and return its `download_url`. |
| POST | `/api/containers/{id}/find-text` | [Text on the screen](#text-on-the-screen), read with OCR, with boxes in desktop pixels. |
| GET | `/api/containers/{id}/applications` | Launched applications and their state. |
| POST | `/api/containers/{id}/applications` | Launch an application in the background. |
| GET | `/api/containers/{id}/applications/{application_id}` | One application's state, pid and exit status. |
| GET | `/api/containers/{id}/applications/{application_id}/logs` | Tail of the application's log. |
| POST | `/api/containers/{id}/applications/{application_id}/stop` | Stop the application. |
| GET | `/api/containers/{id}/applications/{application_id}/diagnostics` | Process tree, resources and latest log lines. |
| GET | `/api/containers/{id}/applications/{application_id}/metrics` | CPU and memory sampled over `duration_ms`. |
| GET | `/api/containers/{id}/processes` | Every process of the desktop with its CPU and memory use. |
| POST | `/api/containers/{id}/processes/{pid}/kill` | Signal a process, and with `tree` its descendants. |
| POST | `/api/containers/{id}/wait` | Wait for a window, application state, log text, a stable or changed screen, or text on it. |
| POST | `/api/containers/{id}/reset` | Recreate the container from its source image; returns a job, which ends once the [desktop is ready](#desktop-readiness). |
| GET | `/api/containers/{id}/recordings` | Recordings and their state. |
| POST | `/api/containers/{id}/recordings` | Start recording desktop frames. |
| POST | `/api/containers/{id}/recordings/{recording_id}/stop` | Stop a recording and encode its video. |
| POST | `/api/containers/{id}/artifacts` | Package recordings and application logs as a tar.gz. |
| POST | `/api/containers/{id}/audio-capture` | Record desktop or microphone audio as WAV. |
| POST | `/api/containers/{id}/audio-inject` | Play WAV data into the microphone or speakers. |
| GET | `/api/containers/{id}/vnc` | The desktop's screen as a WebSocket of the dashboard session, RFB in binary frames; see below the table. |
| GET | `/api/containers/{id}/audio/{direction}` | The desktop's sound (`output`) or its microphone (`input`) as a WebSocket of the dashboard session; see below the table. |
| GET | `/api/containers/{id}/clipboard` | The text on the desktop's [clipboard](#clipboard), or with `selection=primary` the text last selected. |
| PUT | `/api/containers/{id}/clipboard` | Put text on the desktop's clipboard, and paste it with the keyboard when asked. |
| GET | `/api/containers/{id}/terminals` | Terminal sessions and their state. |
| POST | `/api/containers/{id}/terminals` | Open an interactive shell. |
| POST | `/api/containers/{id}/terminals/{terminal_id}/input` | Type into a terminal. |
| GET | `/api/containers/{id}/terminals/{terminal_id}/output` | Read terminal output from an offset. |
| POST | `/api/containers/{id}/terminals/{terminal_id}/close` | Hang up a terminal. |
| GET | `/api/containers/{id}/ui` | AT-SPI accessibility tree of applications. |
| POST | `/api/containers/{id}/gamepad` | Feed the virtual controller. |
| GET | `/api/containers/{id}/windows` | Visible X11 windows and the active window. |
| POST | `/api/containers/{id}/windows/{window_id}/focus` | Raise and focus a window. |
| POST | `/api/containers/{id}/windows/{window_id}/geometry` | Move or resize a window. |
| POST | `/api/containers/{id}/files/read` | Read a bounded file slice as text or base64. |
| POST | `/api/containers/{id}/files/write` | Write text or base64 file data atomically. |
| POST | `/api/containers/{id}/files/list` | List a directory without following symlinks. |
| POST | `/api/containers/{id}/files/search` | [Search a folder's files](#file-search) by content, by name or both, within bounds. |
| POST | `/api/containers/{id}/files/mkdir` | Create a folder with `path` in an existing parent. |
| POST | `/api/containers/{id}/files/move` | Rename or move within the same volume. |
| POST | `/api/containers/{id}/files/delete` | Delete a file, symlink or directory. |
| GET | `/api/containers/{id}/files/download` | Stream a file with `path`, limited by `UPLOAD_MAX_MB`, or a folder as a `.tar.gz` (`folder-download` job). |
| GET | `/api/containers/{id}/changes` | [What changed in the container's own files](#changed-files) since its Docker container was created, with `path`, `kinds`, `all` and `limit`. |
| POST | `/api/containers/{id}/files` | Stream a binary upload. |
| GET | `/api/images` | Every [image](#images): the catalog images first, then script builds, snapshots and imports, with ancestry and availability. A script build carries `build` with its `base_image`, `internet` and, when [merged from templates](#merged-templates), its `parts` without their scripts, a compose its parameters without the scripts, an [icon](#image-icons) its version and an image built from templates their ids in `template_ids`. `mcp_servers` names the [desktop MCP servers](#desktop-mcp-servers-and-endpoints) its builds declared, and `base_compose_missing: true` marks a build that lacks steps of the [compose of the image it was built on](#a-base-images-compose). |
| POST | `/api/images` | Import a desktop-compatible local image using `name` and `reference`. |
| DELETE | `/api/images/{id}` | [Delete an image](#delete-an-image): a saved image's record with its Docker image, or a catalog image's download. `id` is the list's `id`. Answers at once with `{"id", "name", "deleted": true, "image_removed"}`; `409` while a container was created from it. |
| POST | `/api/images/build` | Build an image using `name`, `base_image`, `script` or [`parts`](#merged-templates) and optional `description`, `internet`, `compose`, `inherit_compose` ([a base image's compose](#a-base-images-compose)), `icon`, `template_id` or `template_ids` and `mcp`, the [desktop MCP servers](#desktop-mcp-servers-and-endpoints) the image declares; returns a job. |
| POST | `/api/images/{id}/pull` | Download a catalog image, or a newer release of it; `id` is its catalog ID. A release manager downloads the one published with it. The image the catalog reference named before is removed once nothing needs it, by the rules of `DELETE /api/images/{id}`. Returns a job. |
| POST | `/api/images/{id}/build` | Build a catalog image from the bundled Docker context instead; returns a job. |
| GET | `/api/images/{id}/layers` | What an image's [disk space](#image-sizes-and-layers) is made of: its sizes and the steps of its history, newest first, each with its size, its instruction and the image it came from. `id` is the list's `id`; `404` when Docker does not have the image. |
| GET | `/api/images/{id}/config` | The `name`, `base_image`, `internet` and `script` or `parts` a script build ran, with the image's `description`, `compose`, `icon` and `template_id`, or `template_ids` for several, when it has them, and `inherit_compose: false` for a build that left its base's compose out, a body `/api/images/build` accepts as it is. `id` is the record's `id`, not its Docker `image_id`. `404` for snapshots, imports and builds from earlier releases. |
| GET, PUT, DELETE | `/api/images/{id}/compose` | Read, set or remove an image's [compose](#composes). `id` is the record's `id`. |
| POST | `/api/images/{id}/compose/base` | Give an image built on a saved image the steps of that image's compose it lacks, before its own, without building it again ([a base image's compose](#a-base-images-compose)). `409` when there are none. |
| GET, PUT, DELETE | `/api/images/{id}/icon` | Read, set or remove a saved image's [icon](#image-icons). |
| GET | `/api/images/{id}/archive` | Download a saved image as a gzip-compressed `docker save` archive; the transfer is recorded as a job. |
| POST | `/api/images/import` | Stream a `.tar.gz` or `.tar` image archive with optional `size`, `name` and `description`; returns a job. |
| GET | `/api/volumes` | Docker named volumes that carry the `VOLUME_FILTER` label (by default the ones Thoric created), each as `docker volume ls` describes it plus a boolean `manager` and an `owner`. A route or container setting that names a volume the filter leaves out answers as if it did not exist, except for a volume a container already mounts. `manager: true` marks the manager's own volumes, such as its data volume: containers cannot mount them and their files cannot be opened. Without `MANAGER_CONTAINER`, or when it names no manager container, none is marked. `owner` is set on a volume a [`new_volume` parameter](#composes) created for a container, from its labels: `{"container_id": ..., "parameter": "WORKSPACE", "name": "Unity dev", "deleted": false}`, with `name: null` and `deleted: true` once that container's record is gone. It is `null` on every other volume. |
| POST | `/api/volumes` | Create a named volume using `name`, with the manager's label and the one `VOLUME_FILTER` asks for. |
| DELETE | `/api/volumes/{name}` | Delete a named volume and its files at once, without a job; returns `{"name": ..., "deleted": true}`. `403` for the manager's own volumes, `404` for a missing one or one `VOLUME_FILTER` leaves out, and `409` while a container record mounts it, through its volume settings or its compose, with that container's name in `error`, or while Docker reports another container using it. Nothing is forced or pruned. |
| POST | `/api/volumes/{name}/files/list` | List a folder of a volume; see [volume files](#volume-files). |
| POST | `/api/volumes/{name}/files/read` | Read a bounded file slice as text or base64. |
| POST | `/api/volumes/{name}/files/write` | Write text or base64 file data atomically. |
| POST | `/api/volumes/{name}/files/mkdir` | Create a folder. |
| POST | `/api/volumes/{name}/files/move` | Rename or move within the volume. |
| POST | `/api/volumes/{name}/files/delete` | Delete a file, symlink or folder. |
| GET | `/api/volumes/{name}/files/download` | Stream a file with `path`, limited by `UPLOAD_MAX_MB`, or a folder as a `.tar.gz` (`folder-download` job). |
| POST | `/api/volumes/{name}/files` | Stream a binary upload. |
| GET | `/api/volumes/{name}/archive` | Download a volume's files as a `.tar.gz`; the transfer is recorded as a job. |
| POST | `/api/volumes/import` | Stream a `.tar.gz` or `.tar` with `name` and optional `size` into a new volume; returns a job. |
| DELETE | `/api/volumes/import` | Delete the volume called `import`, as `DELETE /api/volumes/{name}` would, since this path wins over `{name}`. |
| POST | `/api/volumes/{name}/extract` | Stream a `.tar.gz` or `.tar` with `path` and optional `size` into a new folder of the volume; returns a job. |
| POST | `/api/volumes/{name}/copy` | Copy the volume's files into a new volume named by `name`; returns a job. |
| Any | `/api/transfers/{token}` | Run the one upload, download, import or export a [transfer link](#transfer-links) authorizes, `GET` for a download or export and `PUT` or `POST` for an upload or import, `405` for another method; needs no session. |
| Any | `/api/containers/{id}/ports/{port}/{path}` | HTTP or WebSocket to a [port inside the desktop](#desktop-ports) that its `ports` setting lists; needs the MCP token, not a session. |
| GET | `/api/mcp/servers` | Every container's [desktop MCP servers](#desktop-mcp-servers-and-endpoints) as last seen, in `items`. |
| GET, POST | `/api/containers/{id}/mcp/servers` | Read a running desktop's servers again and list them, or register one of source `container` and read its tools. |
| DELETE | `/api/containers/{id}/mcp/servers/{name}` | Remove a server the desktop user registered; `409` for one the image declares. |
| POST | `/api/containers/{id}/mcp/servers/{name}/refresh` | Start the server again and read its tools. |
| GET | `/api/mcp/scopes` | The scopes an endpoint can grant, with their tools, and the `default` a container's own endpoint gets. |
| GET, POST | `/api/mcp/endpoints` | List the MCP endpoints, or create one from `name` and `grants`; creating returns its `token` once. |
| PUT, DELETE | `/api/mcp/endpoints/{name}` | Replace an endpoint's `grants`, or delete it and end its sessions. |
| POST, DELETE | `/api/mcp/endpoints/{name}/token` | Create or replace an endpoint's token, returned once, or revoke it. |
| GET | `/api/mcp/endpoints/{name}/tools` | The tools the endpoint offers, as its agents see them. |
| GET | `/api/mcp/inspect` | The [MCP inspector](#the-mcp-inspector)'s WebSocket, which joins the dashboard to one desktop server, endpoint or `/mcp`. |
| GET | `/api/skills` | The [skills](#skills) Thoric serves its agents: each one's `name`, `description`, `url`, `digest` and `size`, and the MCP `servers` that offer it, `root` for `/mcp` and `set` for a scale set's endpoints. |
| Any | `/mcp` | The root MCP server of [the MCP guide](MCP.md): Streamable HTTP with the MCP token, not a session; `404` unless `ROOT_MCP_ENABLED`. |
| Any | `/mcp/{name}` | An [MCP endpoint](#desktop-mcp-servers-and-endpoints): Streamable HTTP with its own token or the MCP token, not a session. |
| GET | `/api/jobs` | Latest 100 job summaries. The manager keeps the newest 500 finished jobs and removes older ones; a running job is kept until it ends. |
| GET | `/api/jobs/{job_id}` | Job status, log and result. Optional `wait_seconds`, `log_tail_bytes` and `log_offset` [wait for it and read part of its log](#follow-a-job). |
| POST | `/api/jobs/{job_id}/retry` | Start a [failed or interrupted job again](#retry-a-job) with the request it was started with; returns the new `job_id`. |
| POST | `/api/jobs/{job_id}/cancel` | [Cancel](#cancel-a-job) a queued job, or stop a running script, build or image download. |
| GET | `/api/jobs/{job_id}/request` | The request a failed or interrupted job would be retried with, as `{"kind", "request"}`. |
| GET | `/api/events` | The session's [event stream](#event-stream), `text/event-stream`: jobs as they are queued, start and end, agents' requests for a person, and the lists Docker changes by itself, while the session lasts. |
| GET | `/api/human-requests` | The [requests for a person](#requests-for-a-person) that wait for an answer, oldest first, in `items`. |
| POST | `/api/human-requests/{id}/answer` | Answer a request for a person with `answer`, `done` or `decline`, and an optional `reply`; returns the request. `404` once it is gone, `409` when it was answered already. |
| GET | `/api/activity` | The [agent activity](#agent-activity) log: MCP tool calls and used transfer links, newest first, with filters and a cursor. |
| GET | `/api/dependencies` | Every [dependency](#dependencies) the manager image was built from: interface packages, crates and infrastructure. `404` for a manager built without the list. |
| GET | `/api/openapi.json` | This API as an [OpenAPI 3.1 document](#openapi-document): every route with its parameters, request bodies and what it needs. |
| GET | `/.well-known/agent-skills/index.json` | The [skills](#skills) as the Agent Skills discovery index lists them; does not require sign-in. |
| GET | `/.well-known/agent-skills/{name}/SKILL.md` | One skill's `SKILL.md`, `text/markdown`; does not require sign-in. |
| GET | `/healthz` | Liveness: `ok` while the manager answers requests, without asking the database or Docker; does not require sign-in. The manager image's `HEALTHCHECK` and the WSL package's wait at start use it. |
| GET | `/readyz` | Readiness: `200` with `{"ready": true, "database": true, "docker": true}` when the database answers a query and Docker answers its Engine socket's ping (or `docker version` where the manager has no socket), `503` with the failed check `false` otherwise. Each check gives up after two seconds, so a probe never hangs; the answer never says why a check failed. Does not require sign-in. |
| GET | `/metrics` | Prometheus metrics in the text format 0.0.4 with `METRICS_TOKEN` as a bearer token, never the session; `404` while it is not set, `401` without it and `429` for an address that sent 20 wrong ones in a minute. See [Prometheus metrics](#prometheus-metrics). |

The VNC and media WebSocket routes are `/api/containers/{id}/vnc` and `/api/containers/{id}/audio/{direction}`, where direction is `output` or `input`. They use the browser session and require a matching Origin. VNC uses the binary subprotocol: RFB travels in binary frames, and a text frame is the viewer's own message to the manager, `{"fit":true}` while it fits the desktop's screen to its window with an RFB `SetDesktopSize` and `{"fit":false}` once it no longer does; any other text ends the connection. Once no connection fits the screen any more, because the last one sent `{"fit":false}` or closed, the manager gives the desktop the resolution its settings name again, five seconds after a close so a reload that fits the screen again keeps its size. Most automation clients can use screenshots and `/input` instead of implementing a VNC viewer.

## Create a container and follow the job

```bash
curl --cookie thoric.cookies \
  --header 'Content-Type: application/json' \
  --header 'X-Thoric-Request: 1' \
  --data '{"name":"Agent desktop","image":"ghcr.io/co3moz/thoric/ubuntu-xfce:24.04","gpu":true,"internet":true}' \
  http://localhost:8080/api/containers
```

The response contains `job_id`. Poll its detail endpoint:

```bash
curl --cookie thoric.cookies http://localhost:8080/api/jobs/JOB_ID
```

A job is finished when its status is `completed`, `failed` or `interrupted`. A successful creation puts `container_id` in `result`. A job that makes a container or an image, `create`, `duplicate`, `build`, `snapshot` or `image-import`, carries its `name` from the moment it is queued, so a list can show what is on its way before the record exists; an import sent without a name has none, since its image takes the archive's. Other jobs have no `name`. Read the log for a failed job; after interruption, inspect the resources before retrying. A build, a snapshot and a catalog image's download or build can be [retried](#retry-a-job) with the request they were started with.

`GET /api/containers` returns each container's record, with `id`, `spec` (its creation settings), `container` (the Docker container's name), `source_image_id` and `last_error`, and Docker's view beside those fields: `state` is Docker's container state, `missing` when the container is gone or `unavailable` when Docker cannot be reached; `health` is `healthy`, `unhealthy`, `starting` or `unknown`; `status` is Docker's status text; `vnc_private` says whether the desktop's VNC server stays in its private network, away from the members of its [groups](#container-groups), and is `null` without a Docker container; `in_use` says whether something uses its running desktop now, as [Lifecycle](#lifecycle) counts it, and `idle_stop_at` is when its idle stop comes if nothing uses the desktop from now on, in Unix seconds: a whole idle time ahead while it is in use, also by the MCP call that asks, and `null` without an idle stop or while it is stopped. `GET /api/containers/{id}` returns one item of the same shape.

Creating a container or building an image from a catalog image that is not downloaded yet pulls it first, in the same job. While the download runs, the job carries `progress`: `current` and `total` compressed bytes, where `total` is `null` until the size of every remaining layer is known, and `layers_done` of `layers`. Layers already on the host count as done and are not part of the byte total. The field is removed when the download ends. A download that fails is tried up to three times in the same job, unless Docker refused it, such as for an unknown image. `current` never goes back: a layer whose download Docker resumes or starts over keeps the bytes it had, and a layer a failed try finished stays in the total.

A script build (`POST /api/images/build`) also lists its `stages`, in order, once it starts: `base` prepares the base image, downloading it if needed; `script` runs the setup script; `layer` saves the layer the script wrote; `image` writes and tags the image; and `cleanup` removes the build's temporary files and tag. Each stage is `{"name", "started_at", "finished_at"}` in Unix seconds, and a stage that has not started carries only its name. The job log has a line such as `Stage 3/5: Saving the image layer` when one starts. Stages only move forward: a stage that never started before a later one was skipped, and in a failed or interrupted job the last stage that started is where it stopped. `progress` belongs to the running stage: `base` has one while its download runs, and `script` while the scripts run, as [script progress](#script-progress) describes. Docker reports no measurable progress while it saves the layer, so `layer` has only its times; saving a large layer can take minutes. The `layer` and `image` stages start from BuildKit's own progress lines; with a builder that does not print them, `layer` is skipped and its time counts toward `script`.

A snapshot (`POST /api/containers/{id}/snapshot`) and a settings change that recreates its container ([below](#change-settings-after-creation)) list their stages the same way. A snapshot has `check`, which confirms the container is the one the manager created; `commit`, which commits the container layer to an image while Docker briefly pauses the desktop; and `record`, which saves the image record. A recreating settings change has `stop`, skipped when the container was not running; `commit`; `create`, which creates the new container; `start`, also skipped when the container was not running, which starts the new container and waits until its [desktop is ready](#desktop-readiness); and `cleanup`, which removes the previous container and the image an earlier settings change committed. A [duplicate](#duplicate-a-container) has a snapshot's `check`, `commit` and `record`, then `volumes`, which copies the volumes created for the original and is skipped without any, `create`, which saves the new container's record and creates it, and `start`, skipped when it is left stopped, which waits until its desktop is ready. A settings change that applies in place has no stages. `docker commit` reports nothing until it is done, so `commit` has only its times, and committing a large container layer can take minutes. Other jobs have no `stages`.

Creation accepts `name`, `image` and these optional fields:

| Field | Default | Meaning |
| --- | --- | --- |
| `resolution` | `1920x1080` | Desktop size, from 320x240 to 7680x4320. |
| `gpu` | `GPU_DEFAULT`, `true` | Request GPU access supported by the installation. |
| `internet` | `INTERNET_DEFAULT`, `true` | Attach the Docker egress network. |
| `audio`, `microphone` | `AUDIO_DEFAULT` and `MICROPHONE_DEFAULT`, `false` | Enable supported browser media; microphone also needs audio, and is left off by default without it. |
| `sudo` | `SUDO_DEFAULT`, `true` | Allow the desktop user to use sudo. |
| `gamepad` | `GAMEPAD_DEFAULT`, `false` | Attach `/dev/uinput` so the gamepad route can create a virtual controller. |
| `vpn` | `VPN_DEFAULT`, `false` | Give the desktop `NET_ADMIN`, `/dev/net/tun` and `net.ipv4.conf.all.src_valid_mark=1`, so a VPN client inside it, which Thoric does not install, can open a tunnel in the container's own network namespace. See [VPN](USAGE.md#vpn). |
| `debugger` | `DEBUGGER_DEFAULT`, `false` | Give the desktop `SYS_PTRACE`, so a debugger run as root attaches to its running programs. |
| `realtime` | `REALTIME_DEFAULT`, `false` | Let the desktop's programs take real-time priorities up to 95 and nice values down to -20, and lock memory without a limit (`--ulimit rtprio=95`, `nice=40`, `memlock=-1`). |
| `profiler` | `PROFILER_DEFAULT`, `false` | Give the desktop `PERFMON`, so `perf` reads performance counters. |
| `kvm` | `KVM_DEFAULT`, `false` | Attach the host's `/dev/kvm`, for virtual machines and emulators. Refused while the host has none, `kvm_host` being `false` in `GET /api/settings`. |
| `proxy` | `{"mode": "installation"}` while `PROXY_URL` is set, otherwise `null` | The proxy the desktop's programs go through. `{"mode": "installation"}` follows the installation's, which `GET /api/settings` reports in `proxy`, and is refused while it has none. `{"mode": "custom", "url": "http://proxy.example:3128", "username": "name", "bypass": ["intranet.example", "10.0.0.0/8"]}` is one of the container's own: an `http`, `https` or `socks5` address with a host and a port and nothing else, a user name of at most 255 bytes without a colon, and at most 64 hosts, domains (`.example.com`, `*.example.com`), addresses or networks that skip it. `null` is none. See [Proxies](#proxies). |
| `proxy_password` | None | The password of the container's own proxy, at most 255 bytes. It is kept in a record of its own, never in the container's, and no route returns it; the container's `proxy` says only `"password": true`. |
| `hostname` | The Docker container name | The desktop's [machine identity](#machine-identity): one label of letters, digits and hyphens, at most 63 characters. |
| `mac_address` | Random | A unicast MAC address such as `02:42:ac:11:00:02`. Capitals and hyphens are accepted and stored as lowercase with colons. |
| `machine_id` | Random | 32 hex digits, written to `/etc/machine-id` and `/var/lib/dbus/machine-id`. A UUID with hyphens is accepted. |
| `ports` | `[]` | Ports inside the desktop that the [port proxy](#desktop-ports) forwards to, at most 32, from 1 to 65535. |
| `groups` | `[]` | The [groups](#container-groups) the container belongs to, at most 8 names of 1 to 63 lowercase letters, digits and hyphens, without a hyphen at either end. |
| `aliases` | `[]` | More names the members of its groups reach it by, at most 8, of the same form. |
| `cpus`, `memory_mb` | No limit | Positive CPU count and at least 256 MiB of memory. |
| `shm_mb` | `512` | Shared memory size, at least 64 MiB. |
| `tmp_mb` | `512` | Size of `/tmp`, at least 64 MiB. It is a tmpfs, so its files are kept in memory and count against `memory_mb`; large or growing files such as logs belong in a volume or the home folder. |
| `mounts` | `[]` | Objects with `volume`, absolute `target` and optional `readonly`. The manager's own volumes, such as its data volume, are refused, here and whenever the container is created again. |
| `parameters` | `{}` | Values for the [compose](#composes) parameters of the image, by name, or as `STEP.NAME` for a [compose of several steps](#steps). |
| `lifecycle` | `{"idle_stop_minutes": IDLE_STOP_DEFAULT}` | When Thoric stops or deletes the container by itself, with `idle_stop_minutes`, `expires_at` or `expires_in_minutes`, `on_expiry` and `delete_volumes`. See [Lifecycle](#lifecycle). |

A feature a request leaves out gets the installation's default, which `GET /api/settings` reports in `defaults`, and never one the installation switched off. A feature the installation switched off, `gpu_enabled`, `internet_enabled`, `sudo_enabled`, `audio_enabled`, `microphone_enabled`, `gamepad_enabled`, `vpn_enabled`, `debugger_enabled`, `realtime_enabled`, `profiler_enabled` or `kvm_enabled` being `false`, is refused when a request asks for it, here, when a [settings change](#change-settings-after-creation) turns it on and on `POST /api/containers/{id}/internet`. A container that has one keeps it in its settings, also when it is created again; while the feature is off the manager refuses only what it carries itself, the audio and microphone channels and gamepad input. What each permission opens is in [the installation guide](INSTALLATION.md#desktop-permissions). `kvm_enabled` is also `false` where the Docker host has no `/dev/kvm`, `kvm_host` being `false`, and `null` there when the manager cannot see the host. GPU access is the exception: while `gpu_enabled` is false, a reset, a retry or a settings change that creates a GPU container again fails with the GPU settings named, since its desktop would stop at the renderer check. Before 0.6.3 a request without `gpu` got none; now it gets GPU access where the installation has it, as the create form always chose.

### Desktop readiness

A job that starts a desktop ends only once the desktop is ready: a creation, a start or restart (`POST /api/containers/{id}/action`), a retry, a reset and a [settings change](#change-settings-after-creation) that creates the container again. Ready means that the desktop session has written `/run/desktop/ready`, which every Thoric desktop image does once its window manager runs and which its health check reads too. The routes still answer with the job at once; through 0.9.0 these jobs ended as soon as Docker had started the container.

The job fails when the desktop stops before it is ready, when Docker restarts it three times while the job waits, or when it is not ready within five minutes. Its error names what happened, the code the desktop last exited with and the last 20 lines the container logged, such as the renderer check of a GPU desktop that found no GPU. A creation, a retry or a reset leaves the container as Docker has it, which may still be restarting, and keeps the error with the job, so a desktop that comes up later shows none. Only when its [compose setup](#composes) was due does the container also keep the error in `last_error`, as `The compose setup did not run: ...`, since that setup did not run; **Retry setup** runs it. A creation that failed once the container was saved still gives it its own MCP endpoint, unless `mcp_endpoint` is `false`. A settings change restores the previous container instead. An image that replaces the desktop session and never writes the file fails these jobs after five minutes.

A [settings change](#change-settings-after-creation) of the resolution leaves the container with the resolution Docker created it with, which its desktop starts at. Once such a desktop is ready, the job that started it gives it the resolution its settings name through its VNC server, and its log says so; when Docker starts the desktop by itself, such as after the host restarted, the manager does the same. A screen that no longer has the size the desktop started with, such as one a viewer resized to their window, is left as it is until no viewer fits it any more ([VNC](#endpoints)). A desktop that cannot take the resolution keeps its own, which the job log or the manager's log says, and the job does not fail.

While it waits, the job shares the container's lock with input, file access, scripts and the other work that only uses a running desktop, as a script does, so the desktop can be used as soon as it is up; a stop, a reset or another change of the container waits for the job. A creation, a retry or a reset whose [compose setup](#composes) runs next, and a settings change, which may still have to restore the previous container, hold the container alone until they end.

### Desktop health

Docker runs a desktop's health check every 10 seconds: its session has written `/run/desktop/ready`, its X display answers within 3 seconds, below Docker's limit of 4, and a GNOME session has not fallen back to its failure screen. After three failures in a row Docker counts the desktop as unhealthy, which the list reports as `"health": "unhealthy"` and the dashboard as **Desktop error**. `health_detail` is then the first line of the newest check's output, at most 300 bytes, and `null` otherwise. A desktop image newer than Thoric 0.9.3 begins it with what failed: `desktop-starting`, `display-held`, a display that takes the connection and does not answer, `display-gone`, a display that is not running, or `session-failed`. An older image's check that does not finish says Docker's `Health check exceeded timeout (4s)`.

A program can hold the display with a server grab, as ImageMagick's `import` does while it waits for a window to be clicked, which it falls back to when the window it was given has closed. The display then serves that program alone, and every other one that connects waits without an error. A window, screenshot, OCR or clipboard route whose command on the display has not finished after 3 seconds has the display looked at meanwhile, as the desktop user: one X connection setup within 3 seconds, and when it gets no answer, the programs connected to the display, read from the kernel's socket list and `/proc`. A `move_relative` of `/input` is looked after the same way and is then sent as positions, with the reason in `relative_motion`. A display that does not answer ends the request with `409`, whose message names the newest three programs connected to it that it does not keep waiting, the `application_id` of those [launched](#applications-and-windows) through the manager and how long ago each started, and how to let the display go: stop the program, or click once on the desktop when it waits for a click, which `/input` sends through VNC even while the display is held. A display that is not running answers `409` too. One that answers lets the request finish as it would have, and a check that fails changes nothing.

### Container groups

Containers that list the same group share an internal Docker network, `thoric-group-NAME`, labeled `thoric.group=NAME`, which has neither the internet nor the manager on it, and from Docker Engine 28.0 no address on the Docker host, as a container's private network has none. Each member reaches the others there by hostname, by name in DNS form (lowercase, Latin letters without their marks, a hyphen for anything else, as `Geliştirme 2` becomes `gelistirme-2`) and by their `aliases`. A group is only a name in its members' settings: the network is created when the first member joins and removed once no container lists the group, also when its last member is deleted. A group or alias change, and a new name, apply in place: the container leaves and joins networks, and an endpoint whose names changed is connected again. A name another member of a shared group answers to already is refused with `400`, since the group's DNS would answer it with either container. A desktop with a [proxy](#proxies) reaches the members of its groups without it.

Members reach every port the others listen on, so a group suits containers that are meant to trust each other. The VNC server of a desktop listens only in its private network, which its image marks with the label `thoric.vnc=private`. A desktop from an older image shows its screen to the members of its groups without a password: the container list says so with `"vnc_private": false`, and the dashboard warns. Rebuilding the image from a newer base, or creating the container again from one, ends that.

### Machine identity

`hostname`, `mac_address` and `machine_id` are what software inside the desktop, such as a license bound to one machine, sees of the machine it runs on. Docker gives every container it creates a new hostname and new MAC addresses, so the manager keeps all three in the container's `spec` and gives them to every Docker container it creates for it: a reset, a retry and a settings change that recreates the container keep them. A new container gets its own, also one started from a snapshot of another container. Leave them out unless one container is meant to take over the identity of another it replaces.

- The hostname is passed to `docker create`; `/etc/hostname` and `/etc/hosts` follow it.
- The MAC address is the private network's address from creation. The internet network gets the same address when it is connected, so whichever interface software reads, it sees one address; Docker numbers a container's interfaces in no fixed order. Only the Engine API takes an address for a network connected after creation, and Docker Engine 29 (API 1.54) is the first release that uses it. Earlier releases, and an installation whose `DOCKER_HOST` is not a Unix socket, give the internet interface an address of their own at every start.
- The machine ID is written to `/etc/machine-id` and `/var/lib/dbus/machine-id` in the created container before it starts. The catalog images carry one ID that every container of a release shares, and a snapshot carries the ID of the container it was taken from, so neither is used as it is.

Records saved before identities were kept have `null` for all three until a Docker container is next created for them. They then take the hostname of the container being replaced, and its MAC address if it is running, since Docker gives a stopped container its address only when it starts; without such a container they get new ones. Their machine ID stays `null`, which leaves the ID their image or commit carries, as before, until the settings set one.

### Lifecycle

`lifecycle` says when Thoric stops or deletes a container by itself: a desktop nobody uses is stopped, which gives its memory and GPU back, and a container whose time is up is stopped or deleted, so a throwaway desktop an agent made cleans itself up. The policy is the manager's alone: setting or changing it never restarts the desktop, and a container created before it existed has none.

| Field | Default | Meaning |
| --- | --- | --- |
| `idle_stop_minutes` | `IDLE_STOP_DEFAULT`, `0` | Stop the running desktop after this many minutes without use, up to 10080, a week; `0` never. A request that leaves it out gets the installation's, which `GET /api/settings` reports in `defaults.idle_stop_minutes`, copied into the container, so a later change of the setting leaves it as it is. |
| `expires_at` | `null` | When the container expires, in Unix seconds, in the future and at most a year ahead; `null` never. One that has passed stays valid until it is carried out. |
| `expires_in_minutes` | None | Expire this many minutes from now, up to a year, instead of `expires_at`. The record keeps `expires_at`. |
| `on_expiry` | `stop` | `stop` stops the container and clears its expiry, so a start keeps it running; `delete` deletes it as `DELETE /api/containers/{id}` does. |
| `delete_volumes` | `false` | With `on_expiry` `delete`, also delete the volumes created for the container, as `delete_volumes` of a delete does. |

The desktop counts as used, and its idle time starts again, while one of these lasts and when it ends: a job on the container, from its queueing to its end; a request to one of its routes other than a read (`GET` and `HEAD`), such as input, a file, a terminal or a settings change, and a download of its files, until the answer has been sent; a VNC, sound or microphone connection; an [MCP inspector](#the-mcp-inspector) session with one of its desktop servers; an MCP tool call or a [transfer link](#transfer-links) that names it, a link's download until it has been sent; a request to one of its [proxied ports](#desktop-ports), until its answer has been sent or a WebSocket it opened closes. Docker starting the desktop counts too, also when Docker or a person started it, where the manager follows Docker's events, and so does the manager starting. HTTP reads do not count, such as the dashboard's previews, usage charts and container list, `GET /api/containers/{id}` or a screenshot: a page left open repeats them on a timer. An MCP tool call counts whatever it does. A desktop whose processes keep more than a quarter of a CPU core busy on average over two minutes counts as used too, so a build or a training run started by hand inside it is not stopped, where the manager reads [resource usage](#resource-usage) through the Engine API socket; an idle desktop with its VNC server and a browser on a still page uses a few hundredths of one. An open viewer or sound connection keeps a desktop running for as long as it is open.

Every 30 seconds the manager looks at the containers with a policy. It takes a container's lock as a request does, but without waiting: while a job, a request or a settings change holds it, the container waits for the next look. Under the lock it reads the record and Docker's view again, so a policy a settings change moved, or a desktop that became used, is left alone, and the job it starts keeps the lock until it ends. The jobs are the routes' own stop and delete, under kinds that say why, which the job list and the [event stream](#event-stream) show like any other: `idle-stop`, `expiry-stop` and `expiry-delete`, whose result is a delete's. An expiry comes first, whether the container runs or not, and one that stops on a stopped container only clears itself. A job that fails is logged as a warning, and that container is left alone for 15 minutes, so a lasting failure does not fill the job list. A stop or delete that is done sends a [notice](#notifications) of kind `container_lifecycle` when `NOTIFY_EVENTS` names it, which it does not by default; one that fails is a failed job, `job_failed`.

A [restore](#backup-and-restore) keeps each container's policy, except an expiry that passed while it was in the backup: it is cleared, and the restore log says so, so a restored container is never stopped or deleted at once.

## Script progress

The scripts of a build, its script or each of its [parts](#merged-templates), and of a compose setup, each of its [steps](#steps), can say where they are. A line of output that is exactly `THORIC_STAGE=<i>/<n>` or `THORIC_STAGE=<i>/<n> <label>` says the script begins the i-th of its n steps, and `THORIC_SKIP=<i>/<n> <label>` that it leaves the i-th out, such as the step of an option that is off. Numbers go from 1 to 10,000 and a label is one line, cut at 120 characters; any other line is ordinary output. These lines stay in the job log, and a step only moves forward, so a repeated or earlier number changes nothing.

```bash
echo "THORIC_STAGE=1/3 Installing Ghidra"
# ...
if [ "$WINE" = true ]; then
  echo "THORIC_STAGE=2/3 Installing Wine"
  # ...
else
  echo "THORIC_SKIP=2/3 Wine"
fi
echo "THORIC_STAGE=3/3 Cleaning up"
```

A script that prints none of these is still followed: bash reads a short line trace before it, through `BASH_ENV`, which reports each line further down the script reaches as `THORIC_LINE=<line>`. The manager leaves those lines out of the log. Only the script's own top-level commands count, not those inside its functions or subshells, nor another shell it starts, and a script that sets a DEBUG trap of its own ends the trace there. Builds read the trace from a bind mount, so nothing of it stays in the image.

While the scripts run, the job's `progress` is `{"script": {...}}`:

| Field | Meaning |
| --- | --- |
| `part`, `parts` | The part that started last, counted from 1, of the build's parts or the compose's steps. |
| `name` | That part's name, when there are several. |
| `stage`, `stages`, `label`, `skipped` | The step the part named last, `skipped` when it was a `THORIC_SKIP`. |
| `line`, `lines`, `label` | Without a named step, the furthest line the part reached, of the lines its script has, and the command on it; a command continued over several lines is named by its first. |
| `percent` | The share of all parts done, each part weighing the same: a step that began counts the ones before it, a skipped step counts itself and a line counts the lines above it. It never goes back. |

The job also keeps `script`, which stays once the job ends: `{"parts": [...]}`, each part with `name` when there are several, `started_at`, `finished_at` when it ended well, so the part a failed or interrupted job stopped in has none, `cached` when BuildKit reused the layer of an earlier build and the script did not run, `stages`, the count its steps gave, and `steps`, at most 200 of `{"stage", "label", "started_at", "finished_at", "skipped"}`. A skipped step has no times. A number a script jumped over, and the numbers after its last step in a part that ended well, were skipped too and are not listed. Job lists leave `script` out, as they leave the log out; `GET /api/jobs/{job_id}` has it.

## Follow a job

`GET /api/jobs/{job_id}` takes three optional query parameters. Without them it answers with the whole job and its whole log, at most the last 256 KiB.

- `wait_seconds`, at most 300, waits for a queued or running job to end before answering, and answers as soon as it does. Once the time is up, or the manager stops, it answers with the job as it is then, so ask again to wait longer instead of polling in a loop.
- `log_tail_bytes`, at most 262144, returns only the last that many bytes of the log.
- `log_offset` returns the log from that byte of the job's whole output, counted from its first byte also after the log dropped its oldest part. When a new manager takes over an update or a settings change and starts its log again, the count goes on from where the log ended, so a reader that asks from its last `log_end` gets the restart notice and what follows.

With either log parameter the answer adds `log_start`, where the returned log begins in the whole output, and `log_end`, where it ends. Pass the `log_end` of one answer as the next `log_offset` to read only what was added since; a `log_start` past the `log_offset` asked for means bytes were left out, older than the log keeps or before the tail.

```bash
curl --cookie thoric.cookies 'http://localhost:8080/api/jobs/JOB_ID?wait_seconds=60&log_tail_bytes=4096'
```

## Cancel a job

`POST /api/jobs/{job_id}/cancel` cancels a queued job of any kind, which then never runs, and stops a running one of the kinds that can stop anywhere: a script (`exec`), a script build (`build`) and a catalog image's download (`image-pull`) or source build (`source-build`). Stopping drops the job's Docker commands, and a script is stopped inside its desktop as its time limit stops it, TERM to its processes and KILL five seconds later; a build still cleans up after itself, the image it already tagged included, since its record was never saved. Any other running job changes a container, an image or a volume and is answered with 409, and so is a job that has already ended. The answer is `{"job_id", "cancelled": true}`, and the job ends as `failed` with the error `Cancelled by request` shortly after. A job's detail says `cancellable: true` while a cancel reaches it; lists leave it out. A cancelled build, snapshot or download keeps its request and can be [retried](#retry-a-job).

## Retry a job

A script build (`build`), a snapshot (`snapshot`) and a catalog image's download (`image-pull`) or source build (`source-build`) keep the request they were started with, so a job that failed, such as on a dropped connection to Docker, or that a restart interrupted, can start again without its form being filled in again. `GET /api/jobs/{job_id}` then carries `"retryable": true`, and `POST /api/jobs/{job_id}/retry` sends the request through its route once more: it is checked again, and a new job of the same kind and name answers with its own `job_id`. Each retry is a job of its own, which takes the request over: the job retried is no longer retryable, so a second click, or two retries at once, start one job, and a retry its route refuses leaves the request with the job. `GET /api/jobs/{job_id}/request` returns what a retry would send, `{"kind": "build", "request": {...}}`, where a build's `request` is the body of `POST /api/images/build`, so it can be changed and sent there instead. A snapshot's is `{"container_id", "name", "description"}` and a catalog image's `{"image_id"}`. Both answer `409` for a job that is still running or completed, for one that was retried already, and for one that keeps no request: a container's creation can carry secret values, which are kept nowhere but in its setup record, and an upload's body is gone once it has been read. A request goes with its job when it completes or when the job history is pruned, and backups leave it out with the jobs.

## Change settings after creation

`POST .../settings` takes the creation fields to change, except `image`; fields that are left out keep their value. `name`, `internet`, `ports`, `groups`, `aliases`, a change of a proxy the container has, `proxy_password` and a new or changed `cpus` or `memory_mb` apply to the running container. `lifecycle` is the manager's alone and never touches the container; its fields that a request leaves out keep their values, so `{"lifecycle": {"expires_in_minutes": 60}}` moves the expiry alone. So does `resolution`: the desktop's VNC server changes the screen while applications keep running, as **Resize to window** does, a stopped desktop takes it when it starts, and a desktop gets it again after every restart ([desktop readiness](#desktop-readiness)). `gpu`, `audio`, `microphone`, `sudo`, `gamepad`, `vpn`, turning `proxy` on or off, `hostname`, `mac_address`, `machine_id`, `shm_mb`, `tmp_mb`, `mounts` and removing a limit (`null`) cannot be changed on an existing Docker container: the job stops the container, commits its container layer to an image tagged `thoric/container:JOB_ID`, creates a new container from that image with the new settings and starts it again if it was running. Running applications close; files in the container layer are kept, and named volumes are not touched. So turning `sudo` off removes the desktop user's sudo rule but keeps whatever root changed while it was on, such as a setuid program, another sudoers file or a password for root; a reset clears it. If the new container cannot start, or its desktop is not [ready](#desktop-readiness), the previous one is restored, and the job's error says why. A reset still recreates the container from its original source image, and deleting the container removes the committed image unless a snapshot builds on it.

With `"dry_run": true` the request is validated and answered at once with `live` and `restart`, the changed fields grouped by how they apply, without a job:

```bash
curl --cookie thoric.cookies \n  --header 'Content-Type: application/json' \n  --header 'X-Thoric-Request: 1' \n  --data '{"audio":true,"dry_run":true}' \n  http://localhost:8080/api/containers/CONTAINER_ID/settings
```

A feature the installation has switched off cannot be turned on, and an identity field can be changed but not set back to `null`. The job's result repeats the `live` and `restart` lists.

Start/stop/restart uses `{"action":"start"}`, internet uses `{"enabled":false}` and snapshot uses `{"name":"Editor installed","description":"Optional note"}`. Actions on a container are serialized. Deleting it preserves named volumes and saved images, but removes unsaved changes in its container layer. Its job result is `{"deleted": ID, "name": ..., "volumes_preserved": true, "images_preserved": true}`.

`DELETE /api/containers/{id}?delete_volumes=true` also deletes the volumes that [`new_volume` parameters](#composes) created for this container: those whose labels name it as their owner, found by label and inspected again before anything is removed, never a volume chosen by name or the manager's own. A volume another container mounts, including one that picked it by name, stays, and so does one Docker refuses to remove because something still uses it; the job log says why. That holds for a container being created at the same moment too: a creation holds the locks of the volumes it mounts from its check of them until its record is saved, and the delete reads the records under the same locks, so it either sees that container and keeps the volume, or removes the volume first and the creation stops with the volume not found. The volumes are removed after the container itself, so a volume the job could not remove does not keep the container. The result then adds `volumes_deleted` and `volumes_kept`, the names of the volumes created for the container that were removed and those that stay, and `volumes_preserved` is `false` once one was removed. The option is off by default; the dashboard's delete dialog offers it, unchecked, when the container has such volumes.

## Duplicate a container

`POST /api/containers/{id}/duplicate` with `{"name": "Editor copy"}` makes a new container with the files and settings of the container, running or stopped, in one job of the kind `duplicate`. With `"start": false` the new container is left stopped instead of started, and with `"mcp_endpoint": false` it gets no [MCP endpoint](#desktop-mcp-servers-and-endpoints) of its own, which a new container gets otherwise. The name must be one no other container has, compared without case and without the spaces around it (409), and while a duplicate makes a container of that name, another request for the same name answers 409 too. MCP clients call `duplicate_container`.

- **Files.** The job commits the container's files as a snapshot, `thoric/snapshot:JOB_ID`, which `GET /api/images` and the Images page list like any snapshot, under the new container's name, and the new container starts from it. That is why a reset of the duplicate returns it to the moment it was made, not to the image the original started from, and why the snapshot cannot be [deleted](#delete-an-image) while the duplicate exists; once the duplicate is deleted, so can the snapshot be. As with any snapshot, a running desktop is paused while Docker commits it, and volumes, `/run`, `/tmp` and running memory are not part of it.
- **Settings.** Every setting is copied except the name and the [machine identity](#machine-identity): the duplicate gets a hostname, MAC address and machine ID of its own. Its [lifecycle](#lifecycle) keeps the idle stop and what an expiry does, but not `expires_at`: an expiry is when the original's own task ends, so the duplicate has none until one is set. A setting the installation has switched off since, such as internet access, is switched off as a restore switches it off, and the job log says which. The duplicate joins the original's [groups](#container-groups), and the aliases another member already answers to are left out, the original's among them, since the original stays in the group; the duplicate is reached there by its own hostname and name.
- **Compose.** The duplicate keeps the original's [compose](#composes) and every value it was given, secret ones too, and the password of its own [proxy](#proxies). Both are copied from record to record inside the manager and pass through no route and no log. The setup's work is in the copied files, so a setup that completed does not run again; one that had not completed runs when the duplicate starts, or waits for a retry when it is left stopped. A reset runs it again, as it does for any container made from a snapshot.
- **Volumes.** A volume created for the original by a [`new_volume` parameter](#composes) gets a copy for the duplicate, `thoric-NEW_ID-NAME`, created with the duplicate's labels and filled as a [volume copy](#volume-archives) fills one, in one of the archive transfer slots, which the job waits for. Every other named volume is mounted by both containers. A running desktop may write to a volume while it is copied; stop it first for a copy of one moment.
- **Not copied.** The original's MCP endpoints and their tokens, its activity, applications, terminals and recordings. The desktop MCP servers registered in the desktop user's folder, such as with `register_mcp_server`, are files in the desktop, so the duplicate has them, and its desktop reports them once it runs.

The original is only read. Docker pauses a running desktop while it commits it, so the job holds the original's lock alone for `check`, `commit` and `record`, as a snapshot does, and input, files and scripts wait for it; then it shares the lock, as a script does, until the volumes are copied, so a reset, a settings change or a delete of the original waits for the copy. The name is checked again before the new record is saved. Until the new container stands in Docker, a failure takes back everything the job made: the new container with its networks and records, the volumes it created and the snapshot, which stays only when a container was created from it meanwhile. A desktop that is then not ready, or a setup that fails, leaves the duplicate with its error, as a creation leaves a container, for a retry. The job's [stages](#create-a-container-and-follow-the-job) are `check`, `commit`, `record`, `volumes`, `create` and `start`, and its result is `{"container_id", "name", "duplicated_from", "image_id", "volumes_copied", "volumes_shared", "aliases_left_out", "switched_off", "started"}`, with `mcp_endpoint` when it made one. `image_id` is the snapshot's record id and `volumes_copied` lists `{"from", "to"}` pairs.

## Scalable containers

A container made scalable is instance 1 of a scale set, which creates more instances of it on demand. Create one with `POST /api/containers` and `scaling`, or make an existing one scalable with `POST /api/scale-sets` and its `container_id`:

```json
{"name": "re", "image": "IMAGE", "parameters": {"reverse-engineering.ANALYSIS": "analysis"},
 "scaling": {"min": 1, "max": 4, "idle_delete_minutes": 60,
             "volumes": {"/home/desktop/.config/chrome-agent": "new"},
             "servers": {"ghidra": "first"}}}
```

- **Not a fork.** The set keeps the request as its template: the settings, the image and the compose with the values it was given, secret ones and the proxy's password in a record of their own that no route returns. Every instance is created from it as the first one was, runs the compose setup itself and starts with none of another instance's files: nothing is committed or copied, unlike a [duplicate](#duplicate-a-container). The compose is the one the first instance pinned, so a change of the image's compose later does not reach the set. Each instance has a [machine identity](#machine-identity) of its own and leaves out the [aliases](#container-groups) another member of its groups answers to.
- **Names and numbers.** `scaling.set` names the set, lowercase letters, digits and dashes, at most 40; the container's name made into one when left out. Instances take the lowest number the set has free, from 1 to `max` (at most 16), and are named `SET-NUMBER`. A container made scalable keeps its own name as instance 1. The container list carries `scale`, `{"set", "number"}`, on every instance.
- **Volumes.** `volumes` says, by the folder a volume of the template is mounted at, from `mounts` or a compose volume parameter, what an instance gets: `shared`, the default, mounts the template's volume in every instance; `new` gives each instance an empty volume of its own, `thoric-CONTAINER_ID-PART`, labelled as the [`new_volume` parameter](#composes)'s, which goes with it; `first` keeps the template's volume for instance 1 and gives the others new ones; `first_writes` mounts the template's volume in every instance, to read and write in instance 1 and read-only (`readonly` on the mount, also a compose volume's, which its pinned compose keeps for every recreation) in the others, so they see what instance 1 keeps there and cannot change it. `first_writes` is refused for `/home/desktop` and its hidden folders, which a volume may not be read-only at, and a compose setup that writes into such a folder fails in the instances after the first. `POST /api/scale-sets/{name}/instances` takes `volumes` with `shared` or `new` for that instance alone; under `first_writes`, `shared` stays read-only after instance 1, while `new` gives it a volume of its own to write. A change of `volumes` through `PUT /api/scale-sets/{name}` applies to the instances created afterwards. A shared volume an instance made of its own, such as the first instance's `new_volume` parameter, that is gone when the next instance is created becomes that instance's own, and its job log says so. A set's template keeps its volumes and image: a [volume](#volumes) or an [image](#delete-an-image) a set creates its instances from is not deleted while the set exists.
- **Desktop servers.** `servers` says, by a desktop MCP server's name, where the instances run it: `each`, the default, in every instance, or `first`, in instance 1 alone for all of them, as one program every endpoint and `/mcp` share, for a server that locks what the instances share, such as a Ghidra project on a shared volume. The other instances never start it and list it as `elsewhere`. Unlike `volumes`, a change through `PUT /api/scale-sets/{name}` applies to every instance at once: the connections that ran under the old policy close, and an instance that runs a server again reads its tools. A name no instance declares yet is kept. [The MCP guide](MCP.md#one-server-for-the-whole-set) says what the set's endpoint offers for one.
- **Limits.** `min`, 1 by default, is how many instances the set keeps: its lowest numbers, created with it one after another, which neither an agent nor the idle delete removes. Deleting an instance through its set never goes below `min`; deleting its container with `DELETE /api/containers/{id}` still works, as for any container, and frees its number. `idle_delete_minutes` deletes an instance beyond those, with the volumes made for it, once nothing used it for that long, running or stopped, as the [lifecycle](#lifecycle)'s use counts it; the job kind is `idle-delete` and it sends the `container_lifecycle` notice. An instance's own idle stop and expiry apply as for any container.
- **Jobs.** Creating an instance is a `scale-up` job named after the instance, deleting one through its set `scale-down`, and deleting a set `scale-set-delete`. Its number is taken when the request is answered, so two creations never take one number, and given back when the creation ends without a container. A deletion takes the instance out of the set before its container goes, so two deletions asked for at once never take the set below `min`, and puts it back when the deletion fails. While a set is deleted its record stays, marked `deleting`, so its name stays taken and nothing creates, deletes or changes its instances; a deletion that fails halfway leaves the set with the instances it still has, to be deleted again.
- **Fields.** A set answers its `name`, `created_at`, `image`, `min`, `max`, `idle_delete_minutes`, `volumes` and `servers` (the policies other than the default), `endpoint` and `endpoint_path`, `deleting`, `slots`, the template's volumes with `target`, `volume`, `parameter`, `readonly`, `policy` and `label`, and `instances`, each with its `number`, `container_id`, `name` (null while it is created), `note`, `creating` and `kept`. The `list_instances` tool adds each instance's `state`, `health`, `in_use` and `mounts`, with `sharing` for each, and the set's `free` numbers.
- **MCP.** The set gets an MCP endpoint, `/mcp/SET`, unless `mcp_endpoint` is `false`, and its instances none of their own; see [MCP](MCP.md#scalable-containers).

A [backup](#backup-and-restore) carries the set with its instances, its template and its endpoint, and a restore brings them back together; the template's secret compose values and its proxy's password come along only with `secrets`. An instance whose creation had not saved its record when the backup was made is left out, and its number is free again after a restore.

## Run a script

POST to `/api/containers/{id}/exec`:

```json
{
  "script": "apt-get update\napt-get install -y git",
  "root": true,
  "timeout_seconds": 600
}
```

The default user is UID/GID 1001:1001; root execution must be explicit. Bash runs with fail-on-error and with standard input closed, so a command that reads input receives end-of-file instead of consuming the rest of the script. The response is a job, with the output in its log. Its `result`, also when the script failed, is `{"container_id", "user", "exit_code", "timed_out", "output_tail"}`: the exit code, `null` when a signal or the manager's own limit ended the script, whether it ran out of `timeout_seconds`, and the last 8 KiB of what it printed on stdout and stderr, without the manager's own log lines. A nonzero exit or a timeout fails the job. Timeouts range from 1 to 3500 seconds, defaulting to 300. The working directory is `/home/desktop`, or `/root` for a root script, so nothing it starts imports modules the desktop user left in their home; the container's network and resource limits apply. The script shares the container's lock with input, file access, uploads, terminals, recordings and applications, which go on while it runs, while a start, stop, reset, settings change, snapshot or delete waits for it. The lock is fair, so once such work waits behind a long script, new input, file access, an upload's delivery, terminals, applications, a recording's start and an artifact export wait behind it in turn and answer 409 after five seconds, until the script ends or is [cancelled](#cancel-a-job). Start graphical programs with the [applications API](#applications-and-windows) instead of a script, since the job ends only when its program exits. See [command execution](MCP.md#run-commands-and-install-software) for more detail.

## Images

`GET /api/images` lists every image a container can start from, each with a `kind`:

| Kind | Image |
| --- | --- |
| `catalog` | A desktop published with Thoric. It is downloaded the first time a container or a build starts from it. |
| `build` | Built from a base image with a script through `/api/images/build`. |
| `snapshot` | A container's files saved with `.../snapshot`, or by a [duplicate](#duplicate-a-container) that starts from them. |
| `import` | A desktop image already on the Docker host, registered with `POST /api/images`. |
| `archive` | Loaded from an [image archive](#image-archives). |

The catalog images come first. Their `id` is the catalog ID, such as `ubuntu-xfce`, and a container or a build names one by its `reference`. `image_id` is `null` until the image is downloaded, `downloaded` tells whether Docker has the reference and `available` whether it can be used; `error` explains a downloaded reference whose labels belong to another desktop. `created_at`, `parent_image_id` and `source_container` are `null`. The other kinds are saved records, named by their Docker `image_id`, with `available: false` once Docker no longer has the image. Every kind reports `desktop`, the catalog image it derives from (`catalog_id`) and its session, when its labels say so.

### Delete an image

`DELETE /api/images/{id}` deletes an image, where `id` is the list's `id`. It cannot be undone. What goes depends on the kind:

| Kind | What goes |
| --- | --- |
| `build`, `snapshot`, `archive` | The record, and Docker's image with `docker image rm` of the tag Thoric gave it, or of its `image_id` once that tag is gone. Docker removes only the tag while another tag or another image still holds the image. |
| `import` | Only the record. The image was on the Docker host before it was registered, so Docker keeps it. |
| `catalog` | Only the download. The image stays listed with `downloaded: false`, and a container, a build or `.../pull` downloads it again. `404` when it is not downloaded. |

The answer comes at once, without a job: `{"id": ..., "name": ..., "deleted": true, "image_removed": true}`, where `image_removed` tells whether Docker deleted the image itself rather than only its tag. Nothing is forced and nothing is pruned, and a refusal deletes nothing. A container created from the image refuses it with `409` and its name, since a reset, a retry and a settings change that recreates the container need that image again; delete the container first. Another saved image of the same Docker image, such as an archive imported on the host it was exported from, keeps it for the container, so either record can go then. Docker's own refusal, for an image another Docker container still uses or one that a tag Thoric did not give it still names, is `409` with Docker's message. So is a delete while a download, a build or another change of the same image runs. Images built or saved from the deleted one keep working: Docker keeps the layers they share with it.

## Image sizes and layers

Every image Docker has carries three sizes in bytes, all `null` for an image Docker does not have:

| Field | Size |
| --- | --- |
| `size` | The size Docker reports, as `docker image ls` shows it. The containerd image store, the default of Docker 29 and of the WSL package, also counts the compressed copy it keeps of each layer. |
| `layers_size` | What the image's layers hold unpacked. |
| `own_size` | The part of `layers_size` that no other listed image brought. Images share the layers they have in common, and Docker stores a shared layer once, so own sizes add up to the space the listed images take without counting a layer twice. |

`GET /api/images/{id}/layers` explains them with the image's history, the newest step first as `docker history` lists it:

```json
{
  "id": "5b1e0c9f2a7d4e3b8c6a1f0e9d8c7b6a",
  "image_id": "sha256:c86959cb711a...",
  "size": 2630695169,
  "layers_size": 1977327616,
  "own_size": 1067704320,
  "steps": [
    {"instruction": "LABEL thoric.managed=true", "kind": "config", "size": 0, "created": 1792351597, "layer": false, "from": null, "shared_with": []},
    {"instruction": "RUN bash /tmp/thoric-setup.sh && rm /tmp/thoric-setup.sh", "kind": "run", "size": 1067692032, "created": 1792351597, "layer": true, "from": null, "shared_with": []},
    {"instruction": "ADD file:7121eb4a5ba3... in /", "kind": "add", "size": 87633920, "created": 1790928654, "layer": true, "from": "ubuntu-xfce", "shared_with": ["ubuntu-xfce", "0d4c2b1a9e8f7d6c5b4a3f2e1d0c9b8a"]}
  ]
}
```

`instruction` is the step as a Dockerfile says it, without the shell and build arguments the builder recorded around it, and at most 4 KiB. `kind` is `run`, `copy` or `add` for steps that write files, `config` for settings such as `ENV` or `LABEL`, which write none, and `commit` for a container's files saved by `docker commit`: a snapshot, or a settings change that carried a container's files over to its replacement, named in `comment`. `size` is the bytes the step wrote, unpacked, `created` is in Unix seconds (`null` when Docker records no time), and `layer` tells whether the step wrote one of the image's layers. `from` is the list `id` of the image the step came from, the listed image with the fewest layers that has it, or `null` when it is this image's own; `shared_with` names every other listed image that has it. Steps are compared by the layers below them rather than by their history, so an image built on an earlier build of a catalog image still shares the layers that build has in common with the current one. Docker never changes an image ID's layers or history, so the manager reads them once per image.

## Image icons

A saved image can have an icon, a PNG of at most 256 by 256 pixels and 300 KiB that lists show beside its name. Requests and exported configurations carry it as a data URL, `"icon": "data:image/png;base64,..."`; the dashboard scales any picture to fit before it sends one. `PUT /api/images/{id}/icon` with `{"icon": "data:image/png;base64,..."}` sets it and `DELETE` removes it, both returning the image as lists show it. Lists name the icon by a version instead of carrying it, and `GET /api/images/{id}/icon?v=<version>` returns the PNG; with the current version the response may be cached for good, and without it the browser revalidates with the `ETag`. A snapshot keeps the icon of the saved image its container was created from, and image archives carry it. Catalog images have none.

## Templates

The dashboard's **Templates** page lists ready-made images. It reads the list in the browser from `templates_url` in `/api/settings`, by default `templates/index.json` on the project's GitHub wiki; `TEMPLATES_URL` points it elsewhere or turns it off. A template is a build request, the same JSON `/api/images/{id}/config` returns, beside the list as `<id>.json` with its icon as `<id>.png`; installing one fills in the **Build image** form, and the build sets `template_id` so the page can tell which templates are installed. The manager itself never fetches a template, and the content security policy allows the list's host for these requests and pictures only. [The development guide](DEVELOPMENT.md#add-a-template) describes the list format.

An entry of the list names the `architectures` its template builds on, `amd64` when it names none. The page compares them with `architecture` in `/api/settings`: a template that does not build on the Docker host's architecture, such as one that downloads programs made for x86_64 on a Raspberry Pi, shows apart from the others without **Install**, and **Merge and install** refuses it. The manager does not check this: a build request of such a template still runs, and fails, or builds an image whose programs do not start, where its script installs what the host cannot run.

A template may have `options`, choices the install form shows, such as an optional tool. They are resolved in the browser, and a build request never carries them: the request has the script with each value written at its top, after its `#!` line, as `NAME=true` or `NAME=false`, and leaves out the desktop MCP servers an option that is off names. The image's record, and so its exported configuration, keeps that resolved script. A merged build writes each template's values into its own part.

### Merged templates

**Merge and install** builds several templates into one image, such as VS Code with the Node.js environment. The page merges them in the browser into one build request with `parts` instead of a `script`, a compose with one [step](#steps) for every template that has one, and their ids in `template_ids`, in the order they were chosen:

```json
{
  "name": "Visual Studio Code + Node.js",
  "base_image": "ghcr.io/co3moz/thoric/ubuntu-xfce:24.04",
  "internet": true,
  "parts": [
    {"id": "vscode", "name": "Visual Studio Code", "script": "#!/usr/bin/env bash\n..."},
    {"id": "node", "name": "Node.js", "script": "#!/usr/bin/env bash\n..."}
  ],
  "compose": {"steps": [{"id": "vscode", "name": "Visual Studio Code", "parameters": [{"name": "GIT_NAME"}, {"name": "GIT_EMAIL"}], "script": "..."}]},
  "template_ids": ["vscode", "node"]
}
```

A request has a `script` or 1 to 8 `parts`, not both. Each part has an `id` of lowercase letters, digits and dashes, unique in the build, a `name` of one line and at most 100 characters, and a `script`; the scripts take at most 1,000,000 bytes together. The build runs them as root in this order, each in a layer of its own named after its part, so the build's output says whose script failed and Docker's cache keeps the parts before it. The image is built on one base, the first template's; the page warns when another template was made for a different one. The build form shows every part before anything runs, and a part can be moved or left out there, with its compose step. An exported configuration of a merged image has the same `parts` and `template_ids`, so it builds the image again elsewhere, and can be a template of its own.

## Composes

A saved image can carry a compose: parameters that a new container from it asks for, and a setup script that receives them. `PUT /api/images/{id}/compose` sets or replaces it, `GET` returns it with its script and `DELETE` removes it; `id` is the record's `id`. Image lists and job results show the compose without its script.

```json
{
  "parameters": [
    {"name": "UNITY_EMAIL", "label": "Unity email", "description": "The Unity ID you sign in with.",
     "required": true},
    {"name": "UNITY_PASSWORD", "label": "Unity password", "required": true, "secret": true},
    {"name": "ACCELERATOR", "label": "Cache server", "pattern": "[A-Za-z0-9.-]+:[0-9]{1,5}",
     "pattern_message": "host:port, for example 192.168.1.20:10080"},
    {"name": "UNITY_EDITOR", "label": "Editor", "type": "choice", "allow_custom": true,
     "options": [{"value": "6000.0.23f1", "label": "Unity 6"}, {"value": "2022.3.50f1", "label": "2022 LTS"}],
     "default": "6000.0.23f1"},
    {"name": "BUILD_JOBS", "label": "Build jobs", "type": "number", "display": "slider",
     "min": 1, "max": 16, "default": "4"},
    {"name": "PLATFORMS", "label": "Platforms", "type": "choices",
     "options": [{"value": "linux"}, {"value": "windows"}, {"value": "webgl"}], "default": "linux"},
    {"name": "UNITY_HOME", "label": "Unity installs", "type": "volume", "path": "/opt/unity",
     "default": "unity-installs"},
    {"name": "WORKSPACE", "label": "Workspace", "type": "volume", "path": "/home/desktop/Projects",
     "new_volume": true},
    {"name": "UNITY_PROJECT", "label": "Unity project", "type": "folder",
     "default": "/home/desktop/UnityProjects/MyGame"}
  ],
  "script": "set -eu\nwhile read -r platform; do echo \"$UNITY_EDITOR for $platform\"; done <<< \"$PLATFORMS\"\ntest -d \"$UNITY_HOME/$UNITY_EDITOR\" || echo \"install into $UNITY_HOME\"\n",
  "root": false,
  "timeout_seconds": 600
}
```

| Field | Default | Meaning |
| --- | --- | --- |
| `parameters` | `[]` | Up to 32 objects with `name` and optional `label`, `description`, `type`, `display`, `required`, `default` and the fields of their type, below. |
| `script` | Required | Bash script of at most 1,000,000 bytes, run with fail-on-error and standard input closed. |
| `root` | `false` | Run as root, in `/root`, instead of the desktop user, UID/GID 1001:1001, in `/home/desktop`. |
| `timeout_seconds` | `600` | From 1 to 7200 seconds, two hours, for setups that download large files. |

### Steps

A compose can also be several steps, run in order, such as one for each template [merged](#merged-templates) into an image. It is written as `{"steps": [...]}`, 1 to 8 objects with the fields above and an `id` of lowercase letters, digits and dashes, unique in the compose, and an optional `name` the dashboard shows. A compose of one step without an `id` or `name` is written with its fields at the top, as above. The scripts take at most 1,000,000 bytes together, and each step has its own `root` and `timeout_seconds`.

Each step's script receives only its own parameters, so two steps may ask for the same name, such as a `VERSION` each. A value then goes by `STEP.NAME`: the create request's `parameters`, the container's `setup.parameters` and `setup.secrets` all name it `chrome.VERSION` or `vscode.VERSION`. A request may still give a value by its name alone when only one step asks for that name; when several do, the request is refused with the names to use. Volume parameters of all steps still need paths of their own. A step's mistake, in the compose or a value, names the step, and the dashboard asks for each step's values in a section of its own.

The setup runs the steps one after another in the same job, each a part of its [script progress](#script-progress). A step that fails stops the setup, and the error names it; a retry or a reset runs every step again, so write each script to work where it already ran. The setup job's log masks the secrets of every step in each step's output, in case a script prints something an earlier one saved.

A parameter's `name` is the environment variable that carries its value to the script: at most 64 capital letters, digits and underscores, starting with a letter. Names that the shell or the desktop session relies on, such as `PATH`, `HOME`, `DISPLAY` or names starting with `LD_`, `XDG_` or `DESKTOP_`, are refused. A `label` is what the dashboard asks for, the name when empty. A `description` of at most 500 characters is shown with the field, such as where to find the value. Values are text of at most 16 KiB. A secret parameter cannot have a default, because image lists return the compose.

Every value reaches the script as text, so a parameter's `type` decides what text it accepts and how the dashboard asks for it. `default` is written the same way the script receives it.

| `type` | `display` | The script receives | Fields of the type |
| --- | --- | --- | --- |
| `text` (default) | `input` (default), `multiline` | The text; one line unless `multiline` | `secret`, `pattern` and `pattern_message` for a format |
| `number` | `input` (default), `slider` | A plain decimal such as `42` or `-1.5`, without exponents | `min`, `max`, `step`; a slider needs `min` and `max`, and steps count from `min` or from 0 |
| `boolean` | `toggle` (default), `checkbox` | `true` or `false`; `false` when left out | |
| `choice` | `select` (default), `radio` | One option's `value` | `options`, `allow_custom` to also accept other text |
| `choices` | `checkboxes` | The chosen values, one per line, each at most once | `options` |
| `volume` | `select` | Its `path`, and the chosen volume's name in `NAME_VOLUME` | `path`, required; `new_volume` for a volume of each container's own |
| `folder` | `input` | An absolute folder of the desktop | |

`options` holds 1 to 100 objects with a `value` of one line and an optional `label` the dashboard shows instead. A field of another type, a display the type does not have or a default the parameter would refuse is an error. `required` means a value must be given; for a boolean, that it must be `true`, as for an agreement. Read several choices in bash with `while read -r item; do ...; done <<< "$PLATFORMS"`.

A text parameter's `pattern` is a regular expression that a value must match as a whole, as if it were written `^(?:pattern)$`: one line of at most 500 characters in the syntax of Rust's [regex crate](https://docs.rs/regex/latest/regex/#syntax). A multiline value is matched as a whole too, so a pattern for several lines spells out the line breaks, such as `[a-z]+(\n[a-z]+)*`. The crate takes time linear in the value whatever the pattern, and has no lookaround or backreferences. A pattern it cannot compile, or one that needs more than 4 MiB once compiled, such as `\w{1,100}` with its Unicode classes, is refused when the compose is saved, with an error that names the parameter. `pattern_message` describes the expected format in one line of at most 200 characters, and needs a `pattern`: the dashboard shows it under the field, and the error repeats it, `Cache server does not have the expected format: host:port, for example 192.168.1.20:10080`. An empty value is still left to `required`, a `default` must match, and a secret can have a pattern too; the error never contains the value. The dashboard checks a pattern as it is typed, with the browser's regular expressions, which read most patterns the same way. There, `\d`, `\w` and `\b` cover ASCII only, while in the manager they cover the digits and letters of every script, so write `[0-9]` or `[A-Za-z]` where the difference matters. A pattern the browser cannot read, such as one with `\A` or `(?i)`, is checked by the manager alone when the container is created.

A volume parameter keeps data that every container of an image needs in the same place, such as Unity installs, out of the image and the container. Its value is the name of an existing named volume (`GET /api/volumes`, one not marked `manager`), or empty or `"-"` for none, and `required` means a volume must be chosen. The volume is mounted at the parameter's `path`, an absolute, normalized folder that the compose fixes and that follows the same rules as a [volume setting](#create-a-container-and-follow-the-job): not `/`, `/opt`, `/var` or `/home` themselves, and nothing under the system folders or the paths the manager mounts itself, such as `/tmp`, `/run`, `/dev` and `/usr`. Two volume parameters cannot share a path, and no other parameter can be named like a volume parameter's second variable. The script receives the path in the parameter's variable and the volume's name, or an empty string, in the same name with `_VOLUME` appended: `UNITY_HOME=/opt/unity` and `UNITY_HOME_VOLUME=unity-installs`. Without a volume the folder stays inside the container, in its files and its snapshots. Either way, before the script runs, a missing folder is created and an empty one owned by root is given to the desktop user, as Docker leaves a new volume's mount point to root; a folder that already holds files keeps its owner. Docker creates the missing folders above a volume's path owned by root. In the desktop's home, those of every volume the container mounts go to the desktop user before the container starts, so a volume under `~/.config` leaves the desktop session a configuration folder it can write. Elsewhere, those inside a folder of the desktop user go to it before the script runs, while they hold nothing else.

A volume parameter with `"new_volume": true` gives each container a volume of its own instead, such as a workspace that no other container shares. A container whose request leaves the value empty, or out, gets the volume `thoric-CONTAINER_ID-NAME`, the parameter's name in lowercase with dashes for underscores, such as `thoric-0f1e...e1f0-workspace`, or `thoric-CONTAINER_ID-STEP-NAME` for a step of [several](#steps), whose step id and parameter name take at most 83 characters together; a request that names a volume still uses that one, and one that gives `"-"` gets none, which the container keeps as an empty value, as the dashboard's **No volume** does. `required` then refuses only `"-"`, and the parameter cannot have a `default`, since an empty value is what asks for the new volume. The creation job creates the volume, so a request that is refused or a form that is cancelled leaves nothing behind, with the labels `thoric.managed=true`, `thoric.environment=CONTAINER_ID` and `thoric.parameter=NAME`. A volume of that name that does not carry the container's labels stops the job. The value, the script's `NAME_VOLUME` and the container's `setup.mounts` name the volume like a chosen one, and a reset, a retry and a settings change mount it again. A container created from a snapshot gets a new volume of its own, not its source container's. The volume outlives its container unless the container is deleted with `delete_volumes=true`, and `GET /api/volumes` names its owner.

A folder parameter names a folder of the desktop the script works in, such as the project an editor opens. Its value is an absolute, normalized path outside the system folders, by the rules of a volume parameter's `path` except that it may hold a comma: `/home/desktop/UnityProjects/MyGame` or `/srv/projects`, not `/tmp/build`. The manager neither creates the folder nor changes its owner; the script does what it needs. Uploads may go into it and into a volume parameter's `path`, with or without a volume, and the dashboard's upload dialog offers both by their labels, as [files](#files) describes.

`POST /api/containers` takes the values in `parameters`, such as `{"UNITY_EMAIL":"dev@example.com","UNITY_PASSWORD":"..."}`. A value that is left out uses the default. Every parameter reaches the script, an optional one without a value as an empty string, so `set -u` does not trip on it. An unknown name, a missing required value or a value its parameter refuses, such as text that does not match its `pattern`, fails the request, and so does a volume parameter's volume that is also a `mounts` entry's volume, chosen for two volume parameters, or given at a path a `mounts` entry already takes. The job refuses a volume that does not exist, without creating an empty one, and the manager's own volumes, which a reset, a retry and a settings change that recreates the container refuse too. The one exception is the volume a `new_volume` parameter gets, which the job creates before it saves the container record. Once the record exists, its volume was created, so a reset, a retry or a settings change that finds it missing stops like it does for any other volume instead of creating an empty one in its place: the volume may have held the container's work, and an empty replacement would hide that it is gone. Restore it under the same name, for example by [importing an archive](#volume-archives) of it, or delete the container. The creation job waits until the desktop reports ready, for at most five minutes, then runs the script in the desktop session's environment (`HOME`, `DISPLAY`, `XDG_RUNTIME_DIR`) and completes only when the script succeeds.

The container pins the compose and the values, so editing the image later does not change what it runs. Its `setup` field reports `status` (`pending`, `running`, `completed` or `failed`), the `image` record the compose came from, the values of ordinary `parameters`, the names of the `secrets`, the `mounts` its volume parameters were given, as `volume` and `target`, and the `places` its volume and folder parameters name, as `label` and `path`; a folder parameter left empty names none. Containers created before places were kept have none. A compose of [several steps](#steps) adds `steps`, each step's `id`, `name` and `status` in order, and its places' labels begin with the step's name. Those volumes are mounted beside the `mounts` setting every time the container is created again: by a reset, a settings change and retry. The settings do not list them, so a settings request cannot drop them, and one that puts another volume at their path or their volume at another path is refused. Secret values are never returned, and the setup job's log shows `***` in their place. The manager keeps them in its database until the container is deleted, so that the setup can run again: after a reset, and when retry recreates a missing container from the source image. A settings change keeps the container's files and does not run it. A script that fails leaves the container running with `last_error` set; retry runs it again.

A snapshot keeps the compose its container was created with, so a container created from the snapshot asks for the values again. Write the script so it also works in a container where it already ran. The build configuration and an image archive carry the compose too.

### A base image's compose

A build on a saved image keeps that image's compose, before its own. The base's layers already hold what its own build installed, with the [options](#templates) chosen then, so nothing of that build runs or is asked for again. Its compose is what a new container asks for and sets up once its desktop is ready, which a container from the built image needs too. When only one of the two has a compose, the built image gets that one as it is, a compose of one step keeping its value names. When both do, the image's compose is the base's [steps](#steps) followed by its own, joined as [merged templates](#merged-templates) are:

- A compose of one step that names none takes the id of the one template its image was built from, otherwise `base` for the base's and `build` for the build's own, and the image's name as its `name`.
- A step of the build's own with the id of one of the base's, or with the same script and parameters, takes that step's place, such as a template installed again on an image that has it. An id the manager chose replaces nothing and takes the next free one, such as `build-2`.
- The values then go by `STEP.NAME`, and a `new_volume` parameter's volume is `thoric-CONTAINER_ID-STEP-NAME`, for new containers; a container keeps the compose it was created with.
- The image's `template_ids` are the base's followed by its own, each once, the newest 8.

Composes that do not fit together, more than 8 steps or two volume parameters at one path, fail the build before its script runs. `"inherit_compose": false` builds without the base's compose and is kept in the exported configuration. The job's log names the steps it kept.

`POST /api/images/{id}/compose/base` gives an image built on a saved image before builds kept their base's compose the steps it lacks, by the same rules and without building anything; `id` is the record's `id`. It answers the image as lists show it, or `409` when the image was not built on a saved image with a compose or already has every step of it. Image lists mark such an image with `base_compose_missing: true`, and the dashboard's **Edit compose** offers to add them.

The job stops reading the script's output a second after the script exits. A program the script leaves running fails if it writes to that output later, so start long-running programs with the [applications API](#applications-and-windows), or redirect their output.

## Files

File reads and writes use the desktop user's permissions and require a running container. They reject symlinks and noncanonical paths. Reads accept regular files only.

Read request:

```json
{"path":"/home/desktop/project/config.json","encoding":"utf8","offset":0,"limit":65536}
```

The response includes `data`, `encoding`, `bytes`, `size`, `offset`, `next_offset` and `eof`. `limit` is 1 to 524288 bytes. Use `encoding:"base64"` for binary files or slices that split UTF-8 characters.

Write request:

```json
{"path":"/home/desktop/project/config.json","data":"{\"enabled\":true}\n","encoding":"utf8","replace":false}
```

Writes are atomic, create missing parent directories and use mode 0600, and return the `path`, its `bytes` and its `download_url`. Existing files return 409 unless replacement is explicit. The decoded file limit is 512 KiB. Use `encoding:"base64"` for binary data.

`POST .../files/list` with `{"path":"/home/desktop/project","limit":500}` returns `items` with `name`, `type` (`file`, `directory`, `symlink` or `special`), `size`, `modified_at` and `mode`, plus `total` and `truncated`. `POST .../files/mkdir` with `{"path":"/home/desktop/project/assets"}` creates one folder in an existing parent and returns 409 when the name exists. `POST .../files/move` with `{"source":...,"destination":...,"replace":false}` renames within the same volume and returns 409 when the destination exists. `POST .../files/delete` with `{"path":...,"recursive":false}` removes a file, a symlink itself or a directory; top-level directories, `/home/desktop` and mounted volume roots are refused. `GET .../files/download?path=...` streams a regular file with `Content-Length`, a guessed `Content-Type` and an attachment `Content-Disposition`; it refuses symlinks, files larger than `UPLOAD_MAX_MB` and, with 403, a file the desktop user cannot open, which is tried before any headers are sent so a refused file never arrives as a body cut short. Responses that describe a saved file, such as `download_file`, `export_artifacts` and saved screenshots, include this route as a root-relative `download_url` for the dashboard session.

The same route downloads a folder: its path answers with a gzip-compressed tar named after the folder, such as `project.tar.gz`, that holds the folder under its own name. tar runs in the desktop as the desktop user while the archive streams, so its size is not known up front and `UPLOAD_MAX_MB` does not apply. No symlink on the path is followed, and symlinks inside the folder are archived as links. The transfer is a `folder-download` job, named in the `X-Job-Id` response header, whose `progress` counts the bytes sent; it shares the two archive transfer slots with image and volume archives. A file the desktop user may not read fails the download instead of leaving the archive incomplete, and a file that changes while it is read is archived as it was read and noted in the job log. `/` itself is refused; download the folders in it.

```bash
curl --cookie thoric.cookies --remote-header-name --remote-name \
  'http://localhost:8080/api/containers/CONTAINER_ID/files/download?path=/home/desktop/project'
```

For larger uploads, POST the raw file body with `Content-Type: application/octet-stream` to `/api/containers/{id}/files`. Supply `destination`, `name`, `size` and optional `replace` as query parameters:

```bash
curl --cookie thoric.cookies \
  --header 'X-Thoric-Request: 1' \
  --header 'Content-Type: application/octet-stream' \
  --data-binary @notes.txt \
  'http://localhost:8080/api/containers/CONTAINER_ID/files?destination=/home/desktop/Downloads&name=notes.txt&size=FILE_SIZE_IN_BYTES'
```

Replace `FILE_SIZE_IN_BYTES` with the exact size. URL-encode filenames and destination paths when necessary. Streaming uploads are limited by `UPLOAD_MAX_MB`. They return the saved `path`, its `bytes` and its `download_url`, and create missing folders of the destination. The body is received whole into a temporary file of the manager first, after the destination is checked, so a slow transfer holds nothing of the container; only the delivery into the desktop shares the container's lock, and it waits five seconds for a reset, a snapshot or another change of the container and answers 409 after that. The same holds for a volume's upload and its write lock.

### File search

`POST .../files/search` searches a folder of a running desktop with everything below it, or one regular file, as the desktop user and without following symlinks:

```json
{"path":"/home/desktop/project","pattern":"TODO|FIXME","literal":false,"name":"*.py","case_sensitive":true,"max_results":100,"context":1,"include_ignored":false,"max_seconds":10}
```

Only `path` and one of `pattern` and `name` are needed. `pattern` is a regular expression in Python's syntax matched within each line of a file, plain text with `literal`, of at most 1000 characters on one line; `name` is a glob for an entry's own name. The answer of a content search holds `matches` with `path`, `line`, `column`, `text` (at most 400 characters, `cut` when shortened) and, with `context` of up to 10, `before` and `after`; a name search holds `files` with `path`, `type` and `size`. Both hold the counts `entries_visited`, `files_searched`, `files_matched`, `binary_skipped`, `directories_skipped` and `unreadable`, `elapsed_ms`, and `truncated` with `truncated_by`: `results` (`max_results`, 1 to 1000), `time` (`max_seconds`, 1 to 30), `entries` (100,000 looked at) or `size` (256 KiB of results), or null. The bounds, the folders left out unless `include_ignored` and what is skipped are those of the MCP tool `search_files` (`docs/MCP.md`). A request out of bounds, a pattern that is no regular expression and a path that does not exist or holds a symlink answer 400, a stopped container 409, and a search Docker did not answer within its time and 15 seconds more 504.

### Changed files

`GET /api/containers/{id}/changes` lists what changed in the container's own files: Docker's change list of its Docker container, each path `added`, `changed` or `deleted` against the image that Docker container was created from. It reads the Engine API's `GET /containers/{id}/changes` where the manager reaches the socket and `docker container diff` otherwise, and works whether the container runs or not. A container whose Docker container is missing answers 409 with a hint to retry it.

The list starts when the Docker container was created. A reset, a retry that recreates a missing container, a settings change that recreates the container and a restore of a backup with the container's files each start it again, and after the last two the earlier changes are inside the commit it was created from. `base` says which: `image` is the reference Docker created it from, `image_id` that image, `commit` is true for such a commit of the earlier files, and `created_at` is when Docker created it, in Unix seconds. Docker keeps no list from a snapshot, so none is offered. Named volumes, host folders and the folders in memory are mounted over the container's files and never in the list; `mounts` names each with its `path`, `type` (`volume`, `bind` or `tmpfs`), `read_only` and, for a volume, `volume`, without a host folder's own path. The paths are Docker's view of the whole filesystem, so the list also names files in folders the desktop user cannot open, such as `/root`, though never what is in them; an [MCP endpoint](#desktop-mcp-servers-and-endpoints) with `files_read` sees them the same way. At most two lists are read at once, and a third request waits its turn.

Query parameters, all optional:

| Parameter | Meaning |
| --- | --- |
| `path` | A canonical absolute folder; only what is at or below it is listed and counted. |
| `kinds` | Some of `added`, `changed` and `deleted`, separated by commas. All three by default. |
| `all` | `true` also lists what the desktop changes on its own. |
| `limit` | The most entries listed, 1 to 5000, 500 by default. |

What the desktop changes on its own is left out by default, since a desktop's session writes dozens of files at every start: `noise` has `left_out`, whether it was, `count`, how many listed or left out entries the rules matched, and its `rules`, each with its `id`, `description`, `count` and, when it matched an entry, its `paths`, where a path ending in `/**` stands for a folder and everything in it and one ending in `*` for every path that starts so. The rules are `system` (`/proc`, `/sys`, `/dev`), `temporary` (`/tmp`, `/run`, `/var/tmp`, `/var/run` and `/var/lock`, with the X11 and ICE sockets), `caches` (`/var/cache`, `/var/log`, apt's package lists, the library cache `/etc/ld.so.cache`, `~/.cache` and root's), `start` (the sudo rules, the login profile and the machine ID every start writes), `session` (the session's bus, sound and input method state, the XFCE and GNOME settings and state, the services GNOME starts and the standard folders such as `~/Documents`, whose files are still listed, as are GNOME Shell extensions, file manager scripts and Thunar's files beside its default actions), `gpu` (the driver files and tools the NVIDIA Container Toolkit mounts into a desktop that has the GPU through it, only for such a desktop), `mounts` (the folders mounts sit on) and `folders` (a changed folder whose entries below say what changed in it, and a new folder whose entries below are all left out). They live in `core/manager/src/changes/noise.rs`, whose tests hold a fresh desktop of either catalog image, and one with the GPU in WSL, to an empty list. A desktop with render devices under `/dev/dri` still lists `/etc/group` and `/etc/gshadow`, which its start changes to join their groups, since who belongs to which group is never left out. With `all=true` every entry is listed, one a rule matches with that rule as `noise`.

```json
{"container_id":"...","running":true,
 "base":{"image":"ghcr.io/co3moz/thoric/ubuntu-xfce:24.04","image_id":"sha256:...","commit":false,"created_at":1793740921},
 "mounts":[{"path":"/home/desktop/project","type":"volume","volume":"project","read_only":false},{"path":"/tmp","type":"tmpfs","read_only":false}],
 "path":"/",
 "summary":{"total":3,"added":2,"changed":0,"deleted":1,"folders_truncated":false,
   "folders":[{"path":"/home/desktop/notes","total":2,"added":2,"changed":0,"deleted":0},{"path":"/etc","total":1,"added":0,"changed":0,"deleted":1}]},
 "noise":{"left_out":true,"count":47,"rules":[{"id":"session","description":"...","count":38,"paths":["/home/desktop/.dbus/**","..."]}]},
 "total":3,"returned":3,"truncated":false,"details":"read",
 "changes":[{"path":"/etc/legal","kind":"deleted"},
   {"path":"/home/desktop/notes","kind":"added","type":"directory","modified_at":1793741000},
   {"path":"/home/desktop/notes/todo.txt","kind":"added","type":"file","size":42,"modified_at":1793741000}]}
```

`changes` are sorted by path, with what is below a folder right after it. `summary` counts what is listed by kind, and in `folders` the entries right below `path`, folders or files, with the most entries at or below them first, at most 100; without `path` the desktop home's own entries count apart. `total` is how many entries match, `returned` how many are in `changes` and `truncated` whether `limit` cut the list. In a running desktop, each listed entry under `/home/desktop` that still exists also has `type`, `size` for a file and `modified_at`, read as the desktop user without following symlinks; `details` is `read`, `partial` when only the first 1000 such entries were read, `failed` when they could not be, and `stopped` for a stopped container.

Uploads, saved screenshots and exported artifacts follow the same destination rule. The folder may be anywhere in the desktop home, `/home/desktop`, in a volume mounted writable, by the `mounts` setting or a compose's volume parameter, or in one of the `places` in the container's `setup`; nothing under a read-only volume is accepted. The rule keeps writes out of the system folders. What protects the desktop is that files are written as the desktop user, so its permissions apply, and that no symlink in the destination is followed.

## Volume files

The `/api/volumes/{name}/files/...` routes work on a named volume without a container that mounts it, with the same request bodies and limits as a container's [file routes](#files). Paths are absolute within the volume: `/` is its root, which can be listed and written into but not deleted or moved. `mkdir` takes `{"path":"/project/assets"}` and returns 409 when the name exists. Uploads take `destination`, `name`, `size` and optional `replace` as query parameters, as a container's do.

Each request starts a short-lived helper container from the manager's own image, so it needs `MANAGER_CONTAINER`. The helper mounts only that volume, has no network, a read-only root and no capabilities beyond the ones the operation needs, never gets the Docker socket and is removed when the request ends. Files are read and written as the desktop user, UID/GID 1001, so the permissions a desktop would meet apply: a `403` names a file that user may not change. Docker creates a volume's root owned by root; a write to a volume that is still empty first gives its root to the desktop user, as mounting it over a desktop folder would. A volume that holds files keeps its owners.

A missing volume returns 404 instead of being created. The manager's own volumes, such as its data volume, are refused with 403. Writes to one volume run one at a time; reads do not wait for them.

A folder of a volume downloads as a `.tar.gz` like a container's. Its path is checked as the desktop user, then the archive is read the way a [volume export](#volume-archives) reads, as root on a read-only mount, so files the desktop user may not read are included. `GET /api/volumes/{name}/archive` downloads the whole volume.

## Volume archives

`GET /api/volumes/{name}/archive` streams a volume's files as an ordinary gzip-compressed tar, so other tools read it too. A helper container reads the volume as root on a read-only mount, with only the capability to read every file, so files the desktop user may not read are included; numeric owners and permissions are recorded. The transfer is a `volume-export` job, named in the `X-Job-Id` response header, whose `progress` counts the bytes sent. A file that changes while it is read is archived as it was read and noted in the job log.

```bash
curl --cookie thoric.cookies --output unity-projects.tar.gz \
  http://localhost:8080/api/volumes/unity-projects/archive
```

A volume's file routes run their helper under the volume's lock from the check that the volume exists until Docker has the helper container, since `docker run` would otherwise create a volume anew that a delete removed in between; a listing or a file read holds it for the whole request. An upload the helper refuses, such as into a folder the desktop user cannot write, is answered with the helper's reason.

`POST /api/volumes/import?name=NEW_VOLUME` with `Content-Type: application/octet-stream` and the archive as the body creates the volume `name` and unpacks the archive into it. The name must not exist yet: an import never replaces a volume's files, and a name in use returns 409. The archive is `.tar.gz` or `.tar`, at most `IMAGE_ARCHIVE_MAX_MB` after decompression. tar unpacks it as the desktop user in a helper without network, so every file belongs to UID/GID 1001: a setuid bit grants nothing beyond that user, device files cannot be created, and nothing lands outside the volume. The response is a `volume-import` job once the whole archive has arrived; if the upload or unpacking fails, the new volume is removed again.

`size`, the archive's exact length in bytes, is optional. With it, a body of any other length fails the import, and a size above the limit is refused at once with 413. Without it, an archive can stream in as it is made, with no temporary file on the sending side:

```bash
tar -c -C project . | curl --cookie thoric.cookies \
  --header 'X-Thoric-Request: 1' \
  --header 'Content-Type: application/octet-stream' \
  --request POST --upload-file - \
  'http://localhost:8080/api/volumes/import?name=NEW_VOLUME'
```

curl sends such a body in chunks. Its bytes are counted as they arrive, and the import fails once they pass `IMAGE_ARCHIVE_MAX_MB`. A connection that closes before the last chunk fails it too, and so does an archive that ends without tar's end-of-archive blocks, which every tar writes: GNU tar would otherwise unpack an archive cut at a file boundary as if it were whole, as one is when the sending tar stops early. The job's `progress` then has `current` and no `total`.

`POST /api/volumes/{name}/extract?path=/FOLDER` unpacks a `.tar.gz` or `.tar` into a new folder of a volume that exists, such as one a running container mounts. The body, `size` and limits are those of an import:

```bash
tar -c -C project . | curl --cookie thoric.cookies \
  --header 'X-Thoric-Request: 1' \
  --header 'Content-Type: application/octet-stream' \
  --request POST --upload-file - \
  'http://localhost:8080/api/volumes/unity-projects/extract?path=/worktree-42'
```

The folder must not exist yet. The extract creates it, so no file of the volume can be replaced and no symlink already in the volume can lead the archive elsewhere. `path` is absolute and canonical, as for the [volume file routes](#volume-files), and its parent must exist; `/` is the volume itself, accepted only while it is empty, such as right after `POST /api/volumes`. tar runs inside the new folder as the desktop user, in a helper that mounts only that volume, so every file belongs to UID/GID 1001. The extract holds the volume's write lock, so file writes to the volume wait for it. A folder that exists returns 409, a missing volume 404 without creating it, and the manager's own volumes 403. The response is a `volume-extract` job once the whole archive has arrived; if the upload or unpacking fails, the folder is removed again, or the volume emptied again for `/`. Unpacking over a folder that already holds files is not supported: extract into a new folder, then delete or move the old one. An MCP client, which has no session, gets a `volume_extract` [transfer link](#transfer-links) for this route.

`POST /api/volumes/{name}/copy` with `{"name":"NEW_VOLUME"}` copies every file of the volume into a new one, for work that starts from a prepared volume or a copy kept before a risky change; MCP clients call `copy_volume`. Docker has no volume copy, so the manager pipes the two archive helpers into each other, without compression: one reads the source as root on a read-only mount with only the capability to read every file, the other unpacks into the new volume as the desktop user, and no helper mounts both. The copy keeps the tree, symlinks, hard links and modes. Every file belongs to UID/GID 1001, as after an import, and device files cannot be copied. The new name must not exist (409), a missing source returns 404 and is not created, and the manager's own volumes are refused on either side with 403. The response is a `volume-copy` job whose `progress` counts the bytes copied. It takes one of the two archive transfer slots, `IMAGE_ARCHIVE_MAX_MB` does not apply since nothing is uploaded, and a copy that fails removes the new volume again. A file that changes while it is read is copied as it was read and noted in the job log.

A copy is a full copy, not copy-on-write: disk use grows by the volume's size, and the WSL virtual disk does not shrink by itself after a volume is deleted.

## Image archives

`GET /api/images/{id}/archive` streams a saved image, where `id` is the record's `id` from `GET /api/images`. The download is a gzip-compressed `docker save` archive of the image ID, so it names no image. Its first entry, `thoric.json`, holds the record's name, description, kind, desktop information, icon and template, and a script build's configuration, so `/api/images/{id}/config` works for the imported record too. Docker's own `docker load` reads the archive too and reports the loaded image ID.

```bash
curl --cookie thoric.cookies --output unity.tar.gz \
  http://localhost:8080/api/images/IMAGE_RECORD_ID/archive
```

An archive holds every layer, the base desktop included, and its size is not known in advance. Docker prepares the whole image before its first layer arrives. The transfer is an `image-export` job, named in the `X-Job-Id` response header, whose `progress` counts the bytes sent. If Docker fails partway, the download ends with an error instead of a short file.

`POST /api/images/import` takes the archive as an `application/octet-stream` body: an exported `.tar.gz` or a plain `docker save` `.tar`. Supply its exact `size` when it is known, and optionally `name` and `description`, as query parameters; the name and description default to the values stored in the archive. `--upload-file` streams the file instead of reading it into memory:

```bash
curl --cookie thoric.cookies \
  --header 'X-Thoric-Request: 1' \
  --header 'Content-Type: application/octet-stream' \
  --request POST --upload-file unity.tar.gz \
  'http://localhost:8080/api/images/import?size=FILE_SIZE_IN_BYTES&name=Unity%20workspace'
```

The archive streams into `docker load` as it arrives. On the way, the manager removes every tag and image name the archive carries, so an import cannot replace a catalog image or any other existing tag. Docker can register the image only after the archive has passed these checks:

- It holds exactly one image, with its `manifest.json` and configuration.
- The image has the `thoric.desktop=true` label.
- Its `thoric.desktop-level` label, 0 when it has none, is one this Thoric handles. An image made for a newer Thoric is refused with a reason that says to update Thoric; containers, builds and image records refuse it the same way, and the image list reports a catalog image that carries one with `"available": false` and that reason as its `error`.
- It is at most `IMAGE_ARCHIVE_MAX_MB` after decompression.

The response arrives once the whole archive has passed: `{"job_id":...}` while Docker finishes loading in that `image-import` job, or status 400 with the reason it was refused. A `size` above the limit is refused at once with 413, and a third transfer while two archives are moving with 429. Without `size`, as for `docker save IMAGE_ID | curl ... --upload-file -`, the archive is held to the limit as it arrives, the import waits up to an hour for the image's first bytes while `docker save` prepares it (see [errors](#errors)), the job's `progress` has no `total`, and an archive cut short is refused because it lacks the manifest, the configuration or a layer that loading needs. The finished job's result is the new image record, of kind `archive`, with the reference `thoric/imported:<id>`.

## Backup and restore

`GET /api/backup` streams a backup of the whole installation as a `.tar.gz` the browser saves as it is made, so one installation can be moved to another or kept aside. It always holds the installation's records:

- every container's record, with its settings, its machine identity (hostname, MAC address and machine ID) and its compose setup's state;
- each container's pinned compose and the values it was given;
- the MCP endpoints, with their grants and the hashes of their tokens. A hash does not reveal its token, and a restore drops it: a token that opened an endpoint where the backup was made, or one whose hash was written into the file, opens nothing where it is restored;
- the [scale sets](#scalable-containers), with their instances by number and their templates: the settings, the compose and the values new instances are created from.

Query parameters of `true` or `false`, all `false` by default, add the optional parts, which can be tens of gigabytes:

| Parameter | What it adds |
| --- | --- |
| `images` | The saved images, as their records with names, descriptions, icons, composes and build configurations, and each image Docker still has as a `docker save` of its ID with every layer. Catalog images stay out: the installation the backup returns to downloads them when it needs them. Without `images` the saved images stay out altogether. |
| `files` | Each container's files: a commit of its container layer made for the backup, saved the same way and removed afterwards, or the commit a settings change left a container whose Docker container is missing. Docker pauses a running desktop while it commits. Named volumes, `/tmp` and `/run` are not part of it. |
| `volumes` | The named volumes the containers and the scale sets' templates mount, also one `VOLUME_FILTER` leaves out, and those Thoric created that the volume list shows, read as a [volume export](#volume-archives) reads them. The manager's own volumes never are. |
| `secrets` | The values of the composes' secret parameters and the passwords of the containers' own proxies, also those a scale set's template keeps. Without it they are left out, and the manifest names the containers (`secrets_left_out`) and the scale sets (`scale_sets_secrets_left_out`) whose values were; a restored container whose proxy password was left out asks for it again in its settings, and a set's new instances get none. A set's template whose compose the backup cannot tell, since neither the template nor its image's record names one, leaves all its values out. |

Jobs, the applications, terminals and recordings of running desktops, the tool lists of [desktop MCP servers](#desktop-mcp-servers-and-endpoints), which the manager reads again once a desktop runs, and any record kind added later stay out. So do the installation's settings, which live in `.env` outside the manager; [the installation guide](INSTALLATION.md#move-to-another-computer) says how they move.

```bash
curl --cookie thoric.cookies --remote-name --remote-header-name \
  'http://localhost:8080/api/backup?images=true&volumes=true&files=true'
```

The backup is an ordinary tar inside gzip. Its first entry, `thoric-backup.json`, is the manifest: `kind` (`thoric-backup`), `format`, the `version` that wrote it, `created_at`, the `included` parts, the `containers`, `images`, `volumes`, `endpoints` and `scale_sets` by name, and the `parts` in order. It holds no secret value. `records.json` follows with the records. Each part is then a `docker save` stream or a volume's plain tar in entries `parts/NNNN/NNNNNNNN` of at most 4 MiB, closed by `parts/NNNN/end` with the part's `bytes` and its BLAKE2s-256 digest `blake2s`; `thoric-backup-end.json` comes last. Joined in order, a part's entries are the stream as Docker or tar wrote it, so `cat parts/0001/0* | docker load` loads an image without Thoric. tar needs an entry's size before its data, and Docker and tar tell a stream's length only when it ends, so the manager holds one entry in memory at a time and writes nothing to its disk; the browser's file is the only copy. An image and a container's files are each saved whole, so the desktop layers they share are in the backup once for each of them.

The transfer is a `backup` job, named in the `X-Job-Id` response header, whose `progress` counts the bytes sent and whose stages are `plan`, which reads the installation, then `images`, `files` and `volumes`, each skipped when the backup has no such part. If Docker or tar fails partway, the download ends with an error instead of a short file, and so does a record of the carried kinds that this release cannot read, which the error names by kind and id. One backup or restore runs at a time, another answers 409, and each takes one of the two archive transfer slots (429 while both are in use).

`POST /api/backup/check` takes the manifest as its JSON body, as the dashboard reads it from the start of the file, and answers what a restore here would meet: `{"conflicts": [{"kind": "container", "name": "Unity"}], "missing_volumes": ["projects"]}`. A conflict is a name the restore would take that this installation already uses: a container's record, Docker container, networks or the tag of its files; a saved image's record or tag; an endpoint's name; a scale set's name, kind `scale-set`, which its record or its template has; a volume the backup carries. A missing volume is one a container or a scale set's template mounts that the backup does not carry and this installation lacks.

`POST /api/backup/restore` takes the backup as an `application/octet-stream` body, with its exact `size` when it is known, as an [image import](#image-archives) does:

```bash
curl --cookie thoric.cookies \
  --header 'X-Thoric-Request: 1' \
  --header 'Content-Type: application/octet-stream' \
  --request POST --upload-file thoric-backup-20261022-134506.tar.gz \
  'http://localhost:8080/api/backup/restore?size=FILE_SIZE_IN_BYTES'
```

A restore needs `MANAGER_CONTAINER` and reads the upload as it arrives, at most `IMAGE_ARCHIVE_MAX_MB` after decompression, in a `restore` job:

- `verify` reads the manifest and the records and holds them to the rules of the routes that create the same things. A restore adds to an installation and never replaces anything in it: a backup that would take a name in use is refused with 409 and the conflicts named, before anything is created. Delete or rename those first, or restore into a new installation.
- `images` and `files` load each image through the checks an [image import](#image-archives) applies, so no name the stream carries is applied, and tag it with Thoric's own name after its record: `thoric/custom:<id>` for a script build, `thoric/snapshot:<id>` for a snapshot, `thoric/imported:<id>` for the others, whose kind becomes `archive` since this installation's copy is the restore's own. A container's files get `thoric/container:restored-<id>`. Docker gives a loaded image the ID its own image store computes, and every record that names an image ID is moved to it.
- `volumes` creates each volume anew, with the owner labels of one a `new_volume` parameter made, and unpacks it as the desktop user, keeping the modes it was saved with. The name must not exist.
- `records` saves every record at once, once the whole backup has arrived and each part's length and digest matched. The response to the upload comes here: `{"job_id": ...}`. As the create route would refuse them, each container's settings come without what this installation disables (`INTERNET_ENABLED`, `SUDO_ENABLED`, `GAMEPAD_ENABLED`, `VPN_ENABLED`, `DEBUGGER_ENABLED`, `REALTIME_ENABLED`, `PROFILER_ENABLED`, `KVM_ENABLED`, audio and microphone, and the installation's proxy without `PROXY_URL`), and without the volumes `VOLUME_FILTER` does not list here; the job log names what was switched off or left out. A scale set's template is treated the same way, and its image moves to the restored image as a container's does; a set whose image is neither a catalog image nor on this installation creates no instance until the image is here, which the job log says. The endpoints are saved without their tokens, and a set keeps its endpoint only when the backup carries that endpoint and it still covers the set.
- `containers` creates each container whose files came along from them, with its networks, its hostname, its MAC address and its machine ID, and leaves it stopped. A container is created only from a Thoric desktop image of a level this manager handles, here and on every later reset or retry, since a backup names the image it started from by ID. A container without its files is saved with an error that says so: a retry (`POST /api/containers/{id}/retry`, **Retry setup** in the dashboard) creates it from its image and runs its compose setup again. One that mounts a volume this installation lacks waits the same way until the volume is here and it is retried.

A part whose length or digest does not match, an upload that stops early and a backup without its last entry fail the restore, and the images, tags and volumes it created are removed again. The job's result names the containers it `created`, those `not_created` with the reason, the restored `volumes`, `endpoints` and `scale_sets`, the `endpoints_without_tokens` that had a token, whose agents need a new one, the containers whose secret values the backup left out, whose compose setup a reset runs without them, and the scale sets whose templates' secret values it left out, whose new instances run their setup without them.

A reset, or a retry that creates a missing container again from its image, finds the image a container started from by its ID; when Docker does not have it, as with a catalog image of the installation the backup came from, it uses the catalog image the container's settings name and downloads it when needed. A restored container whose source was a saved image the backup did not carry runs from its files, but cannot be reset.

There is no MCP tool for backups or restores: a restore creates containers and images at once, and a backup carries the installation's secrets.

## Transfer links

`/api/transfers/{token}` runs one of the transfer routes above for a client that has neither a session nor the administrator password: an agent that holds the MCP token gets the link from the MCP tool `create_transfer`, which names the operation and its arguments. The [MCP guide](MCP.md#move-large-files) lists the operations: uploading into a container or volume, downloading from one, importing or exporting a volume or image archive, and unpacking an archive into a new folder of a volume.

The request runs the route the link stands for, with that route's limits, locks, job and answer, and the arguments the link was issued with. Uploads and imports take `PUT` or `POST` with the file or archive as the body, whatever its `Content-Type`; their size is `Content-Length`, or a `size` query parameter when the body is sent without one. A file upload is refused with 411 when neither is there, while an import then takes the archive as one of unknown size, as described under [volume archives](#volume-archives). Downloads and exports take `GET`. No other query parameter is accepted.

```bash
curl --fail-with-body --upload-file unity-projects.tar.gz http://localhost:8080/api/transfers/TOKEN
```

The token replaces the cookie and the `X-Thoric-Request` header; an `Origin` header, if present, must still match the host. It is 244 random bits, held in memory for five minutes, and spent by the first request that presents it, whatever the outcome: a method the operation does not take answers 405, and a second use, an expired link or one the manager forgot in a restart answers 404. Signing out does not end a transfer a link began, while a client that stops reading or sending ends it after five minutes, as [errors](#errors) describes. Without MCP the route answers 404. The token is left out of job logs, error messages and request tracing. A link that is used is recorded in the [agent activity](#agent-activity) log under the caller whose `create_transfer` issued it.

## Desktop MCP servers and endpoints

A desktop declares MCP servers in `/etc/thoric/mcp.d/NAME.json`, which an image build writes, and in `~/.config/thoric/mcp/NAME.json`, which the desktop user writes; [the MCP guide](MCP.md#mcp-servers-inside-a-desktop) describes the files and how the manager runs them. A server as the routes list it:

```json
{"container_id": "...", "container_name": "Chrome dev", "name": "chrome", "source": "image",
 "description": "", "transport": "stdio", "command": "playwright-mcp", "args": ["--cdp-endpoint", "http://127.0.0.1:9223"],
 "cwd": null, "env": [], "url": null, "headers": [], "state": "ready", "runs_in": null, "error": null,
 "tools": [{"name": "browser_navigate", "description": "Navigate to a URL", "exposed": "chrome__browser_navigate"}], "tools_at": 1760900000, "log": ""}
```

`env` and `headers` are names only; their values, which may hold credentials, are never returned, and `url` is shown without its query, where a token goes when a server takes no header. `state` is `ready`, `pending`, `error` or `stopped`, or `elsewhere` in an instance of a [scale set](#scalable-containers) that runs the server in instance `runs_in` alone. A tool's `exposed` is its name on an endpoint of this container alone, which an endpoint's `excluded_tools` uses. Registering takes the declaration with its `name`, such as `{"name": "x", "command": "node", "args": ["/home/desktop/ext/mcp.js"]}` or `{"name": "api", "url": "http://127.0.0.1:8931/mcp", "headers": {"Authorization": "Bearer ${API_TOKEN}"}}`, needs a running container, and answers once the server's tools were read or its error is known. A build takes the same declarations in `mcp`, at most 16 with names of their own; `/api/images/{id}/config` returns them and the image list names them in `mcp_servers`, with those of the images it was built or snapshotted from.

An endpoint is created with:

```json
{"name": "chrome-dev",
 "grants": [{"container_id": "...", "alias": "chrome-dev", "scopes": ["view", "input", "files_read"], "servers": "all",
             "excluded_tools": ["container_diagnostics", "chrome__browser_run_code_unsafe"]}]}
```

`alias` defaults to the container's name and names it in the tools of an endpoint of several containers; `servers` is `"all"` or a list of server names; scopes are those of [the MCP guide](MCP.md#mcp-endpoints-for-one-agent). `excluded_tools`, optional and empty by default, names tools left out although their scope or server is granted, as the endpoint exposes them for that container alone: a container tool's own name, which must be one of the granted scopes', or `SERVER__TOOL`, the `exposed` name of the server lists, kept whether or not the server is known now. Each grant is returned with its list, empty when nothing is left out. The answer is `{"endpoint": {...}, "token": "bsk_..."}`; the token is not shown again, only `has_token` and `token_created_at`. A container created with `mcp_endpoint` true, the default, gets an endpoint with `automatic: true` and no token, which its deletion removes; an endpoint left without containers keeps its name and token. `/mcp/{name}` answers `404` with MCP disabled or for an unknown name, `401` without the endpoint's or the MCP token and `403` for a foreign `Origin`.

### The MCP inspector

`/api/mcp/inspect` is a WebSocket of the signed-in dashboard, opened from its own `Origin`, that joins the browser to one MCP server until either side closes. The browser is the MCP client and the manager carries its JSON-RPC messages without reading them, so the page sees exactly what the server sends. The first text frame names the server:

```json
{"kind": "server", "container": "CONTAINER_ID", "name": "chrome"}
{"kind": "trial", "container": "CONTAINER_ID", "server": {"name": "try", "command": "npx", "args": ["-y", "@playwright/mcp"]}}
{"kind": "endpoint", "name": "chrome-dev"}
{"kind": "administrator"}
```

A `server` is a desktop server the manager knows, and a `trial` is a declaration, as registering takes it, tried before it is saved. Both need a running container and start in a `docker exec` of their own as the desktop user, apart from the connections endpoints hold, and closing the socket ends it. Being a second process, a server that locks something of its own, such as a browser profile, may refuse to start while an agent's connection to it is open. An `endpoint` offers the tools its agents see and `administrator` those of `/mcp`; both are served inside the manager without a token, `administrator` only while the root `/mcp` is on (`ROOT_MCP_ENABLED`).

Every later frame from the browser is one JSON-RPC message, or a batch, on one line, at most 1.1 MB. Every frame from the manager is a JSON object with a `type`:

| `type` | Carries |
| --- | --- |
| `ready` | `server`, what was started, such as `{"kind": "server", "container_id": "...", "container_name": "Chrome dev", "name": "chrome", "source": "image", "transport": "stdio"}` |
| `message` | One JSON-RPC `message` from the server, as it wrote it |
| `stdout`, `stderr` | A `line` of a desktop server's output that is not a message, cut at 8 KiB, with `truncated` |
| `error` | Why the server cannot be opened, after which the socket closes, or why a frame was refused |
| `closed` | The `reason` the server stopped, after which the socket closes |

At most eight inspectors are open at once. A line of output longer than 16 MiB ends the server, as it ends an endpoint's connection.

## Agent activity

`GET /api/activity` lists what agents did, newest first: every `tools/call` on `/mcp` and on the [MCP endpoints](#desktop-mcp-servers-and-endpoints), and every [transfer link](#transfer-links) that is used. [The MCP guide](MCP.md#agent-activity) says what is recorded and what is masked. The answer is `{"items": [...], "next": 1234, "kept": 10000}`: pass `next` back as `before` for the following page, until it is `null`. `kept` is how many calls the manager keeps. Every query parameter is optional:

| Parameter | Lists |
| --- | --- |
| `container` | Calls on the container with this ID, also after it was deleted. |
| `endpoint` | Calls on the endpoint of this name. |
| `caller` | `administrator` for calls on `/mcp`, `endpoint` for calls on any endpoint. |
| `tool` | Calls of a tool, or of a desktop server, whose name contains this text in any case. |
| `kind` | `read`, `change` or `destructive`, several separated by commas. |
| `outcome` | `ok`, `error` or `refused`, several separated by commas. |
| `before` | Calls older than this ID, the `next` of the page before. |
| `limit` | At most this many calls, 1 to 200; 50 by default. |

A value no call could have, such as `kind=delete`, answers 400. A call is listed as:

```json
{"id": 1234, "at": 1760900000000, "endpoint": "chrome-dev", "credential": "endpoint_token",
 "client": "claude-code 2.1.0", "source": "tool", "tool": "write_file", "server": null,
 "container_id": "...", "container_name": "Chrome dev", "kind": "destructive",
 "arguments": {"path": "/home/desktop/notes.txt", "data": {"$masked": "string", "length": 1200}},
 "summary": "/home/desktop/notes.txt", "outcome": "ok", "error": null, "duration_ms": 180, "job_id": null}
```

| Field | Meaning |
| --- | --- |
| `at`, `duration_ms` | When the call began, in Unix milliseconds, and how long it took to answer. A download through a transfer link streams on after its answer began. |
| `endpoint` | The endpoint's name, `null` for `/mcp`. |
| `credential` | `endpoint_token`, `mcp_token`, `none` with `MCP_AUTH_MODE=none`, `inspector` for the dashboard's [MCP inspector](#the-mcp-inspector), or `link` for a transfer link, which names the caller whose tool call issued it. |
| `client` | The name and version the MCP client gives itself: a session's client, or any request of the `2026-07-28` revision. `null` for a client of an older revision on `/mcp`, which keeps no session. |
| `source`, `tool`, `server` | `tool` for a Thoric tool; `server` for a desktop server's tool, with `tool` its own name and `server` the server's; `transfer` for a link, with `tool` its operation. |
| `container_id`, `container_name` | The container the call worked on, with its name when the call began. |
| `kind` | `read`, `change` or `destructive`, from the tool's annotations. |
| `arguments` | The arguments as [masked](MCP.md#agent-activity): a value kept only by its size is `{"$masked": "string", "length": 12}`, `{"$masked": "array", "items": 2}`, `{"$masked": "object", "keys": 3}`, `{"$masked": "number"}` or `{"$masked": "boolean"}`; a text cut short is `{"$truncated": "the start", "length": 9000}`; `"$more"` counts the arguments past the first 64. |
| `summary` | The arguments that say most about the call, on one line, such as a script's first line or a path. |
| `outcome`, `error` | `ok`; `error` when the tool reported one, with the start of its text in `error`; `refused` when no tool ran, such as for an unknown tool or arguments its schema does not take. |
| `job_id` | The job the call started, for [`/api/jobs/{job_id}`](#endpoints). |

## Proxies

A container's `proxy` leads its programs through a proxy inside the desktop. `docker create` gives the container `HTTP_PROXY`, `HTTPS_PROXY` and their lowercase forms set to `http://127.0.0.1:31280`, and `NO_PROXY` and `no_proxy` set to `localhost,127.0.0.1,::1`. A container without a proxy gets the six empty, so a proxy that a snapshot's image still carries does not apply, and neither do an image's own values for them.

The desktop's proxy is a `python3` process that the manager runs as `nobody`. It sends `CONNECT` to an `http` or `https` proxy with `Proxy-Authorization: Basic`, signs in to a `socks5` proxy with the user name and password and leaves the host name to it, and forwards plain HTTP in full with `Connection: close`. The hosts of the bypass list and the desktop's own loopback go straight to their address. An upstream refusal reaches the program with its status but without `Proxy-Authenticate`, so a browser does not ask for a password of its own. The credentials reach the proxy on stdin and over a socket only root opens under `/run/thoric-proxy`, on the desktop's tmpfs. They are never in a file, an argument, `docker inspect`, a job log or a commit. Until the manager has given the proxy its settings it answers 503, so a desktop that started while the manager was away reaches nothing through it rather than everything around it.

The manager looks at every running desktop with a proxy every ten seconds and as soon as one starts. It starts the desktop's proxy when it is not running and gives it the settings when they changed, so every change but turning the proxy on or off applies without a restart. A container that follows the installation's proxy gets a changed `PROXY_URL` once the manager has started again, and goes straight to every address while the installation has none. The image needs `python3`, as the manager's other helpers do. GNOME's own proxy settings follow the variables, and `sudo` keeps them, in the catalog images from the release that added proxies; older images still give the variables to every program.

## Desktop ports

A desktop publishes no ports. The manager reaches it on the private network it shares with it, and `/api/containers/{id}/ports/{port}/{path}` forwards HTTP and WebSocket requests there, to a port the container's `ports` setting lists. A program in the desktop that serves its own API, such as an editor's MCP server, a test runner or a debugger, is then usable without a script job per request. `ports` is empty by default; it is set at creation or changed later, and a change applies at once because the manager, not Docker, does the forwarding. The service must listen on the desktop's network, such as on `0.0.0.0`: one that listens on `127.0.0.1` only is out of the manager's reach.

```bash
curl --header 'Authorization: Bearer YOUR_MCP_TOKEN' \
  http://localhost:8080/api/containers/CONTAINER_ID/ports/8090/status
```

The service receives the method, `{path}` with the query, the body and the other headers as sent, with `Host: localhost:PORT`, as a local client would. Responses stream as the service sends them, server-sent events included, and only the connection to the service has a time limit of 10 seconds. A WebSocket handshake that the service accepts joins the two connections until either side closes. A port not listed answers 403, a stopped container 409, and a service that does not answer 502.

Whatever a desktop serves would appear on the dashboard's address, and a desktop browses sites nobody vouches for, so the route keeps the dashboard's session away from it:

- It takes the MCP bearer token, as `/mcp` does, and never the session cookie, so a browser does not open it signed in. With MCP disabled it answers 404; with `MCP_AUTH_MODE=none` it needs no token, like `/mcp`. A request with an `Origin` header must come from the manager's own address.
- The `Authorization` header, the `thoric_session` cookie, `Origin` and the connection's own headers are removed before the request is forwarded, so the desktop sees no credential of the manager. Other cookies pass. A service that requires its own `Authorization` header cannot be reached this way.
- Every response carries `Content-Security-Policy: sandbox` beside the dashboard's own policy, and `X-Content-Type-Options: nosniff`. A browser that renders one, as it could with `MCP_AUTH_MODE=none`, runs no script in it and treats it as an origin of its own. `Set-Cookie` and `Clear-Site-Data` from the service are dropped, and a redirect to a path of the service stays under the proxy's prefix.

## Screenshots

`GET /api/containers/{id}/screenshot` returns a JPEG at the desktop's resolution, so its pixels match `/input` coordinates. `max_width` (16 to 7680) scales the image down, `region=x,y,width,height` crops in desktop pixels and `window_id` captures one window from the windows route. The response carries `X-Image-Width`, `X-Image-Height` and `X-Captured-At` (Unix milliseconds) headers. The dashboard requests `max_width=640` for previews. Two screenshots are taken at a time across the installation: another request waits up to five seconds for its turn and is then answered 429, a capture on a display a program holds answers 409 after about six seconds, naming the programs connected to it (see [desktop health](#desktop-health)), and a capture the desktop has not finished in 20 seconds answers 504.

`POST /api/containers/{id}/screenshots` with `{"path":"/home/desktop/Downloads/clock.jpg","replace":false,"capture":{"max_width":960}}` captures with the same options and writes the JPEG inside the desktop as the desktop user, under the upload destination rules. It returns `path`, `bytes`, `width`, `height`, `captured_at` and `download_url`; an existing file returns 409 unless `replace` is set.

`POST /api/containers/{id}/act-and-observe` takes `{"actions":[...],"settle_ms":500,"capture":{"max_width":960}}`, performs the actions, waits `settle_ms` (0 to 10000) and returns `{"input":{...},"screenshot":{"width","height","captured_at","bytes","scale","origin_x","origin_y","desktop_width","desktop_height","jpeg_base64"}}`. `scale` is image pixels per desktop pixel and `origin_x`, `origin_y` the desktop pixel at the image's top left, so a point of the image is `/input`'s `origin_x + x / scale`, `origin_y + y / scale`; each is `null` when the desktop image cannot tell it.

## Text on the screen

`POST /api/containers/{id}/find-text` with `{"text":"Save project","region":"0,0,1280,720","level":"line","min_confidence":50,"case_sensitive":false}`, every field optional, captures the screen at desktop resolution, or `region` of it, and reads it with Tesseract OCR in the manager. The capture is read in gray and enlarged up to twice, and the boxes are scaled back, so they are desktop pixels that `/input` takes as they are:

```json
{
  "width": 1280, "height": 720, "region": "0,0,1280,720", "captured_at": 1792689876000, "truncated": false,
  "lines": [{"text":"Save changes before closing?","x":226,"y":186,"width":278,"height":12,"confidence":91}],
  "text": "Save project", "found": true,
  "matches": [{"text":"Save project","x":319,"y":211,"width":75,"height":12,"center_x":356,"center_y":217,"confidence":88}]
}
```

`level` is `line` (the default), words on one row joined into a line wherever they are close, or `word`, which lists `words` instead of `lines`. Words Tesseract is less sure of than `min_confidence` (0 to 100) are left out. `text` (at most 256 bytes) is matched within one line, ignoring case unless `case_sensitive`, with a run of spaces matching one; a match covers whole words. An answer lists at most 500 lines or words and 50 matches, and `truncated` says when there were more. Tesseract reads English. A reading that takes longer than 30 seconds answers `504`, and a screenshot that is no JPEG a desktop makes `502`. A manager without Tesseract, such as a build run outside the manager image, answers `501`, which only this route uses, and names the Debian packages it needs, `tesseract-ocr` and `tesseract-ocr-eng`.

## Mouse and keyboard

POST to `/api/containers/{id}/input` with an `actions` array:

```json
{
  "actions": [
    {"type":"click","x":400,"y":250,"button":"left","count":1},
    {"type":"key_press","keys":["Control","a"]},
    {"type":"type_text","text":"Hello"},
    {"type":"key_press","keys":["Enter"]}
  ]
}
```

The response includes `applied_actions`, `width`, `height`, the final `pointer` position, `delay_ms` and `released`, which lists keys and buttons the request left held and the manager released. Coordinates use the remote desktop's pixels. Other actions are:

```json
[
  {"type":"move","x":100,"y":100},
  {"type":"drag","from_x":100,"from_y":100,"to_x":300,"to_y":200,"button":"left","duration_ms":300},
  {"type":"scroll","x":300,"y":200,"direction":"down","steps":3},
  {"type":"wait","milliseconds":500},
  {"type":"move_relative","dx":40,"dy":-10,"steps":4,"duration_ms":200},
  {"type":"button_down","x":100,"y":100,"button":"left"},
  {"type":"button_up","button":"left"},
  {"type":"key_down","keys":["w","Shift"]},
  {"type":"key_up","keys":["Shift"]},
  {"type":"hold_keys","keys":["space"],"milliseconds":250},
  {"type":"release_all"}
]
```

Put the actions you need in your request's `actions` array. Batches allow 1 to 100 actions, single waits or holds of at most 10 seconds and at most 30 seconds of combined delays. Keys and buttons held by `key_down` and `button_down` stay pressed for later actions in the same request and are released when the request ends. Input travels through a shared RFB connection with Fence acknowledgement; it does not disconnect the browser viewer. `move_relative` starts from the pointer location reported by the desktop and is the exception: RFB carries only positions, so it is sent as relative motion through XTest with `xdotool` inside the desktop, one `docker exec` per action after the input before it is acknowledged, and programs that read raw motion get its deltas even at the desktop's edge. A request with one reports `relative_motion`, `{"method":"xtest"}`, or `{"method":"emulated","reason":"..."}` when the desktop has no `xdotool`, or a program holds its display, and the motion was sent as absolute positions; `pointer` is where the pointer ended, where a later `button_down` or `button_up` without coordinates acts. See the [MCP input guide](MCP.md#use-the-mouse-and-keyboard) for key names and limits.

## Clipboard

`PUT /api/containers/{id}/clipboard` with `{"text":"Merhaba dünya, 你好 👋","selection":"clipboard","paste":true,"paste_keys":"ctrl+shift+v"}` puts the text on the desktop's clipboard: at most 512 KiB of UTF-8 without NUL characters, in any language. `selection` is `clipboard` (the default), `primary`, the text a middle click pastes, or `both`. With `paste` it then presses `paste_keys`, `ctrl+v` (the default) or `ctrl+shift+v` for a terminal, as `/input` presses keys, so the focused application pastes the text; `paste` needs `clipboard` or `both`. The answer has `selection`, `bytes`, `characters` and, after a paste, `pasted` with the keys pressed. `GET /api/containers/{id}/clipboard?selection=primary`, `clipboard` by default, answers `selection`, `text`, `bytes` and `truncated`: `text` is `null` when nothing holds the selection or it holds something other than text, such as a picture, and `truncated` is true when it held more than 512 KiB, of which `text` is the start. Both run a helper as the desktop user through GTK 3 on the desktop's display; a set leaves a process in the desktop that owns the selection and serves the text until another program takes it, since an X11 selection ends with its owner. An empty `text` is set as it is and reads back as `""`. A stopped container answers 409, a desktop image without GTK 3 for Python, from before Thoric 0.6.3, answers 400 naming it, a program holding the selection that does not hand its text over within 10 seconds answers 504, and a desktop whose display cannot be opened answers 502. The 512 KiB leave room in the 1,100,000 bytes of a request for a text whose every byte JSON escapes to two, such as one of newlines and quotes; a client that writes other characters as `\u` escapes sends up to six bytes for each. The text travels only in the request and the answer, never on a command line.

## Applications and windows

POST `{"command":"glxgears","args":["-info"],"cwd":"/home/desktop","gpu":true,"env":{"MY_SETTING":"1"},"name":"gears"}` to `/api/containers/{id}/applications`. The program runs in the background as the desktop user; `gpu` runs it through `gpu-run` and needs a GPU-enabled container. The response includes `application_id`, `state`, `pid` and, once the program ends, `exit_code` or the terminating `signal`. States are `starting`, `running`, `exited`, `failed` and `unknown`.

`GET .../applications/{application_id}/logs?tail_bytes=65536` returns the end of the combined stdout/stderr log, which keeps about the latest 1 MiB. `POST .../applications/{application_id}/stop` with an optional `{"grace_seconds":5}` body sends SIGTERM to the process group, then SIGKILL. Supervisor files live under `/tmp` in the desktop and disappear when it restarts, after which the state is `unknown`.

`GET .../applications/{application_id}/diagnostics` returns the application's state, exit status, its session's processes with memory use, open file count, thread count and the latest log lines. `GET .../applications/{application_id}/metrics?duration_ms=1000` samples CPU time and resident memory of those processes over the window (100 to 10000 ms) and reports container memory from cgroups; frame rate is not available.

`GET .../processes?sort=cpu&limit=50&query=chrome&interval_ms=500` lists every process of a running desktop, whatever started it, as the MCP tool `list_processes` describes: `sort` is `cpu` (default), `memory` or `pid`, `limit` 1 to 1000 (default 50), `query` at most 200 characters matched against the name, the command line and the user, ignoring case, and `interval_ms` (100 to 5000, default 500) the time CPU use is measured over. It answers `{"processes":[{"pid":412,"ppid":87,"uid":1001,"user":"desktop","state":"S","name":"chrome","command":"/opt/google/chrome/chrome --no-first-run","role":"program","threads":31,"rss_bytes":268435456,"cpu_percent":12.5,"cpu_seconds":41.73,"started_at":1793876400}],"total":64,"matched":64,"cpu_percent":15.3,"rss_bytes":1073741824,"memory_bytes":1395864371,"memory_limit_bytes":null,"cpus":8.0,"interval_ms":503}`, where `cpu_percent` is 100 for one full core and `role` is `desktop`, `thoric` or `program`. A stopped container is a `409`.

`POST .../processes/{pid}/kill` with an optional `{"signal":"term","tree":false,"root":false,"wait_ms":2000}` sends `term` (the default), `kill`, `int` or `hup` to the process and, with `tree`, to its descendants, parents first, then waits up to `wait_ms` (at most 10000) and answers `{"pid":412,"signal":"term","signalled":[412,415],"ended":[412,415],"running":[],"refused":[]}`. It goes as the desktop user unless `root` is `true`; a process of another user without `root` is a `409` that names the user, a process that is not there a `404`, and PID 1 a `400`, since the container stops with it. The request shares the container's lock, as input and scripts do, and counts as use for its [idle stop](#lifecycle).

`POST .../wait` with `{"window_title":"Game","application_id":"...","application_state":"running","log_contains":"ready","screen_stable_ms":1000,"region":"0,0,800,600","timeout_ms":10000,"interval_ms":500}` polls until every supplied condition holds and returns `satisfied`, `elapsed_ms` and the last observation; after `timeout_ms` (100 to 60000) it returns `satisfied:false` rather than an error. A check that could not look, for a screenshot slot that stayed taken (429) or a step that ran out of time (504), is not an error either: the wait goes on, and when the last check could not look the answer carries its reason in `last_error`. A display a program holds ends the wait with the `409` that [desktop health](#desktop-health) describes, once a check has had the six seconds or so it takes to find that out; a shorter check runs out of time instead. `screen_changed` compares with the first screen the wait could capture, which comes later while the slots stay taken. No check runs past `timeout_ms`, except that the first may take up to five seconds, so a shorter wait still looks once. `application_state` defaults to `exited`, which also matches `failed`. `{"screen_changed":true,"screen_change_percent":0.5}` waits until the screen, or `region`, differs from how it looked when the wait began: `screen_change_percent` (0.01 to 100, default 0.5) of its pixels must have turned noticeably brighter or darker, and the answer reports `screen_changed_percent`. `{"text":"Loading","text_state":"gone"}` waits until the text is no longer read on the screen, or `region`, and `visible`, the default, until it is; the text is read and matched as [`find-text`](#text-on-the-screen) does, and the answer reports `text_found` and `text_match`, the first place it was read.

`POST .../reset` recreates the container from its saved source image ID and returns a job. Unsaved changes in the container layer are discarded; named volumes, networks and the container ID are kept, and the data inside volumes is not touched. The reset is refused when the source image is missing.

`POST .../recordings` with `{"fps":2,"max_seconds":60,"max_width":960,"region":"0,0,1280,720","name":"level 1"}` starts capturing JPEG frames into `/tmp/thoric-recordings/{recording_id}` inside the desktop and returns `recording_id`. Recording stops at `max_seconds`, after 3000 frames or 256 MiB, or on `POST .../recordings/{recording_id}/stop`, which returns `frames`, `bytes`, `started_at`, `ended_at` and `reason`. Frames are named `frame-00000-<unix ms>.jpg`, the moment each was captured, and can be read with the file routes. Stopping also encodes them, as the desktop user and for at most 90 seconds, into `recording.mp4`, H.264 in MP4 that browsers play, each frame shown from its capture time and the picture scaled down to the area of 3840x2160 when larger. The result describes it under `video` as `{"path":".../recording.mp4","download_url":"/api/containers/{id}/files/download?path=...","codec":"H.264","container":"MP4","mime_type":"video/mp4","width":960,"height":540,"frames":120,"duration_ms":60000,"bytes":812345}`. A desktop image without GStreamer's `x264enc`, as from Thoric 0.8.4 or earlier, or an encoding that runs out of time gets `recording.avi` instead, an MJPEG video playable with VLC, mpv or ffmpeg at the recording's `fps`, with `"codec":"MJPEG","container":"AVI","fps":2` and the reason in `fallback`. A video that could not be made at all is `{"error":"..."}`. The list returns the same `video` once a recording has one; one that ended at a limit gets it when it is stopped or exported. `POST .../artifacts` with `{"destination":"/home/desktop/Downloads","name":"run.tar.gz","recording_ids":[...],"application_ids":[...],"replace":false}` writes an archive containing `recordings/<id>/` frames and video, `applications/<id>/` logs and a `manifest.json`, whose `videos`, also in the result, maps each recording to its video in the archive; the destination follows the upload rules and existing archives return 409 unless `replace` is set. Recording files live in tmpfs and disappear when the desktop restarts.

## Audio, terminals, accessibility and gamepad

`POST .../audio-capture` with `{"seconds":3,"source":"desktop"}` records what the desktop plays (or, with `microphone`, what reaches its microphone) as 16-bit stereo 48 kHz WAV and returns `wav_base64`, `peak`, `rms` and `silent`. `POST .../audio-inject` with `{"data_base64":"...","target":"microphone"}` plays 16-bit PCM WAV data into the desktop's microphone (needs the microphone option) or, with `speakers`, into its audio output. Both need audio enabled for the container.

`POST .../terminals` with an optional `{"cwd":"/home/desktop","root":false}` opens bash on a pseudo-terminal and returns `terminal_id`. `POST .../terminals/{terminal_id}/input` with `{"text":"ls","press_enter":true}` types into it; `GET .../terminals/{terminal_id}/output?offset=0&limit=65536&strip_ansi=true` returns `output`, `next_offset`, `more` and the shell state, keeping about the latest 1 MiB. `POST .../terminals/{terminal_id}/close` hangs up the shell, kills one that ignores the hangup and forgets the terminal with its output. The list leaves out terminals that ended with a desktop restart or stop. Root shells are an explicit administrator choice. A terminal an MCP endpoint opened names it in `endpoint`; that endpoint reaches it and no other terminal.

`GET .../ui?pid=1234&name=&max_depth=6&max_nodes=500` returns the AT-SPI accessibility tree of applications that expose one (GTK, Electron, Qt with `QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1`): roles, names, states, screen extents, text and actions. Games usually expose nothing.

`POST .../gamepad` with `{"actions":[{"type":"button","button":"a"},{"type":"axis","axis":"left_x","value":0.5},{"type":"wait","milliseconds":100},{"type":"reset"}]}` drives a virtual Xbox-style controller in a container created with `gamepad:true`. The device is created on first use as `/dev/input/eventN` inside the desktop and keeps button and axis state between requests until `reset` or a desktop restart. A request that arrives while another one creates it waits for that, up to 20 seconds, then answers 409, so a desktop never gets two devices. Buttons: a, b, x, y, lb, rb, back, start, guide, ls, rs. Axes: left_x, left_y, right_x, right_y (-1 to 1), left_trigger, right_trigger (0 to 1), dpad_x, dpad_y (-1, 0, 1). SDL applications that use udev can set `SDL_JOYSTICK_DISABLE_UDEV=1` to scan `/dev/input` directly.

`GET .../windows` lists visible windows with `window_id`, `title`, `pid`, `x`, `y`, `width`, `height` and `active`. `POST .../windows/{window_id}/focus` activates a window, and `POST .../windows/{window_id}/geometry` with `x`,`y` and/or `width`,`height` moves or resizes it and returns the resulting geometry. Window IDs are hexadecimal X11 IDs such as `0x3a00007`. The window routes, window screenshots and relative pointer movement need `xdotool` and `wmctrl` in the desktop image.

## Resource usage

The manager follows the Docker stats stream of every running container and keeps one sample a second for the last 30 minutes. The history lives in memory only: a manager restart starts it over, and stopped and deleted containers keep their samples until they are 30 minutes old. It needs the Docker Engine socket; when `DOCKER_HOST` names a TCP address instead, there is no history.

`GET /api/usage` returns:

- `available` and `error`: whether Docker's running containers can be read right now. The samples already taken stay while it cannot.
- `interval_ms` (1000) and `window_ms` (1800000).
- `cursor`: the stamp of the newest sample. Pass it back as `since` to receive only the samples stamped after it; two requests in a row miss nothing.
- `containers`: each container that has samples, with `id`, `name`, `cpu_limit` (its CPU limit, or `null`), `removed`, `memory_limit` (bytes, the host's memory when it has no limit), `online_cpus` (the host CPUs it sees) and `samples`. A container stays listed with an empty `samples` while nothing new arrived, so a client can drop the ones that are no longer listed. A deleted container stays listed with `removed: true`, under the name and CPU limit it last had, until its samples age out; it comes after the existing ones.
- `gpu`: the host's GPU, below.

Each sample has `t`, the Unix time in milliseconds when it was stored, and:

| Field | Meaning |
| --- | --- |
| `cpu` | CPU cores in use: the CPU % of `docker stats` divided by 100. It is measured against the host's CPUs, not the container's limit, so a container limited to two CPUs reads at most 2. |
| `memory` | Bytes in use without the page cache, as `docker stats` counts them. |
| `disk_read`, `disk_write` | Bytes per second through the container's block I/O counters. The desktop's tmpfs folders and shared memory are not disk I/O. |
| `net_rx`, `net_tx` | Bytes per second received and sent on all of the container's networks. |

`gpu.state` is `sampling`, `idle` while no GPU container runs, `unavailable` with a `reason`, or `disabled` when the installation gives desktops no GPU (`gpu_enabled` is false). With only the `/dev/dri` route it is `unavailable`, since `nvidia-smi` reads NVIDIA GPUs alone. Each of `gpu.devices` has `index`, `name`, `memory_total` and samples of `t`, `utilization` (percent) and `memory` (bytes); a figure the driver does not report is `null`. These are the whole GPU's figures, which `scope: "host"` says: WSL gives Linux a paravirtual GPU whose driver reports neither processes nor containers, and Windows applications count too; on a Linux host the host's other programs count as well. They come from `nvidia-smi`, so only NVIDIA GPUs report them. While a GPU container runs, the manager runs it in a helper container from its own image that gets the GPU device, no network, no capabilities and a read-only root, and ends with the manager.

`GET /api/containers/{id}/usage` returns the same header with one container's fields and samples, and no GPU. Its `samples` stay empty until the container has run since the manager started. A deleted container's ID is still answered while its samples are in the window, and returns 404 once they have aged out.

## Prometheus metrics

`GET /metrics` answers in Prometheus' text format, version 0.0.4 (`text/plain; version=0.0.4; charset=utf-8`), for a scraper that keeps the history the usage charts drop after 30 minutes. It stays outside `/api`: a scraper sends `Authorization: Bearer <METRICS_TOKEN>` and holds no session, so neither the cookie nor `X-Thoric-Request` is asked for, while the network and Host checks of every route still come first. The route answers `404` while `METRICS_TOKEN` is not set, also with `AUTH_MODE=none`; without the token, or with the MCP token, it answers `401` with `WWW-Authenticate: Bearer realm="Thoric metrics"`. Wrong tokens count against the client's address, as failed sign-ins do but in a count of their own, and an address that sent 20 within a minute gets `429` for up to a minute, the right token included. A successful scrape is logged at the debug level only.

```bash
curl --header "Authorization: Bearer $METRICS_TOKEN" http://localhost:8080/metrics
```

A scrape asks Docker nothing. It reads the usage collector's latest listing of the containers, made every two seconds, and the latest Docker stats message of each running one, read once a second, with the host GPU's latest sample while it is sampled, the container and image records, the jobs that have not ended, the requests for a person, and counters the manager keeps from its start of the jobs that ended, the MCP tool calls and the transfer links used. Only `thoric_volumes` needs a Docker listing, which a scrape starts in the background once the last count is a minute old. Every series has a `# HELP` and a `# TYPE` line, and its labels are a container's `id` and `name`, a Thoric tool's name in the catalog (`unknown` for any other name a call gave) or small fixed sets. The GPU series are the host's, named `thoric_host_gpu_*` with the GPU's `gpu` index and `name`, and no container's series names a GPU, since WSL reports no container's share. [Installation](INSTALLATION.md#prometheus-metrics) lists every series with a Prometheus configuration.

## Event stream

`GET /api/events` tells a signed-in client what happens as it happens, so it does not have to poll for it. It is a [server-sent event](https://html.spec.whatwg.org/multipage/server-sent-events.html) stream that a browser reads with `EventSource`, which sends the session cookie, and that curl follows with `--no-buffer`:

```bash
curl --no-buffer --cookie thoric.cookies http://localhost:8080/api/events
```

Every event is unnamed, and its data is one JSON object whose `type` says what it is:

| `type` | Fields | Sent when |
| --- | --- | --- |
| `ready` | `docker_events` | The stream opened; always the first event. `docker_events` is whether Docker's own events reach the stream, as `docker-events` reports it. |
| `job` | `job`: `id`, `kind`, `resource`, `name`, `status`, `created_at`, `finished_at` and `error` | A job was queued, started, began a [stage](#create-a-container-and-follow-the-job) or ended. `status` is as `GET /api/jobs/{job_id}` reports it; `error` is the first line of a failed job's error, at most 300 characters, and `null` otherwise. The job's log and result stay on its route. |
| `job-progress` | `id`, `progress` | A running job's `progress` changed, such as a download or a [script](#script-progress), as its route reports it: at most once a second for each job, and `null` when the measured step ended. |
| `human-request` | `request` | An agent asked for a person; `request` is as [`GET /api/human-requests`](#requests-for-a-person) lists it. |
| `human-request-answered` | `request` | Someone answered a request for a person, on any page or through the API; `request` has its `status`, `reply` and `answered_at`. |
| `changed` | `lists` | Docker changed these lists by itself, without a job of this manager: `containers` when one of Thoric's containers is created, starts, stops or exits, is renamed, paused or removed, or its health check changes; `images` when an image is pulled, tagged, untagged, deleted, imported or loaded; `volumes` when a volume is created or removed. A burst is one event; read those lists again. |
| `docker-events` | `live` | Docker's own events began (`true`) or stopped (`false`) reaching the stream, such as while Docker restarts. The events in between are lost, so read `containers`, `images` and `volumes` again when it turns `true`. |
| `resync` | | The client fell more than 256 events behind and missed some; read again what it shows. |

Nothing is replayed: a client reads the lists it shows when it connects and again after any gap, such as a lost connection or a `resync`, since the events of that gap are gone. A comment line every 15 seconds keeps an idle stream open through proxies. The stream ends when its session signs out or expires, and when the manager stops; reconnect with a growing delay, and sign in again when the reconnection answers `401`. The dashboard keeps one stream for each open page, closes it a minute after the page goes to the background and reconnects when it comes back. While the stream is up and `docker_events` is `true`, it reads containers, images, volumes and the system summary once a minute and when `changed` names them, instead of every 8 seconds. The manager follows Docker's events only where `DOCKER_HOST` is a Unix socket; elsewhere `docker_events` stays `false` and those lists are polled as before.

A proxy in front of the manager must pass the stream on as it arrives. The bundled Caddy configuration does, since Caddy flushes `text/event-stream` responses at once, and the manager sends `X-Accel-Buffering: no` for nginx.

## Requests for a person

An agent that reaches something only a person may do in its desktop, such as a CAPTCHA, a second sign-in factor, a license window or a sign-in screen, asks with the MCP tool [`request_human`](MCP.md#ask-a-person-for-help) and waits. `GET /api/human-requests` lists the requests still waiting, oldest first:

```json
{"items": [{
  "id": "5f0c...", "container_id": "a1b2...", "container_name": "Chrome dev",
  "message": "Solve the CAPTCHA on the sign-in page in Chrome",
  "endpoint": "chrome-dev", "status": "pending", "reply": null,
  "created_at": 1792689876, "answered_at": null
}]}
```

`container_name` is the container's name when the agent asked, kept if the container is deleted meanwhile, and `endpoint` the [MCP endpoint](#desktop-mcp-servers-and-endpoints) it asked through, `null` for `/mcp`. `POST /api/human-requests/{id}/answer` with `{"answer": "done"}`, or `{"answer": "decline", "reply": "No phone at hand"}`, answers it: `status` becomes `done` or `declined`, a `reply` of at most 1,000 characters goes to the agent, and the waiting call returns at once. The first answer counts; a second one gets `409`. Deleting a container declines its open requests, and deleting an MCP endpoint those asked through it, with a `reply` that says so. Each new request and each answer is also an [event](#event-stream).

Requests live in the manager's memory, at most 64 waiting at once, and an answered one can be read by its agent for an hour. A manager restart ends them all: a waiting call then fails with an error saying so, and the agent has to ask again.

## Updates

Unless `UPDATE_CHECK=false`, the manager asks GitHub for the newest release when it starts and every six hours, and a manager that follows `edge` asks Docker about it at the same time, as `edge` below says. `POST /api/update/check` asks at once and answers with the same fields as `GET /api/update` once GitHub has answered, within 30 seconds. A check asked for while another runs waits for that one and takes its answer, so a second click makes no second request; GitHub allows a host 60 such requests an hour. It answers 409 while the lookup is turned off. A failed check is not an error of the request: `error` says why, as below. `GET /api/update` returns:

- `enabled`: whether the manager looks for releases at all.
- `current`: this manager's release.
- `commit`: for an `edge` build of `master`, the commit it was built from, since it still carries the version of the release before it; `null` for a release or a build from the source.
- `newest`, `release_url` and `checked_at`: the newest release, the page of its notes and when GitHub was last asked, `null` before the first answer. `error` says why the last look failed; `newest` then keeps the release seen before.
- `available`: whether `newest` is later than `current`.
- `edge`: `null`, unless the manager's container, the one `MANAGER_CONTAINER` names, was created from a reference that names the `edge` tag, such as `ghcr.io/co3moz/thoric:edge`. Then `{"image": "ghcr.io/co3moz/thoric:edge", "available": true, "error": null, "changes_url": "https://github.com/co3moz/thoric/compare/<commit>...master"}`: the reference, whether the tag names another image than the manager's now, which Docker's `/distribution` endpoint tells without downloading it, why the last look at the tag failed, `available` then being what the look before found, and the commits on `master` since this build, `null` without its commit. An edge build carries the version of the release before it, so for such a manager the update the dashboard offers is the newer build, whatever `available` says.
- `installable`: whether this installation runs the update itself, which only the [WSL package](WSL_PACKAGE.md#update-from-the-dashboard) does.
- `job_id`: the running update job, or `null`.

`POST /api/update` starts that update and returns a job. It answers 409 while the lookup is turned off, on an installation other than the WSL package, without a newer release, or for a manager that follows `edge` without a newer build of it, and while any job runs, since every job ends with the manager. The job first copies the manager's database beside it, as `state-<this release>-to-<the newest>.sqlite`, or `-to-edge.sqlite` for an update of `edge`, keeping the newest three, and fails before anything else when the copy cannot be written; see [Go back after an update](WSL_PACKAGE.md#go-back-after-an-update). The manager then leaves a request in the folder the package mounts at `/update`, and the distribution runs `thoric update` as root, which moves `MANAGER_IMAGE` in `/opt/thoric/.env` to the newest release, or downloads `edge` again, and recreates the manager. The job's log is that command's output. The new manager finds the job still running and follows it to its end; sessions end with the old manager, so sign in again.

## Notifications

`NOTIFY_URL` names where notices go, and `NOTIFY_EVENTS` which are sent; [Get notified on your phone](USAGE.md#get-notified-on-your-phone) sets them up. The manager sends each notice with one POST to every address, through `curl` with the address and its headers on stdin, within 15 seconds, and never makes a request wait for it: a notice that finds 64 others waiting is dropped. An address that fails is logged at warn when it begins failing or its reason changes and at info once it works again, named by its place in `NOTIFY_URL`, its format and its host alone. A request for a person sends one notice, however often its agent waits on it again or asks it again while it is open.

`POST /api/settings/notify/test` sends a test notice to every address at once, whatever `NOTIFY_EVENTS` says, and answers once each has answered:

```json
{"results": [
  {"format": "ntfy", "host": "ntfy.sh", "ok": true, "status": 200, "answer": "{\"id\":\"...\",\"topic\":\"[hidden]\"}", "error": null},
  {"format": "discord", "host": "discord.com", "ok": false, "status": 404, "answer": "{\"message\": \"Unknown Webhook\", \"code\": 10015}", "error": "Discord at discord.com answered HTTP 404: ..."}
]}
```

`format` is `ntfy`, `discord`, `slack`, `telegram` or `json`, and `host` the address's host: the rest of an address can hold a credential, so it never appears, and `[hidden]` stands for any part of it in `answer`, at most 300 characters of what the address answered, and in `error`, why it did not take the notice. `ok` is true for a 2xx `status`; `status` is `null` when nothing answered. Without `NOTIFY_URL` the route answers `409`.

An address that is not ntfy, Discord, Slack or Telegram gets this JSON payload:

```json
{"kind": "human_request", "title": "Unity needs a person", "body": "Solve the CAPTCHA in Chrome", "container": "Unity", "link": "https://thoric.lan:8443/#/desktop/3f2a...", "time": 1793700000}
```

`kind` is `human_request`, `job_failed`, `update_available`, `container_lifecycle` or `test`, `title` at most 120 characters on one line and `body` at most 1,000. `container` is the name of the container it concerns, `null` for none. `link` is where to look: the desktop that asks for a person, the **Containers** page for a container its [lifecycle](#lifecycle) stopped or the **Jobs** page under `DASHBOARD_URL`, `null` without it, and a new release's notes on GitHub. `time` is when it was sent, in seconds since 1970. A notice carries no secret value and nothing a tool returned: a failed job names its kind and what it worked on, and its error stays in its log.

## Configuration

`GET /api/configuration` returns `{"variables": [...], "editable": ..., "job_id": ..., "exposure": ..., "docker": ...}`. `variables` are the settings of `.env.example` that Compose passes to the manager, in the order the dashboard's **Management** page lists them. Each has the `name` it has in `.env` and the `value` the manager runs with, read once when it started: a string, a number for the limits in MiB, a boolean for the switches or `null` for a name left unset. A value is what the manager made of its environment rather than the text it was given, so an empty `TEMPLATES_URL` reads as the wiki's list and `none` as `"none"`. `ALLOWED_HOSTS` is the list `.env` gives, and `DESKTOP_HOST`, which the manager answers too, comes on its own. `ADMIN_PASSWORD`, `MCP_TOKEN`, `METRICS_TOKEN`, `PROXY_PASSWORD` and `NOTIFY_URL` come as `{"name": ..., "secret": true, "set": ..., "editable": ...}`, without a value: none ever leaves the manager. Every setting says whether the Management page may change it, `"editable"`, which of the secrets only `NOTIFY_URL` is, and an editable one that is not a secret also has its `text`, the value as its environment gave it, empty when it gave none. `editable` at the top says whether this installation takes changes from the dashboard: only the WSL package does, once its `thoric-settings.path` unit watches the folder it mounts at `/update`. `job_id` is a change that is running. `exposure` is `null` unless something answers without a credential while Docker publishes the manager beyond loopback: then `{"open": ["dashboard", "mcp"], "published": ["*:8080"]}` names what is open, the dashboard and the API with sign-in off and `/mcp` without a token, and where, `*` for every address, since every device that reaches it and every desktop with internet access is then an administrator. `docker` is `null` unless the Docker Engine answers within 5 seconds and lacks some of what Thoric uses: then `{"version": "27.5.1", "api": "1.47", "lacks": ["isolation", "mac_address"], "package": false}` names the engine, what it lacks, `isolation` (desktops' private networks without an address on the Docker host, Docker Engine 28.0) and `mac_address` (a desktop's MAC address kept on its internet network, Docker Engine 29), and whether the WSL package, whose `thoric update` and `thoric system` keep Docker Engine current, runs the manager.

`POST /api/configuration` changes settings, with `{"changes": {"NAME": "text", ...}}`, the text each setting gets in `.env`. The editable ones are `SECURE_COOKIE`, `ALLOWED_HOSTS`, `DESKTOP_IMAGE`, `DESKTOP_USER_NAMESPACES`, `DESKTOP_PIDS_LIMIT`, `DESKTOP_ADDRESS_POOL`, `WSL_GPU`, `NVIDIA_COMPUTE`, `GPU_ADAPTER`, `AUDIO_ENABLED`, `MICROPHONE_ENABLED`, `INTERNET_ENABLED`, `SUDO_ENABLED`, `GAMEPAD_ENABLED`, `VPN_ENABLED`, `DEBUGGER_ENABLED`, `REALTIME_ENABLED`, `PROFILER_ENABLED`, `KVM_ENABLED`, `GPU_DEFAULT`, `INTERNET_DEFAULT`, `SUDO_DEFAULT`, `AUDIO_DEFAULT`, `MICROPHONE_DEFAULT`, `GAMEPAD_DEFAULT`, `VPN_DEFAULT`, `DEBUGGER_DEFAULT`, `REALTIME_DEFAULT`, `PROFILER_DEFAULT`, `KVM_DEFAULT`, `IDLE_STOP_DEFAULT`, `UPLOAD_MAX_MB`, `IMAGE_ARCHIVE_MAX_MB`, `TEMPLATES_URL`, `VOLUME_FILTER`, `PROXY_URL`, `PROXY_USERNAME`, `PROXY_BYPASS`, `UPDATE_CHECK`, `NOTIFY_URL`, `NOTIFY_EVENTS` and `DASHBOARD_URL`; sign-in, the root MCP and its token, `METRICS_TOKEN`, `PROXY_PASSWORD`, `MANAGER_CONTAINER` and what only Compose reads change in `.env` alone. `NOTIFY_URL` stays a secret: its text replaces the addresses, an empty one removes them, and the job's log names it as `NOTIFY_URL=[hidden]`. A text must be one line of at most 2048 printable ASCII characters, without quotes, `$`, `#`, backslashes, backquotes or spaces at either end, which `.env` would read otherwise, and the manager reads each change with the code it starts with, alone and together, so a value it would refuse when it starts again is refused before anything restarts. With `"dry_run": true` nothing changes and the answer is `{"errors": {"NAME": "..."}, "error": ..., "running_jobs": n}`: what is wrong with each setting, what is wrong with them only together, and how many jobs a restart would interrupt. Otherwise a change answers 400 with the problems, 400 when every value is the one the manager has, and 409 on an installation other than the WSL package, while an update or another change runs, and while other jobs run unless `"interrupt_jobs": true`, since they end with the manager. It returns a job of kind `settings`: the manager leaves the change in the folder, and the distribution runs `thoric apply-settings` as root, which accepts only these settings, checks each value again, rewrites only their lines of `/opt/thoric/.env`, keeps the previous file as `.env.previous` and recreates the manager. When the new manager does not answer within 90 seconds, it puts the previous file back and recreates the manager again, and the job fails. The job's log is that command's output, and the next manager follows the job to its end; sessions end with the old manager, so sign in again.

## Dependencies

`GET /api/dependencies` returns the list the dashboard's **Dependencies** page shows, which `core/manager/dependencies.py` writes while the manager image builds, so it is the list of that build: `{"format": 1, "version": "...", "platform": "linux/amd64", "dependencies": [...]}`, with `version` the release built and `platform` the one the image was built for, `linux/amd64` or `linux/arm64`. A list from a release up to 0.9.1 has no `platform` and is linux/amd64's. A manager built outside the image, such as with `cargo run`, answers `404`. Each dependency has `group`, `name`, `version` (`null` for a system package the image takes at the version its distribution has) and, where known, `license`, `description` and `url`, and:

| Group | Fields |
| --- | --- |
| `ui` | The interface's packages from `core/web/bun.lock` for the platform, with the license their installed `package.json` names. `scope` is `runtime` for what the interface's own dependencies reach, `development` for what only the build tools do, and `direct` whether `core/web/package.json` names it. |
| `backend` | The manager's crates from `cargo metadata` for the platform's target, `x86_64-unknown-linux-gnu` or `aarch64-unknown-linux-gnu`. `scope` is `runtime` for what is compiled into the binary, `build` for what only build scripts and procedural macros use, `development` for what only tests do, and `direct` whether `core/manager/Cargo.toml` names it. |
| `infrastructure` | What the Dockerfiles, Compose manifests and workflows use. `kind` is `image`, `package` (an APT package, with `source`, the image of the stage that installs it), `tool`, `download` (a GitHub release), `action` (with `pin`, the commit it is pinned to) or `runner`, and `used_in` lists where: `manager`, `desktop`, `wsl`, `compose` or `ci`. |

## Skills

The skills of the repository's `docs/skills/`, such as `thoric-scale-sets`, are built into the manager, so each matches the tools of the release that serves it. The web serves them without a session, since every file is public in the repository: `GET /.well-known/agent-skills/index.json` is the index of the [Agent Skills discovery RFC](https://github.com/cloudflare/agent-skills-discovery-rfc) 0.2.0,

```json
{"$schema": "https://schemas.agentskills.io/discovery/0.2.0/schema.json",
 "skills": [{"name": "thoric-scale-sets", "type": "skill-md", "description": "How to work through a Thoric scale set, ...",
             "url": "/.well-known/agent-skills/thoric-scale-sets/SKILL.md", "digest": "sha256:..."}]}
```

and each `url` answers with the file, so one command copies a skill into an agent's skills folder:

```bash
mkdir -p ~/.claude/skills/thoric-scale-sets && curl -fsSL http://localhost:8080/.well-known/agent-skills/thoric-scale-sets/SKILL.md -o ~/.claude/skills/thoric-scale-sets/SKILL.md
```

The dashboard's **Add to a client** dialog writes that command for each client's own folder, for a POSIX shell and for PowerShell. Over MCP the same skills are resources of `/mcp` and of a scale set's endpoints, as [the MCP guide](MCP.md#skills) describes.

## OpenAPI document

`GET /api/openapi.json` returns this API as an [OpenAPI 3.1](https://spec.openapis.org/oas/v3.1.0) document, which client generators and API tools read. It needs the session like the other routes here, so send the cookie, or nothing with `AUTH_MODE=none`:

```bash
curl --cookie thoric.cookies http://localhost:8080/api/openapi.json > thoric-openapi.json
```

The manager makes the document from its own code, so it describes the release that serves it. It lists every route of the [table above](#endpoints), with `/mcp`, the transfer links, the desktop ports, the skills, `/healthz`, `/readyz` and `/metrics`, and for each:

- its path and query parameters and its request body. The JSON Schemas of the bodies and query strings are those of the types the manager reads them into, with their defaults and limits, and the shared ones are under `components.schemas`. A create request's features have no default there, since a request that leaves one out gets the installation's, which `GET /api/settings` reports.
- what it needs: `session`, the cookie, with the `X-Thoric-Request` header on a change; `mcp`, `endpoint` or `metrics`, a bearer token; or nothing, for the session state, sign-in, the skills, `/healthz`, `/readyz` and transfer links, which the token in their path authorizes.
- what it answers: a JSON type where the manager answers with one, such as `JobStarted` (`{"job_id"}`) for a route that queues a job, a job, a compose or a build configuration; otherwise JSON that this guide describes. Every operation names the statuses of the [errors](#errors) table, from `400` to `504`, each with its meaning under `components.responses` and the body `Error`, `{"error": "..."}`, which the other statuses a route uses carry too; `/readyz` describes its own `503`, its checks with the failed one `false`. Every operation takes the optional `X-Request-Id` header and every answer names it, as [the start of this guide](#http-api) says.

A route that answers with something other than one JSON value says how in `x-thoric-stream`: `download` for a file, an archive or a picture streamed as it is read, `upload` for a body the manager streams as it arrives, `event-stream` for `/api/events`, `websocket` for VNC, the audio channels and the MCP inspector, which OpenAPI describes only as their `101` answer, and `relay` for `/mcp`, the MCP endpoints, transfer links and desktop ports, which answer with what is behind them. Operation IDs come from the method and the path, such as `postContainersByIdFilesRead`.

## Limits and results

JSON request bodies are limited to 1,100,000 bytes. Binary uploads have their own configured limit. Logs retain the last 256 KiB; application logs keep about 1 MiB. Screenshot requests share two processing slots and text reading with OCR two more, uploads allow two simultaneous transfers, archive exports, imports, extracts and volume copies two more, and desktop input allows four; operations on one container still use its resource lock, and a launched application does not hold it while it runs. Work that only uses a running desktop, a script, input, file access, an upload's delivery, a terminal, a recording, an application's launch or a duplicate copying the container's volumes, shares the lock, while work that changes or replaces the container, or commits it, a start, stop, reset, settings change, snapshot, a duplicate's commit or a delete, takes it alone and waits for the sharers; sharers that come after it wait for it in turn. A job that starts the desktop shares it again while it [waits for the desktop](#desktop-readiness), unless a compose setup or a settings change still needs it alone. Two input requests to one desktop take turns. An input request waits at most five seconds for a job that holds the container alone, such as a snapshot, and answers 409 after that without taking one of the four input slots. A request that needs a container, volume or image a job holds, such as a snapshot or an export, waits five seconds for it and then answers 409 instead of waiting for the job.

MCP JSON responses are limited to 8 MiB and, like every `/mcp` response, use chunked transfer encoding, which survives the [HTTP scanning of some Windows security products](MCP.md#broken-responses-from-localhost-on-windows). This includes screenshots returned as base64 image content. A [stdio bridge](MCP.md#connect-through-stdio-on-windows) is bundled for clients that cannot use the HTTP connection directly.

File and input operations return their result directly. Creation, lifecycle changes, scripts and image preparation return jobs. A VNC acknowledgement confirms that input was processed, not that an application has finished responding; inspect a screenshot or application result before the next step.
