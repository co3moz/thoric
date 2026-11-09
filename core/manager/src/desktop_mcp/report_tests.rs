//! What a desktop hears about its servers: its own servers' states and tools, the endpoints that
//! reach it with its own grant alone and never a token, and a report sent now, again after a
//! change, also an endpoint's, and not again without one.
use super::*;
use crate::{
    desktop_mcp::{
        endpoints::{ENDPOINTS, Grant, revoke_token, token},
        registry::{SERVERS, ServerRecord},
        scopes::Scope,
        tests::{endpoint, grant, server},
    },
    test_support::mcp_app,
};
use axum::extract::{Path, State};
use tokio::io::{AsyncBufReadExt, BufReader};

const WAIT: Duration = Duration::from_secs(5);

#[test]
fn a_report_carries_this_desktops_servers_and_grants_and_no_token() {
    let app = mcp_app();
    server(&app, "c1", "playwright", &["browser_navigate"]);
    server(&app, "c2", "other", &["secret_tool"]);
    let mut record: ServerRecord = app
        .db
        .get(SERVERS, &ServerRecord::key("c1", "playwright"))
        .unwrap();
    record.tools[0].description = Some(format!("{}\nSecond line", "x".repeat(300)).into());
    app.db
        .put(SERVERS, &ServerRecord::key("c1", "playwright"), &record)
        .unwrap();
    let broken = ServerRecord {
        container_id: "c1".into(),
        name: "broken".into(),
        error: Some("No such file".into()),
        tools: Vec::new(),
        tools_at: None,
        spec: None,
        ..record
    };
    app.db
        .put(SERVERS, &ServerRecord::key("c1", "broken"), &broken)
        .unwrap();
    let mut shared = endpoint(
        "shared",
        vec![
            grant("c1", "mine", &[Scope::View, Scope::Input]),
            grant("c2", "theirs", &[Scope::Commands]),
        ],
    );
    shared.token_hash = Some("feedfacefeedface".into());
    shared.token_created_at = Some(1);
    app.db.put(ENDPOINTS, "shared", &shared).unwrap();
    let elsewhere: Vec<Grant> = vec![grant("c2", "theirs", &[Scope::View])];
    app.db
        .put(ENDPOINTS, "elsewhere", &endpoint("elsewhere", elsewhere))
        .unwrap();

    let report = report(&app, "c1");
    let servers = report["servers"].as_array().unwrap();
    assert_eq!(
        servers
            .iter()
            .map(|s| (s["name"].as_str().unwrap(), s["state"].as_str().unwrap()))
            .collect::<Vec<_>>(),
        [("broken", "error"), ("playwright", "ready")]
    );
    assert_eq!(servers[0]["error"], "No such file");
    let tool = &servers[1]["tools"][0];
    assert_eq!(tool["name"], "browser_navigate");
    let description = tool["description"].as_str().unwrap();
    assert!(description.starts_with("xxx") && description.ends_with('…'));
    assert!(!description.contains("Second line") && description.len() < 210);
    assert_eq!(
        report["endpoints"],
        json!([{
            "name": "shared", "path": "/mcp/shared", "automatic": false, "has_token": true,
            "alias": "mine", "scopes": ["view", "input"], "profile": "full", "servers": "all",
            "excluded_tools": [],
        }])
    );
    let text = report.to_string();
    for hidden in ["feedface", "theirs", "secret_tool", "elsewhere", "commands"] {
        assert!(
            !text.contains(hidden),
            "{hidden} reached the desktop: {text}"
        );
    }
}

#[tokio::test]
async fn a_report_is_sent_now_and_again_only_after_a_change() {
    let app = mcp_app();
    server(&app, "c1", "playwright", &["browser_navigate"]);
    let (writer, reader) = tokio::io::duplex(1 << 20);
    let reporter = tokio::spawn(send_reports(app.clone(), "c1".into(), writer));
    let mut lines = BufReader::new(reader).lines();
    let mut next = async || -> Value {
        let line = tokio::time::timeout(WAIT, lines.next_line())
            .await
            .expect("a report")
            .unwrap()
            .unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let first = next().await;
    assert!(first["status"]["status_at"].as_u64().unwrap() > 0);
    assert_eq!(first["status"]["servers"][0]["name"], "playwright");
    assert_eq!(first["status"]["endpoints"], json!([]));

    // A wake without a change sends nothing; the change after it does.
    app.desktop_mcp.report_changes();
    tokio::time::sleep(SETTLE * 3).await;
    server(&app, "c1", "unity", &["build"]);
    app.desktop_mcp.report_changes();
    let second = next().await;
    let names = second["status"]["servers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, ["playwright", "unity"]);

    // The reporter stops once the helper is gone.
    drop(lines);
    server(&app, "c1", "godot", &["run"]);
    app.desktop_mcp.report_changes();
    tokio::time::timeout(WAIT, reporter)
        .await
        .expect("the reporter stops")
        .unwrap();
}

#[tokio::test]
async fn a_new_or_revoked_token_wakes_the_reporters() {
    let app = mcp_app();
    let covering = endpoint("shared", vec![grant("c1", "mine", &[Scope::View])]);
    app.db.put(ENDPOINTS, "shared", &covering).unwrap();
    let mut changes = app.desktop_mcp.reports.subscribe();
    let created = token(State(app.clone()), Path("shared".into()))
        .await
        .unwrap();
    assert!(changes.has_changed().unwrap());
    let reported = report(&app, "c1");
    assert_eq!(reported["endpoints"][0]["has_token"], true);
    let secret = created.0["token"].as_str().unwrap();
    assert!(!reported.to_string().contains(secret));
    changes.borrow_and_update();
    let _revoked = revoke_token(State(app.clone()), Path("shared".into()))
        .await
        .unwrap();
    assert!(changes.has_changed().unwrap());
    assert_eq!(report(&app, "c1")["endpoints"][0]["has_token"], false);
}
