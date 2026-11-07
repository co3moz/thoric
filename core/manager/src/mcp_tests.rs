//! Pins down the administrator's MCP channel at `/mcp`: its bearer token and Host and Origin
//! checks, the tool list agents see and what `docs/MCP.md` and the README say of it, refusals of an
//! unknown tool or argument before anything changes, the bounds of its transport, replies chunked
//! on the wire, and a screenshot call that fits in half a worker's stack.
use crate::test_support::{mcp_app, mcp_request, spec};
use crate::{model, router};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn mcp_requires_its_own_token_and_rejects_cross_origin_access() {
    for (enabled, token, host, origin, expected) in [
        (false, None, "localhost:8080", None, StatusCode::NOT_FOUND),
        (true, None, "localhost:8080", None, StatusCode::UNAUTHORIZED),
        (
            true,
            Some("Bearer wrong"),
            "localhost:8080",
            None,
            StatusCode::UNAUTHORIZED,
        ),
        (
            true,
            Some("Bearer test-token-with-at-least-32-characters"),
            "evil.example",
            None,
            StatusCode::FORBIDDEN,
        ),
        (
            true,
            Some("Bearer test-token-with-at-least-32-characters"),
            "localhost:8080",
            Some("https://evil.example"),
            StatusCode::FORBIDDEN,
        ),
    ] {
        let mut app = mcp_app();
        Arc::get_mut(&mut app).unwrap().config.root_mcp_enabled = enabled;
        let mut req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("host", host);
        if let Some(token) = token {
            req = req.header("authorization", token);
        }
        if let Some(origin) = origin {
            req = req.header("origin", origin);
        }
        let response = router(app)
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
}

#[tokio::test]
async fn mcp_without_a_token_still_rejects_other_hosts_and_origins() {
    let initialize = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}});
    for (token, host, origin, expected) in [
        (None, "localhost:8080", None, StatusCode::OK),
        (Some("Bearer wrong"), "localhost:8080", None, StatusCode::OK),
        (None, "evil.example", None, StatusCode::FORBIDDEN),
        (
            None,
            "localhost:8080",
            Some("https://evil.example"),
            StatusCode::FORBIDDEN,
        ),
    ] {
        let mut app = mcp_app();
        Arc::get_mut(&mut app).unwrap().config.mcp_token = None;
        let mut req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("host", host)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-11-25");
        if let Some(token) = token {
            req = req.header("authorization", token);
        }
        if let Some(origin) = origin {
            req = req.header("origin", origin);
        }
        let response = router(app)
            .oneshot(req.body(Body::from(initialize.to_string())).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
}

#[tokio::test]
async fn mcp_negotiates_and_lists_tools_with_useful_results() {
    let app = mcp_app();
    let (status, body) = mcp_request(app.clone(), "initialize", serde_json::json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["serverInfo"]["name"], "thoric");
    let instructions = body["result"]["instructions"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let (status, body) = mcp_request(app.clone(), "tools/list", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let tools = body["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 86);
    // Usage is read for every container unless one is named.
    let usage = tools.iter().find(|t| t["name"] == "get_usage").unwrap();
    assert_eq!(usage["annotations"]["readOnlyHint"], true);
    assert_eq!(usage["inputSchema"]["required"], serde_json::json!([]));
    assert!(usage["inputSchema"]["properties"]["container_id"].is_object());
    // Compose values reach a new container through create_container; the schema forbids
    // properties it does not list, so a missing one is a value no agent can send.
    let create = tools
        .iter()
        .find(|t| t["name"] == "create_container")
        .unwrap();
    assert!(create["inputSchema"]["properties"]["parameters"].is_object());
    let compose = tools
        .iter()
        .find(|t| t["name"] == "set_image_compose")
        .unwrap();
    assert_eq!(compose["annotations"]["destructiveHint"], true);
    assert_eq!(
        compose["inputSchema"]["properties"]["compose"]["anyOf"][0]["required"],
        serde_json::json!(["script"])
    );
    for name in ["delete_container", "delete_image", "delete_volume"] {
        let delete = tools.iter().find(|t| t["name"] == name).unwrap();
        assert_eq!(delete["annotations"]["destructiveHint"], true, "{name}");
    }
    let screenshot = tools.iter().find(|t| t["name"] == "screenshot").unwrap();
    assert_eq!(screenshot["annotations"]["readOnlyHint"], true);
    assert!(screenshot["inputSchema"]["properties"]["max_width"].is_object());
    assert!(
        screenshot["description"]
            .as_str()
            .unwrap()
            .contains("save_screenshot")
    );
    let save = tools
        .iter()
        .find(|t| t["name"] == "save_screenshot")
        .unwrap();
    assert_eq!(save["annotations"]["readOnlyHint"], false);
    assert!(save["inputSchema"]["properties"]["capture"].is_object());
    assert_eq!(
        save["inputSchema"]["required"],
        serde_json::json!(["path", "container_id"])
    );
    for name in [
        "save_screenshot",
        "download_file",
        "export_artifacts",
        "write_file",
        "upload_file",
    ] {
        let tool = tools.iter().find(|t| t["name"] == name).unwrap();
        assert!(
            tool["description"]
                .as_str()
                .unwrap()
                .contains("download_url"),
            "{name} must tell agents how to hand files to the user"
        );
    }
    // Agents reached for docker and wsl on the host when a client hid a returned image. The
    // instructions must steer them to the download link instead, and the guide must document it.
    assert!(instructions.contains("download_url") && instructions.contains("wsl"));
    // Agents wrote their own install scripts for toolchains a template already builds, since no
    // tool lists templates. The instructions and build_image must point them at the list.
    let build = tools.iter().find(|t| t["name"] == "build_image").unwrap();
    for text in [
        instructions.as_str(),
        build["description"].as_str().unwrap(),
    ] {
        assert!(text.contains("templates_url") && text.contains("template_id"));
    }
    // An agent took a picture of an installer's dialog with ImageMagick's import after the dialog
    // had closed, and import held the whole display while it waited for a click. The instructions
    // and the tools that start programs must steer agents to screenshot's window_id.
    for text in
        [instructions.as_str()]
            .into_iter()
            .chain(["exec_container", "launch_application"].map(|name| {
                tools.iter().find(|t| t["name"] == name).unwrap()["description"]
                    .as_str()
                    .unwrap()
            }))
    {
        assert!(
            text.contains("import") && text.contains("window_id"),
            "{text}"
        );
    }
    // An agent ran glxinfo through launch_application without gpu=true, read llvmpipe, found no
    // /dev/dri and concluded that a desktop with the WSL GPU had none. The instructions and the
    // tools that start programs must say that only gpu-run reaches the GPU.
    assert!(instructions.contains("/dev/dxg"), "{instructions}");
    for text in
        [instructions.as_str()]
            .into_iter()
            .chain(["exec_container", "launch_application"].map(|name| {
                tools.iter().find(|t| t["name"] == name).unwrap()["description"]
                    .as_str()
                    .unwrap()
            }))
    {
        assert!(
            text.contains("gpu-run") && text.contains("llvmpipe"),
            "{text}"
        );
    }
    let guide = include_str!("../../../docs/MCP.md");
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        assert!(
            guide.contains(&format!("`{name}`")),
            "docs/MCP.md must mention {name}"
        );
    }
    assert!(guide.contains("`download_url`"));
    assert!(guide.contains("## Build from a template") && guide.contains("`templates_url`"));
    // The count went stale once already; it is stated in the guide, so it is checked from here.
    assert!(
        guide.contains(&format!("exposes {} tools", tools.len())),
        "docs/MCP.md must state the tool count"
    );
    // The README leads with the same count, so it goes stale the same way.
    assert!(
        include_str!("../../../README.md").contains(&format!("with {} tools", tools.len())),
        "README.md must state the tool count"
    );
    assert_eq!(
        screenshot["inputSchema"]["required"],
        serde_json::json!(["container_id"])
    );
    let observe = tools
        .iter()
        .find(|t| t["name"] == "act_and_observe")
        .unwrap();
    assert!(observe["inputSchema"]["properties"]["actions"].is_object());
    assert!(
        observe["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "container_id")
    );
    let stop = tools
        .iter()
        .find(|t| t["name"] == "stop_application")
        .unwrap();
    assert!(
        stop["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "application_id")
    );
    let (_, body) = mcp_request(
        app.clone(),
        "tools/call",
        serde_json::json!({"name":"get_settings","arguments":{}}),
    )
    .await;
    assert_eq!(body["result"]["isError"], false, "{body}");
    assert!(body["result"]["structuredContent"]["base_image"].is_string());
    assert!(!body.to_string().contains("test-token"));
    let (_, body) = mcp_request(
        app.clone(),
        "tools/call",
        serde_json::json!({"name":"get_usage","arguments":{"seconds":30}}),
    )
    .await;
    assert_eq!(body["result"]["isError"], false, "{body}");
    assert_eq!(body["result"]["structuredContent"]["seconds"], 30);
    assert_eq!(body["result"]["structuredContent"]["gpu"]["scope"], "host");
    let (_, body) = mcp_request(
        app,
        "tools/call",
        serde_json::json!({"name":"delete_container","arguments":{"container_id":"missing"}}),
    )
    .await;
    assert_eq!(body["result"]["isError"], true, "{body}");
    assert!(
        body["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not found")
    );
}

/// An unknown tool is a protocol error. Arguments the schema refuses are a tool error naming the
/// argument, which the model reads and can correct its call from, as MCP 2025-11-25 asks; as a
/// protocol error most clients showed it only as a failed call.
#[tokio::test]
async fn mcp_rejects_invalid_tools_and_arguments_without_mutating() {
    let app = mcp_app();
    let (_, body) = mcp_request(
        app.clone(),
        "tools/call",
        serde_json::json!({"name":"not_a_tool","arguments":{}}),
    )
    .await;
    assert_eq!(body["error"]["code"], -32602, "{body}");
    for (args, refusal) in [
        (
            serde_json::json!({"name":"delete_container","arguments":{}}),
            "Missing argument: container_id",
        ),
        (
            serde_json::json!({"name":"create_container","arguments":{"name":"test","image":"ubuntu","unexpected":true}}),
            "Unknown argument: unexpected",
        ),
    ] {
        let (_, body) = mcp_request(app.clone(), "tools/call", args).await;
        assert_eq!(body["result"]["isError"], true, "{body}");
        assert_eq!(body["result"]["content"][0]["text"], refusal, "{body}");
    }
    let (_,body) = mcp_request(app.clone(),"tools/call",serde_json::json!({"name":"exec_container","arguments":{"container_id":"missing","script":"echo ok","timeout_seconds":0}})).await;
    assert_eq!(body["result"]["isError"], true, "{body}");
    assert!(app.db.list::<model::Job>("job").unwrap().is_empty());
    let (_,body)=mcp_request(app.clone(),"tools/call",serde_json::json!({"name":"create_container","arguments":{"name":"test","image":"ubuntu","mounts":[{"volume":"project","target":"/workspace","read_only":true}]}})).await;
    assert_eq!(body["result"]["isError"], true, "{body}");
    assert!(app.db.list::<model::Job>("job").unwrap().is_empty());
    // copy_volume reaches the copy route, which refuses it before any job without a manager.
    for (arguments, refusal) in [
        (
            serde_json::json!({"volume":"project","name":"project-copy"}),
            "MANAGER_CONTAINER",
        ),
        (
            serde_json::json!({"volume":"project","name":"-bad"}),
            "Invalid volume name",
        ),
    ] {
        let (_, body) = mcp_request(
            app.clone(),
            "tools/call",
            serde_json::json!({"name":"copy_volume","arguments":arguments}),
        )
        .await;
        assert_eq!(body["result"]["isError"], true, "{body}");
        assert!(body.to_string().contains(refusal), "{body}");
    }
    let (_, body) = mcp_request(
        app.clone(),
        "tools/call",
        serde_json::json!({"name":"copy_volume","arguments":{"name":"project-copy"}}),
    )
    .await;
    assert_eq!(body["result"]["isError"], true, "{body}");
    assert_eq!(
        body["result"]["content"][0]["text"],
        "Missing argument: volume"
    );
    assert!(app.db.list::<model::Job>("job").unwrap().is_empty());
}

/// A debug build reserves stack in a function's frame for every future it awaits, and the MCP
/// dispatch awaits dozens in one match. Once that took most of a tokio worker's 2 MiB, so a debug
/// manager aborted on screenshot. The whole /mcp call, down to running Docker, must fit in half.
#[test]
fn mcp_screenshot_fits_in_half_a_worker_stack() {
    let app = mcp_app();
    let env = model::Container {
        id: "test-id".into(),
        container: "test-container".into(),
        private_network: "test-private".into(),
        egress_network: "test-egress".into(),
        source_image_id: "sha256:test".into(),
        created_at: 0,
        spec: spec(),
        last_error: None,
        desktop: None,
        working_image: None,
        setup: None,
    };
    app.db.put(model::CONTAINERS, &env.id, &env).unwrap();
    // A stack overflow aborts the test binary and names this thread. The call needs under 512 KiB
    // on Linux and 768 KiB on Windows; with Docker's read chunks in arrays it overflowed 2 MiB.
    std::thread::Builder::new()
        .name("mcp-screenshot-in-1-mib".into())
        .stack_size(1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let (_, body) = mcp_request(
                        app,
                        "tools/call",
                        serde_json::json!({"name":"screenshot","arguments":{"container_id":"test-id","max_width":320}}),
                    )
                    .await;
                    // The test has no Docker, so the call ends where it would run the first command.
                    assert_eq!(body["result"]["isError"], true, "{body}");
                    assert!(
                        body.to_string().contains("Could not start Docker CLI"),
                        "{body}"
                    );
                })
        })
        .unwrap()
        .join()
        .unwrap();
}

#[tokio::test]
async fn mcp_transport_bounds_messages_and_accepts_notifications() {
    let app = mcp_app();
    for (method, version, payload, expected) in [
        (
            "POST",
            "2025-11-25",
            serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string(),
            StatusCode::ACCEPTED,
        ),
        (
            "GET",
            "2025-11-25",
            String::new(),
            StatusCode::METHOD_NOT_ALLOWED,
        ),
        (
            "POST",
            "1900-01-01",
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"ping"}).to_string(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "POST",
            "2025-11-25",
            "x".repeat(1_100_001),
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
    ] {
        let response = router(app.clone())
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri("/mcp")
                    .header("host", "localhost")
                    .header(
                        "authorization",
                        "Bearer test-token-with-at-least-32-characters",
                    )
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .header("mcp-protocol-version", version)
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{method} {version}");
    }
}

/// Norton's web protection rewrites a Content-Length reply on a loopback `/mcp` path into a
/// chunked header over an unchunked body. Chunked replies pass through it intact, so the framing
/// that reaches the socket is what has to stay chunked, rejections included.
#[tokio::test]
async fn mcp_replies_are_chunked_on_the_wire() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router(mcp_app())).await });
    let initialize = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})
    .to_string();
    for (authorization, expected) in [
        ("", "HTTP/1.1 401 "),
        (
            "Authorization: Bearer test-token-with-at-least-32-characters\r\n",
            "HTTP/1.1 200 ",
        ),
    ] {
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        let request = format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost:8080\r\n{authorization}Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: 2025-11-25\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{initialize}",
            initialize.len()
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply).await.unwrap();
        let reply = String::from_utf8(reply).unwrap();
        let (head, body) = reply.split_once("\r\n\r\n").unwrap();
        let head = head.to_ascii_lowercase();
        assert!(reply.starts_with(expected), "{head}");
        assert!(head.contains("\r\ntransfer-encoding: chunked"), "{head}");
        assert!(!head.contains("\r\ncontent-length:"), "{head}");
        let (size, rest) = body.split_once("\r\n").unwrap();
        let size = usize::from_str_radix(size, 16).unwrap();
        let json: serde_json::Value = serde_json::from_str(&rest[..size]).unwrap();
        assert!(json.is_object(), "{json}");
        assert!(rest[size..].ends_with("0\r\n\r\n"), "{body}");
    }
}

fn build_image_tool() -> &'static rmcp::model::Tool {
    super::tools::TOOLS
        .iter()
        .find(|tool| tool.name == "build_image")
        .expect("build_image")
}

/// `build_image` says a template's `<id>.json` is a request it takes unchanged, so every field
/// `POST /api/images/build` reads is one of its arguments; a new field fails here until it is.
#[test]
fn build_image_takes_every_field_of_a_build_request() {
    let request =
        serde_json::to_value(rmcp::schemars::schema_for!(crate::api::BuildImage)).unwrap();
    let offered = build_image_tool().input_schema["properties"]
        .as_object()
        .unwrap();
    let missing: Vec<_> = request["properties"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|field| !offered.contains_key(*field))
        .collect();
    assert!(missing.is_empty(), "build_image does not take {missing:?}");
}

/// A template that declares desktop MCP servers, as most templates of the wiki do, passes the
/// argument check as the template list serves it.
#[test]
fn build_image_takes_a_template_with_mcp_servers() {
    let template = serde_json::json!({
        "name": "Chrome with Playwright MCP",
        "description": "Google Chrome that an agent drives through Playwright MCP.",
        "base_image": "ghcr.io/co3moz/thoric/ubuntu-xfce:24.04",
        "internet": true,
        "script": "set -eu\napt-get install -y google-chrome-stable\n",
        "mcp": [
            {"name": "playwright", "command": "chrome-mcp", "args": ["serve", "${CAPS}"]},
            {"name": "notes", "url": "http://127.0.0.1:8931/mcp", "headers": {"Authorization": "Bearer ${TOKEN}"}}
        ],
        "template_id": "chrome-mcp"
    });
    super::check_arguments(build_image_tool(), &template).unwrap();
    let declared: Vec<crate::desktop_mcp::Declaration> =
        serde_json::from_value(template["mcp"].clone()).unwrap();
    crate::desktop_mcp::check_declarations(&declared).unwrap();
}

/// `download_file` once returned its base64 in the text block and again in structuredContent, so
/// a 4 MiB file outgrew the 8 MiB reply and the client got a 502 instead of a result. The bytes
/// travel once: a picture as an image block, any other file as an embedded resource.
#[test]
fn a_downloaded_file_travels_once_within_the_reply_limit() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let bytes = vec![0x5a; crate::files::DOWNLOAD_LIMIT];
    let encoded = STANDARD.encode(&bytes);
    for (path, kind) in [
        ("/home/desktop/shot.png", "image"),
        ("/home/desktop/build log.tar.gz", "resource"),
    ] {
        let name = path.rsplit('/').next().unwrap();
        let summary = serde_json::json!({
            "path": path,
            "name": name,
            "bytes": bytes.len(),
            "mime_type": crate::files::mime_type(name),
            "download_url": crate::files::download_url("c1", path),
        });
        let result = super::dispatch::file(summary.clone(), &bytes);
        let wire = serde_json::to_string(&result).unwrap();
        assert!(wire.len() < 4 * 1024 * 1024, "{path}: {} bytes", wire.len());
        assert_eq!(wire.matches(encoded.as_str()).count(), 1, "{path}");
        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(value["isError"], false);
        assert_eq!(value["content"][0]["type"], kind, "{path}");
        assert_eq!(value["structuredContent"], summary);
        let text: serde_json::Value =
            serde_json::from_str(value["content"][1]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text, summary);
    }
    let summary = serde_json::json!({"path": "/home/desktop/build log.tar.gz", "mime_type": "application/gzip"});
    let value = serde_json::to_value(super::dispatch::file(summary, b"\x1f\x8b")).unwrap();
    assert_eq!(
        value["content"][0]["resource"],
        serde_json::json!({"uri": "file:///home/desktop/build%20log.tar.gz", "mimeType": "application/gzip", "blob": "H4s="})
    );
}

/// idempotentHint copied readOnlyHint, so no tool that changes something said that calling it
/// again changes nothing more, and no tool had a title for a client to show a person. Every tool
/// has a title, and the hint is true for the read-only tools and those `hints` names.
#[test]
fn every_tool_has_a_title_and_its_own_idempotent_hint() {
    let tools = &*super::tools::TOOLS;
    let find = |name: &str| tools.iter().find(|tool| tool.name == name);
    for tool in tools {
        let annotations = tool.annotations.as_ref().unwrap();
        assert!(
            tool.title.as_deref().is_some_and(|title| !title.is_empty()),
            "{} has no title",
            tool.name
        );
        assert_eq!(annotations.title, tool.title);
        if annotations.read_only_hint == Some(true) {
            assert_eq!(annotations.idempotent_hint, Some(true), "{}", tool.name);
        }
    }
    for (name, _) in super::hints::TITLES {
        assert!(find(name).is_some(), "a title for {name}, which is no tool");
    }
    for name in super::hints::IDEMPOTENT {
        let tool = find(name).unwrap_or_else(|| panic!("{name} is no tool"));
        let annotations = tool.annotations.as_ref().unwrap();
        assert_eq!(annotations.read_only_hint, Some(false), "{name}");
        assert_eq!(annotations.idempotent_hint, Some(true), "{name}");
    }
    for name in [
        "container_action",
        "exec_container",
        "desktop_input",
        "snapshot",
    ] {
        let annotations = find(name).unwrap().annotations.as_ref().unwrap();
        assert_eq!(annotations.idempotent_hint, Some(false), "{name}");
    }
}
