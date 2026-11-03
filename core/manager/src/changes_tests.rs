//! Pins down a container's change list: what a query may ask for, how the selection, the noise it
//! leaves out and the summary are counted, what an answer says about its base and mounts, the
//! details a running desktop adds, a stopped and a missing Docker container, and the MCP tool on
//! `/mcp` and on an endpoint of `files_read`.
use super::*;
use crate::model::{CONTAINERS, Container, WorkingImage};
use axum::extract::State;

fn query(value: Value) -> ChangeQuery {
    serde_json::from_value(value).unwrap()
}

#[test]
fn a_query_names_a_canonical_folder_known_kinds_and_a_bounded_limit() {
    let selection = query(json!({})).selection().unwrap();
    assert_eq!(
        selection,
        Selection {
            path: None,
            kinds: Kind::ALL.to_vec(),
            all: false,
            limit: 500
        }
    );
    let selection = query(json!({"path": "/home/desktop/project", "kinds": " deleted, added ,added", "all": true, "limit": 5000}))
        .selection()
        .unwrap();
    assert_eq!(selection.path.as_deref(), Some("/home/desktop/project"));
    assert_eq!(selection.kinds, [Kind::Deleted, Kind::Added]);
    assert_eq!(query(json!({"path": "/"})).selection().unwrap().path, None);
    for refused in [
        json!({"path": "home/desktop"}),
        json!({"path": "/home/desktop/../etc"}),
        json!({"path": "/home/desktop/"}),
        json!({"kinds": "added,moved"}),
        json!({"limit": 0}),
        json!({"limit": 5001}),
    ] {
        assert!(query(refused.clone()).selection().is_err(), "{refused}");
    }
    assert!(serde_json::from_value::<ChangeQuery>(json!({"root": true})).is_err());
}

#[test]
fn the_summary_counts_folders_below_the_path_and_the_desktop_homes_apart() {
    let folder = |path: &str, base: Option<&str>| folder_of(path, base).unwrap_or_default();
    assert_eq!(folder("/usr/local/bin/tool", None), "/usr");
    assert_eq!(
        folder("/home/desktop/project/main.py", None),
        "/home/desktop/project"
    );
    assert_eq!(
        folder("/home/desktop/notes.txt", None),
        "/home/desktop/notes.txt"
    );
    assert_eq!(folder("/home/desktop", None), "/home");
    assert_eq!(folder("/usr/local/bin/tool", Some("/usr")), "/usr/local");
    // The folder asked for is not one of its own folders.
    assert_eq!(folder_of("/usr", Some("/usr")), None);
}

/// A desktop's list: a fresh session, an agent's project of three files, a note and a deletion.
fn entries() -> Vec<Change> {
    let mut entries = source::cli_entries(
        "C /home
C /home/desktop
A /home/desktop/.cache
A /home/desktop/.cache/thumbnails
A /home/desktop/project
A /home/desktop/project/a.py
A /home/desktop/project/b.py
C /home/desktop/notes.txt
D /etc/legal
A /tmp/.X11-unix
",
    );
    noise::sort(&mut entries);
    entries
}

fn listed(query_value: Value) -> Listing {
    let entries = entries();
    let rules = noise::classify(&entries, &noise::Desktop::default());
    listing(entries, rules, &query(query_value).selection().unwrap())
}

fn paths(listing: &Listing) -> Vec<&str> {
    listing
        .changes
        .iter()
        .map(|change| change.path.as_str())
        .collect()
}

#[test]
fn noise_is_left_out_and_counted_by_rule_unless_all_is_asked() {
    let listing = listed(json!({}));
    assert_eq!(
        paths(&listing),
        [
            "/etc/legal",
            "/home/desktop/notes.txt",
            "/home/desktop/project",
            "/home/desktop/project/a.py",
            "/home/desktop/project/b.py"
        ]
    );
    assert_eq!(listing.total, 5);
    assert_eq!(
        listing.summary,
        json!({
            "total": 5, "added": 3, "changed": 1, "deleted": 1, "folders_truncated": false,
            "folders": [
                {"path": "/home/desktop/project", "total": 3, "added": 3, "changed": 0, "deleted": 0},
                {"path": "/etc", "total": 1, "added": 0, "changed": 0, "deleted": 1},
                {"path": "/home/desktop/notes.txt", "total": 1, "added": 0, "changed": 1, "deleted": 0},
            ]
        })
    );
    assert_eq!(listing.noise["left_out"], true);
    assert_eq!(listing.noise["count"], 5);
    let rule = |id: &str| {
        listing.noise["rules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|rule| rule["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(rule("caches")["count"], 2);
    assert!(
        rule("caches")["paths"]
            .as_array()
            .unwrap()
            .contains(&json!("/home/desktop/.cache/**"))
    );
    assert_eq!(rule("temporary")["count"], 1);
    assert_eq!(rule(noise::FOLDERS.0)["count"], 2);
    // A rule that left nothing out does not list its paths.
    assert_eq!(rule("session")["count"], 0);
    assert!(rule("session").get("paths").is_none());
    assert!(listing.changes.iter().all(|change| change.noise.is_none()));

    let all = listed(json!({"all": true}));
    assert_eq!(all.total, 10);
    assert_eq!(all.noise["left_out"], false);
    let home = all
        .changes
        .iter()
        .find(|change| change.path == "/home/desktop")
        .unwrap();
    assert_eq!(home.noise, Some(noise::FOLDERS.0));
    let shown = serde_json::to_value(home).unwrap();
    assert_eq!(
        shown,
        json!({"path": "/home/desktop", "kind": "changed", "noise": "folders"})
    );
}

#[test]
fn path_kinds_and_limit_narrow_the_list_and_say_when_it_is_cut() {
    let listing = listed(json!({"path": "/home/desktop/project", "limit": 2}));
    assert_eq!(
        paths(&listing),
        ["/home/desktop/project", "/home/desktop/project/a.py"]
    );
    assert_eq!(listing.total, 3);
    let folders = listing.summary["folders"].as_array().unwrap();
    assert_eq!(folders.len(), 2);
    assert_eq!(folders[0]["path"], "/home/desktop/project/a.py");
    // Noise is counted within the path alone.
    assert_eq!(listing.noise["count"], 0);
    let listing = listed(json!({"kinds": "deleted,changed"}));
    assert_eq!(paths(&listing), ["/etc/legal", "/home/desktop/notes.txt"]);
    assert_eq!(listing.summary["added"], 0);
}

#[test]
fn mounts_name_volumes_host_folders_and_memory_without_a_hosts_path() {
    let info = json!({
        "Mounts": [
            {"Type": "volume", "Name": "project", "Source": "/var/lib/docker/volumes/project/_data", "Destination": "/home/desktop/project", "RW": true},
            {"Type": "bind", "Source": "/usr/lib/wsl", "Destination": "/usr/lib/wsl", "RW": false},
        ],
        "HostConfig": {"Tmpfs": {"/tmp": "rw,nosuid,size=512m", "/run": "ro,size=1m"}},
    });
    assert_eq!(
        mounts(&info),
        [
            json!({"path": "/home/desktop/project", "type": "volume", "read_only": false, "volume": "project"}),
            json!({"path": "/run", "type": "tmpfs", "read_only": true}),
            json!({"path": "/tmp", "type": "tmpfs", "read_only": false}),
            json!({"path": "/usr/lib/wsl", "type": "bind", "read_only": true}),
        ]
    );
    assert!(mounts(&json!({})).is_empty());
}

fn record(working_image: Option<WorkingImage>) -> Container {
    Container {
        id: "c1".into(),
        container: "thoric-c1".into(),
        private_network: "thoric-c1-private".into(),
        egress_network: "thoric-c1-egress".into(),
        source_image_id: "sha256:desktop".into(),
        created_at: 1,
        spec: serde_json::from_value(json!({"name": "Agent", "image": "example/desktop:1"}))
            .unwrap(),
        last_error: None,
        desktop: None,
        working_image,
        setup: None,
    }
}

/// A Docker CLI stand-in for container `c1`, running when `running`, whose diff lists an agent's
/// project beside a session cache and a volume's mount point, and whose desktop describes the
/// project's file. It records its arguments, and what an exec was given on stdin, in the folder.
#[cfg(unix)]
fn desktop(running: bool) -> (Arc<App>, std::path::PathBuf) {
    let dir =
        std::env::temp_dir().join(format!("thoric-changes-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let info = json!([{
        "Id": "docker-c1",
        "Image": "sha256:commit",
        "Created": "2026-11-03T10:00:00.5Z",
        "Config": {"Image": "thoric/container:job", "Labels": {"thoric.managed": "true", "thoric.environment": "c1"}},
        "State": {"Running": running},
        "Mounts": [{"Type": "volume", "Name": "work", "Destination": "/home/desktop/work", "RW": true}],
        "HostConfig": {"Tmpfs": {"/tmp": "rw"}},
    }]);
    let script = r#"#!/bin/sh
cd 'DIR'
echo "$1 $2 $3" >> calls
case "$1 $2" in
  'container inspect') if [ -f missing ]; then echo 'Error: No such container: thoric-c1' >&2; exit 1; fi; printf '%s' 'INFO' ;;
  'ps -a') ;;
  'container diff') printf 'C /home\nC /home/desktop\nA /home/desktop/.cache\nA /home/desktop/work\nA /home/desktop/project\nA /home/desktop/project/main.py\nA /home/desktop/project/gone.txt\nD /etc/legal\n' ;;
  'exec --interactive') cat > stdin; printf '[{"type":"directory","size":4096,"modified_at":10,"mode":493},null,{"type":"file","size":12,"modified_at":11,"mode":420}]' ;;
  *) exit 1 ;;
esac
"#;
    let docker = dir.join("docker");
    crate::test_support::write_script(
        &docker,
        &script
            .replace("DIR", &dir.display().to_string())
            .replace("INFO", &info.to_string()),
    );
    let mut app = crate::test_support::app(false);
    Arc::get_mut(&mut app).unwrap().config.docker_bin = docker.display().to_string();
    app.db
        .put(
            CONTAINERS,
            "c1",
            &record(Some(WorkingImage {
                image_id: "sha256:commit".into(),
                reference: "thoric/container:job".into(),
            })),
        )
        .unwrap();
    (app, dir)
}

#[cfg(unix)]
async fn get(app: &Arc<App>, uri: &str) -> (StatusCode, Value) {
    use tower::ServiceExt;
    let response = crate::router(app.clone())
        .oneshot(
            axum::http::Request::builder()
                .uri(uri)
                .header("host", "localhost")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[cfg(unix)]
#[tokio::test]
async fn a_running_desktop_answers_with_its_base_mounts_and_the_home_files_details() {
    let (app, dir) = desktop(true);
    let (status, body) = get(&app, "/api/containers/c1/changes").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["running"], true);
    assert_eq!(
        body["base"],
        json!({"image": "thoric/container:job", "image_id": "sha256:commit", "commit": true, "created_at": 1_793_700_000})
    );
    assert_eq!(body["mounts"][0]["volume"], "work");
    assert_eq!(body["path"], "/");
    assert_eq!(
        body["changes"],
        json!([
            {"path": "/etc/legal", "kind": "deleted"},
            {"path": "/home/desktop/project", "kind": "added", "type": "directory", "modified_at": 10},
            {"path": "/home/desktop/project/gone.txt", "kind": "added"},
            {"path": "/home/desktop/project/main.py", "kind": "added", "type": "file", "size": 12, "modified_at": 11},
        ])
    );
    assert_eq!(
        (
            body["total"].clone(),
            body["returned"].clone(),
            body["truncated"].clone()
        ),
        (json!(4), json!(4), json!(false))
    );
    assert_eq!(body["details"], "read");
    // The mount point, the cache and the folders that hold them are counted, not listed.
    assert_eq!(body["noise"]["count"], 4);
    // The paths under the home went to the desktop on stdin, in the listed order.
    let given: Vec<String> =
        serde_json::from_slice(&std::fs::read(dir.join("stdin")).unwrap()).unwrap();
    assert_eq!(
        given,
        [
            "/home/desktop/project",
            "/home/desktop/project/gone.txt",
            "/home/desktop/project/main.py"
        ]
    );
    let calls = std::fs::read_to_string(dir.join("calls")).unwrap();
    assert!(calls.contains("container diff docker-c1"), "{calls}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_stopped_desktop_lists_its_changes_without_details_and_a_missing_one_says_so() {
    let (app, dir) = desktop(false);
    let (status, body) = get(&app, "/api/containers/c1/changes?kinds=added&limit=1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["details"], "stopped");
    assert_eq!(
        body["changes"],
        json!([{"path": "/home/desktop/project", "kind": "added"}])
    );
    assert_eq!(body["truncated"], true);
    assert!(
        !std::fs::read_to_string(dir.join("calls"))
            .unwrap()
            .contains("exec")
    );
    let (status, body) = get(&app, "/api/containers/c1/changes?kinds=moved").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    std::fs::write(dir.join("missing"), "").unwrap();
    let (status, body) = get(&app, "/api/containers/c1/changes").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("Agent has no Docker container")
    );
    let (status, _) = get(&app, "/api/containers/nobody/changes").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// One JSON-RPC message to `path` with `token`, in `session` when given, and the reply's message,
/// which a session sends on an event stream.
#[cfg(unix)]
async fn rpc(
    app: &Arc<App>,
    path: &str,
    token: &str,
    session: Option<&str>,
    message: Value,
) -> (Option<String>, Value) {
    use tower::ServiceExt;
    let mut request = axum::http::Request::builder()
        .method("POST")
        .uri(path)
        .header("host", "localhost:8080")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-11-25");
    if let Some(session) = session {
        request = request.header("mcp-session-id", session);
    }
    let response = crate::router(app.clone())
        .oneshot(
            request
                .body(axum::body::Body::from(message.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let session = response
        .headers()
        .get("mcp-session-id")
        .map(|value| value.to_str().unwrap().to_owned());
    let bytes = axum::body::to_bytes(response.into_body(), 8 << 20)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&bytes).to_string();
    let reply = serde_json::from_str(&text).unwrap_or_else(|_| {
        text.lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
            .next_back()
            .unwrap_or(Value::Null)
    });
    (session, reply)
}

#[cfg(unix)]
#[tokio::test]
async fn the_mcp_tool_reads_the_same_list_on_mcp_and_on_an_endpoint_of_files_read() {
    let (mut app, _dir) = desktop(false);
    {
        let config = &mut Arc::get_mut(&mut app).unwrap().config;
        config.auth = true;
        config.root_mcp_enabled = true;
        config.mcp_token = Some("test-token-with-at-least-32-characters".into());
    }
    let (_, body) = crate::test_support::mcp_request(
        app.clone(),
        "tools/call",
        json!({"name": "get_changes", "arguments": {"container_id": "c1", "path": "/etc"}}),
    )
    .await;
    let result = &body["result"];
    assert_eq!(result["isError"], false, "{body}");
    assert_eq!(
        result["structuredContent"]["changes"],
        json!([{"path": "/etc/legal", "kind": "deleted"}])
    );
    let tool = crate::mcp::listed_tool("get_changes").unwrap();
    let annotations = tool.annotations.unwrap();
    assert_eq!(
        (annotations.read_only_hint, annotations.idempotent_hint),
        (Some(true), Some(true))
    );
    // An endpoint of files_read offers it bound to its container, without container_id.
    let Json(created) = crate::desktop_mcp::create_endpoint(
        State(app.clone()),
        Json(
            serde_json::from_value(json!({"name": "reader", "grants": [{"container_id": "c1", "scopes": ["files_read"], "servers": []}]}))
                .unwrap(),
        ),
    )
    .await
    .unwrap();
    let token = created["token"].as_str().unwrap().to_owned();
    let (session, _) = rpc(
        &app,
        "/mcp/reader",
        &token,
        None,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
    )
    .await;
    let session = session.expect("a session");
    rpc(
        &app,
        "/mcp/reader",
        &token,
        Some(&session),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    let (_, listed) = rpc(
        &app,
        "/mcp/reader",
        &token,
        Some(&session),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    )
    .await;
    let bound = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "get_changes")
        .unwrap_or_else(|| panic!("{listed}"));
    assert!(bound["inputSchema"]["properties"]["container_id"].is_null());
    let (_, called) = rpc(
        &app,
        "/mcp/reader",
        &token,
        Some(&session),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_changes","arguments":{"path":"/etc"}}}),
    )
    .await;
    assert_eq!(called["result"]["isError"], false, "{called}");
    assert_eq!(
        called["result"]["structuredContent"]["changes"],
        json!([{"path": "/etc/legal", "kind": "deleted"}])
    );
}

#[test]
fn the_limits_the_dashboard_and_the_helper_take_are_these() {
    // The Changes view asks for at most this many entries, and file_ops.py describes at most
    // as many paths as one answer gives it: a smaller limit there would fail every answer.
    let dashboard = include_str!("../../web/src/lib/changes.ts");
    assert!(
        dashboard.contains(&format!("export const CHANGE_LIMIT = {LIMIT}")),
        "core/web/src/lib/changes.ts must name the manager's LIMIT"
    );
    assert!(
        crate::files::FILE_OPS.contains(&format!("STATS_LIMIT = {DETAILS}")),
        "core/manager/helpers/file_ops.py must describe DETAILS paths at once"
    );
}
