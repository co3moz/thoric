//! Every route of the router, in the order `main.rs` registers them, with what it takes and
//! answers. A route added there needs its entry here; `openapi_tests.rs` names the ones missing.
use super::{
    Access, CONTAINERS, DESKTOP, EVENTS, FILES, IMAGES, INSTALLATION, JOBS, MCP, Route, SESSION,
    TRANSFERS, VOLUMES, any, delete, get, post, put,
};
use crate::{
    activity, api, apps, archives, audio, auth, backup, capture, compose, config, desktop_input,
    desktop_mcp, exec, files, gamepad, human, icons, jobs, model, ocr, recordings, terminal,
    transfers, uploads, usage, volumes, waits, windows,
};

const GZIP: &str = "application/gzip";
const OCTETS: &str = "application/octet-stream";
const ARCHIVE: &[&str] = &["application/gzip", "application/x-tar"];

pub(super) fn all() -> Vec<Route> {
    vec![
        post("/api/logout", SESSION, "Sign out and end the session.").no_content(),
        get(
            "/api/settings",
            INSTALLATION,
            "Defaults and the features the installation allows, without credentials.",
        ),
        get(
            "/api/configuration",
            INSTALLATION,
            "The settings the manager runs with; secrets only say whether they are set. exposure names what answers without a credential while the manager is published beyond loopback.",
        ),
        post(
            "/api/configuration",
            INSTALLATION,
            "Check a change of the editable settings, or have the WSL package apply it.",
        )
        .json::<config::changes::Change>(),
        get(
            "/api/system",
            INSTALLATION,
            "The Docker connection and a host summary.",
        ),
        get(
            "/api/update",
            INSTALLATION,
            "This manager's release, the newest one and whether the installation can update.",
        ),
        post(
            "/api/update",
            INSTALLATION,
            "Have the WSL package run `thoric update`.",
        ),
        post(
            "/api/update/check",
            INSTALLATION,
            "Ask GitHub for the newest release now.",
        ),
        get(
            "/api/backup",
            INSTALLATION,
            "Download a backup of the whole installation, with the optional parts asked for.",
        )
        .query::<backup::Included>()
        .download(GZIP),
        post(
            "/api/backup/check",
            INSTALLATION,
            "Compare a backup's manifest with this installation.",
        )
        .json::<serde_json::Map<String, serde_json::Value>>(),
        post(
            "/api/backup/restore",
            INSTALLATION,
            "Stream a backup and restore it once the whole of it has arrived.",
        )
        .query::<backup::Upload>()
        .upload(&[GZIP])
        .job(),
        get(
            "/api/usage",
            INSTALLATION,
            "Every container's resource usage once a second for the last 30 minutes.",
        )
        .query::<usage::Since>(),
        get(
            "/api/containers",
            CONTAINERS,
            "The managed containers, their state and health.",
        ),
        post(
            "/api/containers",
            CONTAINERS,
            "Create a container, with values for its image's compose.",
        )
        .json::<api::CreateContainer>()
        .job(),
        get(
            "/api/containers/{id}",
            CONTAINERS,
            "One container's configuration, state and health.",
        ),
        delete(
            "/api/containers/{id}",
            CONTAINERS,
            "Delete the container and its networks.",
        )
        .query::<api::Remove>()
        .job(),
        post(
            "/api/containers/{id}/action",
            CONTAINERS,
            "Start, stop or restart the container.",
        )
        .json::<api::Action>()
        .job(),
        post(
            "/api/containers/{id}/retry",
            CONTAINERS,
            "Retry a setup or a compose setup that did not finish.",
        )
        .job(),
        post(
            "/api/containers/{id}/internet",
            CONTAINERS,
            "Turn internet access on or off.",
        )
        .json::<api::Internet>()
        .job(),
        post(
            "/api/containers/{id}/settings",
            CONTAINERS,
            "Change settings after creation; `dry_run` reports what recreates the container.",
        )
        .json::<serde_json::Map<String, serde_json::Value>>(),
        post(
            "/api/containers/{id}/snapshot",
            CONTAINERS,
            "Save the container's files as an image.",
        )
        .json::<api::Snapshot>()
        .job(),
        post("/api/containers/{id}/exec", CONTAINERS, "Run a script.")
            .json::<exec::Execute>()
            .job(),
        post(
            "/api/containers/{id}/files/read",
            FILES,
            "Read a bounded slice of a file as text or base64.",
        )
        .json::<files::ReadFile>(),
        post(
            "/api/containers/{id}/files/write",
            FILES,
            "Write text or base64 data to a file atomically.",
        )
        .json::<files::WriteFile>(),
        post(
            "/api/containers/{id}/files/list",
            FILES,
            "List a folder without following symlinks.",
        )
        .json::<files::ListFiles>(),
        post(
            "/api/containers/{id}/files/mkdir",
            FILES,
            "Create a folder in an existing parent.",
        )
        .json::<files::MakeDirectory>(),
        post(
            "/api/containers/{id}/files/move",
            FILES,
            "Rename or move within the same volume.",
        )
        .json::<files::MoveFile>(),
        post(
            "/api/containers/{id}/files/delete",
            FILES,
            "Delete a file, symlink or folder.",
        )
        .json::<files::DeleteFile>(),
        get(
            "/api/containers/{id}/files/download",
            FILES,
            "Download a file, or a folder as a `.tar.gz` recorded as a `folder-download` job.",
        )
        .query::<files::DownloadQuery>()
        .download(OCTETS),
        post(
            "/api/containers/{id}/audio-capture",
            DESKTOP,
            "Record desktop or microphone audio as WAV.",
        )
        .json::<audio::CaptureAudio>(),
        post(
            "/api/containers/{id}/audio-inject",
            DESKTOP,
            "Play WAV data into the microphone or the speakers.",
        )
        .json::<audio::InjectAudio>(),
        get(
            "/api/containers/{id}/terminals",
            DESKTOP,
            "Terminal sessions and their state.",
        ),
        post(
            "/api/containers/{id}/terminals",
            DESKTOP,
            "Open an interactive shell.",
        )
        .optional_json::<terminal::OpenTerminal>(),
        post(
            "/api/containers/{id}/terminals/{terminal}/input",
            DESKTOP,
            "Type into a terminal.",
        )
        .json::<terminal::TerminalInput>(),
        get(
            "/api/containers/{id}/terminals/{terminal}/output",
            DESKTOP,
            "Read a terminal's output from an offset.",
        )
        .query::<terminal::TerminalOutput>(),
        post(
            "/api/containers/{id}/terminals/{terminal}/close",
            DESKTOP,
            "Hang up a terminal.",
        ),
        get(
            "/api/containers/{id}/ui",
            DESKTOP,
            "The AT-SPI accessibility tree of the applications.",
        )
        .query::<windows::Inspect>(),
        post(
            "/api/containers/{id}/gamepad",
            DESKTOP,
            "Feed the virtual controller.",
        )
        .json::<gamepad::GamepadInput>(),
        post(
            "/api/containers/{id}/input",
            DESKTOP,
            "Mouse and keyboard actions through VNC.",
        )
        .json::<desktop_input::Input>(),
        post(
            "/api/containers/{id}/act-and-observe",
            DESKTOP,
            "Input actions followed by a screenshot, as JSON.",
        )
        .json::<capture::Observe>(),
        post(
            "/api/containers/{id}/screenshots",
            DESKTOP,
            "Save a screenshot inside the desktop and return its `download_url`.",
        )
        .json::<capture::SaveCapture>(),
        post(
            "/api/containers/{id}/find-text",
            DESKTOP,
            "Text on the screen, read with OCR, with boxes in desktop pixels; 501 from a manager without Tesseract.",
        )
        .json::<ocr::FindText>()
        .returns::<ocr::FoundText>(),
        get(
            "/api/containers/{id}/applications",
            DESKTOP,
            "Launched applications and their state.",
        ),
        post(
            "/api/containers/{id}/applications",
            DESKTOP,
            "Launch an application in the background.",
        )
        .json::<apps::Launch>(),
        get(
            "/api/containers/{id}/applications/{application}",
            DESKTOP,
            "One application's state, pid and exit status.",
        ),
        get(
            "/api/containers/{id}/applications/{application}/logs",
            DESKTOP,
            "The tail of an application's log.",
        )
        .query::<apps::Logs>(),
        post(
            "/api/containers/{id}/applications/{application}/stop",
            DESKTOP,
            "Stop an application.",
        )
        .optional_json::<apps::Stop>(),
        get(
            "/api/containers/{id}/applications/{application}/diagnostics",
            DESKTOP,
            "An application's process tree, resources and latest log lines.",
        ),
        get(
            "/api/containers/{id}/applications/{application}/metrics",
            DESKTOP,
            "An application's CPU and memory, sampled over `duration_ms`.",
        )
        .query::<apps::Metrics>(),
        post(
            "/api/containers/{id}/wait",
            DESKTOP,
            "Wait for a window, an application state, log text, a stable or changed screen, or text on it.",
        )
        .json::<waits::WaitFor>(),
        get(
            "/api/containers/{id}/recordings",
            DESKTOP,
            "Recordings and their state.",
        ),
        post(
            "/api/containers/{id}/recordings",
            DESKTOP,
            "Start recording desktop frames.",
        )
        .json::<recordings::StartRecording>(),
        post(
            "/api/containers/{id}/recordings/{recording}/stop",
            DESKTOP,
            "Stop a recording.",
        ),
        post(
            "/api/containers/{id}/artifacts",
            DESKTOP,
            "Package recordings and application logs as a tar.gz inside the desktop.",
        )
        .json::<recordings::Export>(),
        post(
            "/api/containers/{id}/reset",
            CONTAINERS,
            "Create the container again from its source image.",
        )
        .job(),
        get(
            "/api/containers/{id}/windows",
            DESKTOP,
            "Visible X11 windows and the active one.",
        ),
        post(
            "/api/containers/{id}/windows/{window}/focus",
            DESKTOP,
            "Raise and focus a window.",
        ),
        post(
            "/api/containers/{id}/windows/{window}/geometry",
            DESKTOP,
            "Move or resize a window.",
        )
        .json::<windows::Geometry>(),
        get(
            "/api/containers/{id}/logs",
            CONTAINERS,
            "The container's latest log output.",
        ),
        get(
            "/api/containers/{id}/diagnostics",
            CONTAINERS,
            "Desktop and graphics diagnostics.",
        ),
        get(
            "/api/containers/{id}/usage",
            CONTAINERS,
            "One container's resource usage samples.",
        )
        .query::<usage::Since>(),
        get(
            "/api/containers/{id}/screenshot",
            DESKTOP,
            "A JPEG of the desktop at its resolution.",
        )
        .query::<capture::Capture>()
        .download("image/jpeg"),
        post(
            "/api/containers/{id}/files",
            FILES,
            "Stream an upload into the desktop.",
        )
        .query::<uploads::Upload>()
        .upload(&[OCTETS]),
        get(
            "/api/containers/{id}/vnc",
            DESKTOP,
            "The desktop's screen over VNC, binary subprotocol; the Origin must match.",
        )
        .websocket(),
        get(
            "/api/containers/{id}/audio/{direction}",
            DESKTOP,
            "The desktop's audio or microphone channel; the Origin must match.",
        )
        .websocket(),
        get(
            "/api/images",
            IMAGES,
            "Every image: catalog images, script builds, snapshots and imports.",
        ),
        post(
            "/api/images",
            IMAGES,
            "Import a desktop-compatible local image by its reference.",
        )
        .json::<api::RegisterImage>(),
        post(
            "/api/images/build",
            IMAGES,
            "Build an image from a script or parts.",
        )
        .json::<api::BuildImage>()
        .job(),
        post(
            "/api/images/import",
            IMAGES,
            "Stream a `.tar.gz` or `.tar` image archive and import it.",
        )
        .query::<archives::Import>()
        .upload(ARCHIVE)
        .job(),
        delete(
            "/api/images/{id}",
            IMAGES,
            "Delete a saved image with its Docker image, or a catalog image's download.",
        ),
        post(
            "/api/images/{id}/pull",
            IMAGES,
            "Download a catalog image, or a newer release of it.",
        )
        .job(),
        post(
            "/api/images/{id}/build",
            IMAGES,
            "Build a catalog image from the bundled Docker context.",
        )
        .job(),
        get(
            "/api/images/{id}/archive",
            IMAGES,
            "Download a saved image as a gzip-compressed `docker save` archive.",
        )
        .download(GZIP),
        get(
            "/api/images/{id}/config",
            IMAGES,
            "What a script build ran, as a body `POST /api/images/build` accepts.",
        )
        .returns::<api::BuildImage>(),
        get(
            "/api/images/{id}/layers",
            IMAGES,
            "What the image's disk space is made of.",
        ),
        get("/api/images/{id}/icon", IMAGES, "A saved image's icon.")
            .query::<icons::IconQuery>()
            .download("image/png"),
        put("/api/images/{id}/icon", IMAGES, "Set a saved image's icon.").json::<icons::SetIcon>(),
        delete(
            "/api/images/{id}/icon",
            IMAGES,
            "Remove a saved image's icon.",
        ),
        get("/api/images/{id}/compose", IMAGES, "An image's compose.")
            .returns::<compose::Compose>(),
        put(
            "/api/images/{id}/compose",
            IMAGES,
            "Set an image's compose.",
        )
        .json::<compose::Compose>(),
        delete(
            "/api/images/{id}/compose",
            IMAGES,
            "Remove an image's compose.",
        ),
        post(
            "/api/images/{id}/compose/base",
            IMAGES,
            "Give the image the steps of its base image's compose it lacks.",
        ),
        get(
            "/api/volumes",
            VOLUMES,
            "The named volumes `VOLUME_FILTER` lists.",
        ),
        post("/api/volumes", VOLUMES, "Create a named volume.").json::<api::Volume>(),
        delete(
            "/api/volumes/{name}",
            VOLUMES,
            "Delete a named volume and its files.",
        ),
        post(
            "/api/volumes/import",
            VOLUMES,
            "Stream a `.tar.gz` or `.tar` into a new volume.",
        )
        .query::<volumes::Import>()
        .upload(ARCHIVE)
        .job(),
        delete(
            "/api/volumes/import",
            VOLUMES,
            "Delete the volume called `import`, as `DELETE /api/volumes/{name}` would.",
        ),
        get(
            "/api/volumes/{name}/archive",
            VOLUMES,
            "Download a volume's files as a `.tar.gz`.",
        )
        .download(GZIP),
        post(
            "/api/volumes/{name}/copy",
            VOLUMES,
            "Copy the volume's files into a new volume.",
        )
        .json::<volumes::CopyVolume>()
        .job(),
        post(
            "/api/volumes/{name}/extract",
            VOLUMES,
            "Stream a `.tar.gz` or `.tar` into a new folder of the volume.",
        )
        .query::<volumes::Extract>()
        .upload(ARCHIVE)
        .job(),
        post(
            "/api/volumes/{name}/files/list",
            VOLUMES,
            "List a folder of the volume.",
        )
        .json::<files::ListFiles>(),
        post(
            "/api/volumes/{name}/files/read",
            VOLUMES,
            "Read a bounded slice of a file as text or base64.",
        )
        .json::<files::ReadFile>(),
        post(
            "/api/volumes/{name}/files/write",
            VOLUMES,
            "Write text or base64 data to a file atomically.",
        )
        .json::<files::WriteFile>(),
        post(
            "/api/volumes/{name}/files/mkdir",
            VOLUMES,
            "Create a folder.",
        )
        .json::<volumes::MakeDirectory>(),
        post(
            "/api/volumes/{name}/files/move",
            VOLUMES,
            "Rename or move within the volume.",
        )
        .json::<files::MoveFile>(),
        post(
            "/api/volumes/{name}/files/delete",
            VOLUMES,
            "Delete a file, symlink or folder.",
        )
        .json::<files::DeleteFile>(),
        get(
            "/api/volumes/{name}/files/download",
            VOLUMES,
            "Download a file, or a folder as a `.tar.gz` recorded as a `folder-download` job.",
        )
        .query::<volumes::DownloadQuery>()
        .download(OCTETS),
        post(
            "/api/volumes/{name}/files",
            VOLUMES,
            "Stream an upload into the volume.",
        )
        .query::<uploads::Upload>()
        .upload(&[OCTETS]),
        get(
            "/api/mcp/servers",
            MCP,
            "Every container's desktop MCP servers as last seen.",
        ),
        get(
            "/api/mcp/scopes",
            MCP,
            "The scopes an endpoint can grant, with their tools.",
        ),
        get(
            "/api/mcp/inspect",
            MCP,
            "The MCP inspector's WebSocket to one desktop server, endpoint or `/mcp`.",
        )
        .websocket(),
        get("/api/mcp/endpoints", MCP, "The MCP endpoints."),
        post(
            "/api/mcp/endpoints",
            MCP,
            "Create an MCP endpoint; its `token` is returned once.",
        )
        .json::<desktop_mcp::CreateEndpoint>(),
        put(
            "/api/mcp/endpoints/{name}",
            MCP,
            "Replace an endpoint's grants.",
        )
        .json::<desktop_mcp::UpdateEndpoint>(),
        delete(
            "/api/mcp/endpoints/{name}",
            MCP,
            "Delete an endpoint and end its sessions.",
        ),
        post(
            "/api/mcp/endpoints/{name}/token",
            MCP,
            "Create or replace an endpoint's token, returned once.",
        ),
        delete(
            "/api/mcp/endpoints/{name}/token",
            MCP,
            "Revoke an endpoint's token.",
        ),
        get(
            "/api/mcp/endpoints/{name}/tools",
            MCP,
            "The tools the endpoint offers, as its agents see them.",
        ),
        get(
            "/api/containers/{id}/mcp/servers",
            MCP,
            "Read a running desktop's servers again and list them.",
        ),
        post(
            "/api/containers/{id}/mcp/servers",
            MCP,
            "Register a server of the desktop user and read its tools.",
        )
        .json::<desktop_mcp::Declaration>(),
        delete(
            "/api/containers/{id}/mcp/servers/{name}",
            MCP,
            "Remove a server the desktop user registered.",
        ),
        post(
            "/api/containers/{id}/mcp/servers/{name}/refresh",
            MCP,
            "Start a server again and read its tools.",
        ),
        get("/api/jobs", JOBS, "The latest 100 job summaries."),
        get(
            "/api/jobs/{id}",
            JOBS,
            "A job's status, log and result; `retryable` once it can be started again. `wait_seconds` waits for it to end, and `log_tail_bytes` and `log_offset` return part of its log with `log_start` and `log_end`.",
        )
        .query::<jobs::read::Read>()
        .returns::<model::Job>(),
        post(
            "/api/jobs/{id}/retry",
            JOBS,
            "Start a failed or interrupted job again with the request it was started with.",
        ),
        post(
            "/api/jobs/{id}/cancel",
            JOBS,
            "Cancel a queued job, or stop a running script, build or image download; it ends as failed.",
        ),
        get(
            "/api/jobs/{id}/request",
            JOBS,
            "The request a failed or interrupted job would be retried with.",
        ),
        get(
            "/api/events",
            EVENTS,
            "The session's event stream: jobs, requests for a person and Docker's changes.",
        )
        .events(),
        get(
            "/api/human-requests",
            EVENTS,
            "The requests for a person that wait for an answer.",
        ),
        post(
            "/api/human-requests/{id}/answer",
            EVENTS,
            "Answer a request for a person.",
        )
        .json::<human::AnswerBody>()
        .returns::<human::HumanRequest>(),
        get(
            "/api/activity",
            EVENTS,
            "The agent activity log, newest first, with filters and a cursor.",
        )
        .query::<activity::Filter>(),
        get(
            "/api/dependencies",
            INSTALLATION,
            "Every dependency the manager image was built from.",
        ),
        get(
            "/api/openapi.json",
            INSTALLATION,
            "This document: the routes of the HTTP API with their bodies.",
        ),
        any(
            &["GET", "POST", "DELETE"],
            "/mcp",
            MCP,
            "The root MCP server, Streamable HTTP; off unless `ROOT_MCP_ENABLED`.",
        )
        .access(Access::McpToken)
        .upload(&["application/json"]),
        get(
            "/api/session",
            SESSION,
            "The sign-in mode and whether this request is signed in.",
        )
        .access(Access::Public),
        post(
            "/api/login",
            SESSION,
            "Sign in; the session cookie comes back in `Set-Cookie`.",
        )
        .access(Access::SignIn)
        .json::<auth::Login>()
        .no_content(),
        any(
            &["GET", "PUT", "POST"],
            "/api/transfers/{token}",
            TRANSFERS,
            "Run the one upload, download, import or export a transfer link authorizes.",
        )
        .access(Access::TransferLink)
        .query::<transfers::Size>()
        .upload(&[OCTETS]),
        any(
            &["GET", "POST", "DELETE"],
            "/mcp/{name}",
            MCP,
            "An MCP endpoint, Streamable HTTP.",
        )
        .access(Access::EndpointToken)
        .upload(&["application/json"]),
        any(
            PORT_METHODS,
            "/api/containers/{id}/ports/{port}",
            TRANSFERS,
            "HTTP or a WebSocket to a port inside the desktop.",
        )
        .access(Access::McpToken),
        any(
            PORT_METHODS,
            "/api/containers/{id}/ports/{port}/",
            TRANSFERS,
            "HTTP or a WebSocket to a port inside the desktop.",
        )
        .access(Access::McpToken),
        any(
            PORT_METHODS,
            "/api/containers/{id}/ports/{port}/{*path}",
            TRANSFERS,
            "HTTP or a WebSocket to a path of a port inside the desktop.",
        )
        .access(Access::McpToken),
        get(
            "/healthz",
            INSTALLATION,
            "Liveness: `ok` while the manager answers, without asking the database or Docker.",
        )
        .access(Access::Public)
        .text(),
        get(
            "/readyz",
            INSTALLATION,
            "Readiness: `ready`, `database` and `docker` as booleans, true when the database and Docker answer.",
        )
        .access(Access::Public)
        .unavailable("Not ready: the same booleans, with each check that failed false."),
        get(
            "/metrics",
            INSTALLATION,
            "Prometheus metrics in the text format 0.0.4, which `METRICS_TOKEN` opens as a bearer token; 404 while it is not set.",
        )
        .access(Access::MetricsToken)
        .text(),
    ]
}

/// The methods the document names for a desktop port, which passes on any.
const PORT_METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE"];
