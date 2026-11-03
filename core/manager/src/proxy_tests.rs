//! The port proxy: which requests it lets through (the MCP token, a listed port, a running
//! desktop), what a service inside sees of them (no credential of the manager), what a browser
//! could get back (a sandboxed answer that sets nothing on the dashboard's origin), and that
//! streams and WebSockets pass through as they happen.
use super::*;
use crate::{
    model,
    test_support::{app, mcp_app},
};
use axum::{
    Router,
    extract::ws::{Message, WebSocketUpgrade},
    routing::{any, get},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

const TOKEN: &str = "test-token-with-at-least-32-characters";

async fn serve(router: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    address
}

/// The manager's half of the proxy without Docker: every request goes to `service`.
async fn front(service: SocketAddr) -> SocketAddr {
    let forwarded = any(move |request: Request| forward(service, service.port(), request, None));
    serve(
        Router::new()
            .route("/api/containers/{id}/ports/{port}/", forwarded.clone())
            .route("/api/containers/{id}/ports/{port}/{*path}", forwarded),
    )
    .await
}

/// One request on its own connection, as an agent's HTTP client sends it.
async fn call(
    address: SocketAddr,
    request: hyper::Request<Body>,
) -> hyper::Response<hyper::body::Incoming> {
    let stream = TcpStream::connect(address).await.unwrap();
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .unwrap();
    tokio::spawn(connection);
    sender.send_request(request).await.unwrap()
}

async fn body(response: hyper::Response<hyper::body::Incoming>) -> Vec<u8> {
    response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec()
}

fn get_request(uri: &str) -> hyper::http::request::Builder {
    hyper::Request::builder()
        .uri(uri)
        .header("host", "localhost:8080")
}

/// A service that answers with what it received.
fn echo() -> Router {
    Router::new().fallback(|request: Request| async move {
        let headers = request
            .headers()
            .iter()
            .map(|(name, value)| (name.to_string(), json!(value.to_str().unwrap())))
            .collect::<serde_json::Map<_, _>>();
        let method = request.method().to_string();
        let uri = request.uri().to_string();
        let body = axum::body::to_bytes(request.into_body(), 1024)
            .await
            .unwrap();
        axum::Json(json!({
            "method": method,
            "uri": uri,
            "headers": headers,
            "body": String::from_utf8_lossy(&body),
        }))
    })
}

#[test]
fn a_route_splits_into_the_container_the_port_and_the_services_path() {
    let parsed = route("/api/containers/e/ports/8000/mcp/tools").unwrap();
    assert_eq!(
        (parsed.id, parsed.port, parsed.base, parsed.path),
        ("e", "8000", "/api/containers/e/ports/8000", "/mcp/tools")
    );
    for (path, inner) in [
        ("/api/containers/e/ports/8000", "/"),
        ("/api/containers/e/ports/8000/", "/"),
        ("/api/containers/e/ports/8000/a%20b/", "/a%20b/"),
    ] {
        let parsed = route(path).unwrap();
        assert_eq!(
            (parsed.base, parsed.path),
            ("/api/containers/e/ports/8000", inner)
        );
    }
    assert!(route("/api/containers/e/files").is_none());
    assert!(route("/mcp").is_none());
}

#[tokio::test]
async fn a_service_sees_no_credential_of_the_manager() {
    let service = serve(echo()).await;
    let proxy = front(service).await;
    let port = service.port();
    let response = call(
        proxy,
        get_request(&format!("/api/containers/e/ports/{port}/echo/a%20b?x=1"))
            .method("POST")
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("cookie", "thoric_session=dashboard; theme=dark")
            .header("origin", "http://localhost:8080")
            .header("connection", "keep-alive, x-hop")
            .header("x-hop", "connection only")
            .header("x-custom", "kept")
            .header("content-length", "2")
            .body(Body::from("hi"))
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let seen: Value = serde_json::from_slice(&body(response).await).unwrap();
    assert_eq!(seen["method"], "POST");
    assert_eq!(seen["uri"], "/echo/a%20b?x=1");
    assert_eq!(seen["body"], "hi");
    let headers = &seen["headers"];
    assert_eq!(headers["host"], format!("localhost:{port}"));
    assert_eq!(headers["cookie"], "theme=dark");
    assert_eq!(headers["x-custom"], "kept");
    for absent in ["authorization", "origin", "x-hop"] {
        assert!(
            headers.get(absent).is_none(),
            "{absent} reached the service"
        );
    }
    // Only the dashboard's cookie goes; without other cookies, so does the header.
    let response = call(
        proxy,
        get_request(&format!("/api/containers/e/ports/{port}/"))
            .header("cookie", "thoric_session=dashboard")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let seen: Value = serde_json::from_slice(&body(response).await).unwrap();
    assert_eq!(seen["uri"], "/");
    assert!(seen["headers"].get("cookie").is_none());
}

#[tokio::test]
async fn an_answer_is_sandboxed_and_sets_nothing_on_the_dashboards_origin() {
    let service = serve(Router::new().fallback(|request: Request| async move {
        let location = request.uri().query().unwrap_or("/login").to_owned();
        (
            StatusCode::FOUND,
            [
                (header::LOCATION, location),
                (
                    header::SET_COOKIE,
                    "thoric_session=planted; Path=/".to_owned(),
                ),
                (
                    HeaderName::from_static("clear-site-data"),
                    "\"*\"".to_owned(),
                ),
                (
                    header::CONTENT_SECURITY_POLICY,
                    "default-src 'none'".to_owned(),
                ),
            ],
            "<script>parent.document</script>",
        )
    }))
    .await;
    let proxy = front(service).await;
    let port = service.port();
    let base = format!("/api/containers/e/ports/{port}");
    let response = call(
        proxy,
        get_request(&format!("{base}/app"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FOUND);
    let headers = response.headers();
    assert_eq!(headers["location"], format!("{base}/login"));
    assert!(headers.get("set-cookie").is_none());
    assert!(headers.get("clear-site-data").is_none());
    let policies = headers
        .get_all("content-security-policy")
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect::<Vec<_>>();
    assert!(policies.contains(&"sandbox"), "{policies:?}");
    assert!(policies.contains(&"default-src 'none'"), "{policies:?}");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    // A redirect elsewhere is left as the service sent it.
    for location in ["//elsewhere.example/x", "http://localhost:9/x"] {
        let response = call(
            proxy,
            get_request(&format!("{base}/app?{location}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.headers()["location"], location);
    }
}

#[tokio::test]
async fn a_stream_passes_through_as_it_is_sent() {
    let (release, released) = tokio::sync::oneshot::channel::<()>();
    let released = Arc::new(std::sync::Mutex::new(Some(released)));
    let service = serve(Router::new().route(
        "/events",
        get(move || {
            let released = released.lock().unwrap().take();
            async move {
                let events =
                    futures_util::stream::unfold((0, released), |(step, released)| async move {
                        match step {
                            0 => Some((Ok::<_, std::io::Error>("data: one\n\n"), (1, released))),
                            1 => {
                                released?.await.ok()?;
                                Some((Ok("data: two\n\n"), (2, None)))
                            }
                            _ => None,
                        }
                    });
                (
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    Body::from_stream(events),
                )
            }
        }),
    ))
    .await;
    let proxy = front(service).await;
    let response = call(
        proxy,
        get_request(&format!(
            "/api/containers/e/ports/{}/events",
            service.port()
        ))
        .body(Body::empty())
        .unwrap(),
    )
    .await;
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut events = response.into_body();
    let mut next = async || {
        let frame = tokio::time::timeout(Duration::from_secs(5), events.frame())
            .await
            .expect("an event that was sent arrives")
            .unwrap()
            .unwrap();
        frame.into_data().unwrap()
    };
    // The service has not sent its second event yet: the first arrives on its own.
    assert_eq!(next().await, "data: one\n\n");
    release.send(()).unwrap();
    assert_eq!(next().await, "data: two\n\n");
}

/// Open a WebSocket at `path` of `address` and check that a text frame comes back.
async fn echoes(address: SocketAddr, path: &str, authorization: Option<&str>) {
    let mut socket = TcpStream::connect(address).await.unwrap();
    let authorization = authorization
        .map(|value| format!("Authorization: {value}\r\n"))
        .unwrap_or_default();
    socket
        .write_all(
            format!(
                "GET {path} HTTP/1.1\r\nHost: localhost:8080\r\n{authorization}Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(socket.read_u8().await.unwrap());
    }
    let head = String::from_utf8(head).unwrap();
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    // A client's frames are masked; the service's are not.
    let mask = [1u8, 2, 3, 4];
    let mut frame = vec![0x81, 0x80 | 5];
    frame.extend(mask);
    frame.extend(b"hello".iter().zip(mask.iter().cycle()).map(|(b, m)| b ^ m));
    socket.write_all(&frame).await.unwrap();
    let mut answer = [0u8; 7];
    tokio::time::timeout(Duration::from_secs(5), socket.read_exact(&mut answer))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&answer, b"\x81\x05hello");
}

fn websocket_echo() -> Router {
    Router::new().route(
        "/echo",
        get(|upgrade: WebSocketUpgrade| async move {
            upgrade.on_upgrade(|mut socket| async move {
                while let Some(Ok(message)) = socket.recv().await {
                    if matches!(message, Message::Text(_) | Message::Binary(_))
                        && socket.send(message).await.is_err()
                    {
                        break;
                    }
                }
            })
        }),
    )
}

#[tokio::test]
async fn a_websocket_is_joined_to_the_service() {
    let service = serve(websocket_echo()).await;
    let proxy = front(service).await;
    echoes(
        proxy,
        &format!("/api/containers/e/ports/{}/echo", service.port()),
        None,
    )
    .await;
}

#[tokio::test]
async fn an_mcp_client_calls_a_tool_of_an_mcp_server_in_the_desktop() {
    // Thoric's own MCP endpoint stands in for a server inside the desktop.
    let mut inner = app(false);
    let config = &mut Arc::get_mut(&mut inner).unwrap().config;
    config.root_mcp_enabled = true;
    config.mcp_token = None;
    let service = serve(crate::router(inner)).await;
    let proxy = front(service).await;
    let response = call(
        proxy,
        get_request(&format!("/api/containers/e/ports/{}/mcp", service.port()))
            .method("POST")
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-11-25")
            .body(Body::from(
                json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "get_settings", "arguments": {}}}).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    // Chunked, as /mcp itself answers; see mcp::chunked.
    assert!(response.headers().get("content-length").is_none());
    let reply: Value = serde_json::from_slice(&body(response).await).unwrap();
    assert_eq!(
        reply["result"]["structuredContent"]["upload_limit_bytes"],
        1024 * 1024,
        "{reply}"
    );
}

/// The container `e`, whose settings proxy port 8000.
fn container(app: &App, ports: &[u16]) {
    let env = model::Container {
        id: "e".into(),
        container: "thoric-e".into(),
        private_network: "thoric-e-private".into(),
        egress_network: "thoric-e-egress".into(),
        source_image_id: "sha256:desktop".into(),
        created_at: 1,
        spec: serde_json::from_value(
            json!({"name": "Work", "image": "example/desktop:1", "ports": ports}),
        )
        .unwrap(),
        last_error: None,
        desktop: None,
        working_image: None,
        setup: None,
    };
    app.db.put(model::CONTAINERS, "e", &env).unwrap();
}

async fn status(app: &Arc<App>, path: &str, headers: &[(&str, &str)]) -> (StatusCode, String) {
    let mut request = axum::http::Request::builder()
        .uri(path)
        .header("host", "localhost:8080");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = crate::router(app.clone())
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn only_the_mcp_token_opens_a_listed_port() {
    let app = mcp_app();
    container(&app, &[8000]);
    let expiry = std::time::Instant::now() + Duration::from_secs(60);
    app.sessions
        .lock()
        .unwrap()
        .insert("dashboard".into(), expiry);
    let bearer = format!("Bearer {TOKEN}");
    let token = [("authorization", bearer.as_str())];
    // A signed-in browser carries the cookie and nothing else, and is refused.
    let (code, _) = status(
        &app,
        "/api/containers/e/ports/8000/",
        &[("cookie", "thoric_session=dashboard")],
    )
    .await;
    assert_eq!(code, StatusCode::UNAUTHORIZED);
    let (code, _) = status(
        &app,
        "/api/containers/e/ports/8000/",
        &[("authorization", "Bearer wrong")],
    )
    .await;
    assert_eq!(code, StatusCode::UNAUTHORIZED);
    let (code, _) = status(
        &app,
        "/api/containers/e/ports/8000/",
        &[
            ("authorization", &bearer),
            ("origin", "http://attacker.example"),
        ],
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    // Every shape of the route reaches the proxy, which refuses a port the settings do not list.
    for path in [
        "/api/containers/e/ports/9000",
        "/api/containers/e/ports/9000/",
        "/api/containers/e/ports/9000/mcp",
    ] {
        let (code, message) = status(&app, path, &token).await;
        assert_eq!(code, StatusCode::FORBIDDEN, "{path}");
        assert!(message.contains("not proxied"), "{message}");
    }
    let (code, _) = status(&app, "/api/containers/missing/ports/8000/", &token).await;
    assert_eq!(code, StatusCode::NOT_FOUND);
    let (code, _) = status(&app, "/api/containers/e/ports/http/", &token).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    // Without MCP there is no proxy.
    let off = crate::test_support::app(true);
    container(&off, &[8000]);
    let (code, _) = status(&off, "/api/containers/e/ports/8000/", &token).await;
    assert_eq!(code, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_tools_and_guides_describe_the_ports_setting() {
    let (_, reply) = crate::test_support::mcp_request(mcp_app(), "tools/list", json!({})).await;
    let tools = reply["result"]["tools"].as_array().unwrap();
    for name in ["create_container", "update_container"] {
        let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
        assert_eq!(
            tool["inputSchema"]["properties"]["ports"]["items"]["maximum"], 65535,
            "{name}"
        );
    }
    let update = tools
        .iter()
        .find(|tool| tool["name"] == "update_container")
        .unwrap();
    assert!(
        update["description"]
            .as_str()
            .unwrap()
            .contains("name, internet, ports, groups, aliases and")
    );
    assert!(
        include_str!("../../../docs/API.md").contains("`/api/containers/{id}/ports/{port}/{path}`")
    );
    let guide = include_str!("../../../docs/MCP.md");
    assert!(guide.contains("/ports/PORT/") && guide.contains("/ports/8090/mcp"));
}

/// The container `e` behind a stand-in Docker CLI, which reports what `state.json` holds.
#[cfg(unix)]
fn docker(app: &mut Arc<App>, state: &Value) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("thoric-proxy-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let docker = dir.join("docker");
    crate::test_support::write_script(
        &docker,
        &format!(
            "#!/bin/sh\ncase \"$1 $2\" in\n  'container inspect') cat '{}/state.json' ;;\n  *) exit 1 ;;\nesac\n",
            dir.display()
        ),
    );
    std::fs::write(dir.join("state.json"), state.to_string()).unwrap();
    Arc::get_mut(app).unwrap().config.docker_bin = docker.display().to_string();
    dir
}

#[cfg(unix)]
fn inspected(running: bool) -> Value {
    json!([{
        "Id": "c0ffee",
        "Config": {"Labels": {"thoric.managed": "true", "thoric.environment": "e"}},
        "State": {"Running": running},
        "NetworkSettings": {"Networks": {"thoric-e-private": {"IPAddress": "127.0.0.1"}}}
    }])
}

#[cfg(unix)]
#[tokio::test]
async fn the_manager_proxies_a_running_desktops_listed_port() {
    let service = serve(echo().merge(websocket_echo())).await;
    let port = service.port();
    let mut app = mcp_app();
    let dir = docker(&mut app, &inspected(false));
    container(&app, &[port]);
    let bearer = format!("Bearer {TOKEN}");
    let (code, message) = status(
        &app,
        &format!("/api/containers/e/ports/{port}/"),
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(code, StatusCode::CONFLICT, "{message}");
    std::fs::write(dir.join("state.json"), inspected(true).to_string()).unwrap();
    let manager = serve(crate::router(app.clone())).await;
    let response = call(
        manager,
        get_request(&format!("/api/containers/e/ports/{port}/status"))
            .header("authorization", &bearer)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    // The dashboard's own policy still applies; the sandbox comes on top of it.
    let policies = response
        .headers()
        .get_all("content-security-policy")
        .iter()
        .map(|value| value.to_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert!(policies.iter().any(|policy| policy == "sandbox"));
    assert!(
        policies
            .iter()
            .any(|policy| policy.starts_with("default-src 'self'"))
    );
    let seen: Value = serde_json::from_slice(&body(response).await).unwrap();
    assert_eq!(seen["uri"], "/status");
    assert!(seen["headers"].get("authorization").is_none());
    echoes(
        manager,
        &format!("/api/containers/e/ports/{port}/echo"),
        Some(&bearer),
    )
    .await;
    std::fs::remove_dir_all(dir).unwrap();
}
