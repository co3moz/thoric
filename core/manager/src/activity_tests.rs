//! Pins down the agent activity log as `/mcp` writes it: every call is recorded with its caller,
//! container, kind and outcome whatever it returns, a token a tool returns is never kept, every
//! tool has an argument policy, `GET /api/activity` filters and pages and says how many rows the
//! setting keeps, a job's read and cancel name the job's container without using it, a poll is
//! one row, a scale set's route counts the calls of each instance, and the guides say what is
//! kept.
use super::*;
use crate::{
    model::{CONTAINERS, Container},
    test_support::{mcp_app, mcp_request},
};
use serde_json::json;

fn container(id: &str, name: &str) -> Container {
    Container {
        id: id.into(),
        container: format!("thoric-{id}"),
        private_network: format!("thoric-{id}-private"),
        egress_network: format!("thoric-{id}-egress"),
        source_image_id: "sha256:desktop".into(),
        created_at: 1,
        spec: serde_json::from_value(json!({"name": name, "image": "example/desktop:1"})).unwrap(),
        last_error: None,
        desktop: None,
        working_image: None,
        setup: None,
    }
}

async fn call(app: &Arc<App>, name: &str, arguments: Value) -> Value {
    let (_, body) = mcp_request(
        app.clone(),
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    )
    .await;
    body
}

fn find<'a>(rows: &'a [Entry], tool: &str) -> &'a Entry {
    rows.iter()
        .find(|row| row.tool == tool)
        .unwrap_or_else(|| panic!("{tool} was not recorded: {rows:?}"))
}

#[tokio::test]
async fn every_call_on_mcp_is_recorded_with_its_caller_container_and_outcome() {
    let app = mcp_app();
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Unity"))
        .unwrap();
    call(&app, "get_settings", json!({})).await;
    call(
        &app,
        "write_file",
        json!({"container_id": "c1", "path": "/home/desktop/.netrc", "data": "password hunter2-secret"}),
    )
    .await;
    let started = call(
        &app,
        "exec_container",
        json!({"container_id": "c1", "script": "apt-get install -y curl", "root": true}),
    )
    .await;
    call(&app, "no_such_tool", json!({})).await;
    call(
        &app,
        "delete_volume",
        json!({"name": "data", "force": true}),
    )
    .await;
    let rows = recorded(&app, 5).await;

    let read = find(&rows, "get_settings");
    assert_eq!(
        (
            read.endpoint.as_deref(),
            read.credential.as_str(),
            read.source.as_str()
        ),
        (None, "mcp_token", "tool")
    );
    assert_eq!((read.kind.as_str(), read.outcome.as_str()), ("read", "ok"));
    assert!(read.error.is_none() && read.container_id.is_none());
    // rmcp names itself for a client of an older protocol that keeps no session.
    assert_eq!(read.client, None);

    let write = find(&rows, "write_file");
    assert_eq!(write.container_id.as_deref(), Some("c1"));
    assert_eq!(write.container_name.as_deref(), Some("Unity"));
    assert_eq!(
        (write.kind.as_str(), write.outcome.as_str()),
        ("destructive", "error")
    );
    assert!(
        write
            .error
            .as_deref()
            .is_some_and(|error| !error.is_empty())
    );
    assert_eq!(write.arguments["data"]["$masked"], "string");
    assert!(
        !serde_json::to_string(write)
            .unwrap()
            .contains("hunter2-secret")
    );
    assert_eq!(write.summary, "/home/desktop/.netrc");

    // The script is kept, and the job it started is named.
    let exec = find(&rows, "exec_container");
    assert_eq!(exec.arguments["script"], "apt-get install -y curl");
    assert_eq!(exec.summary, "apt-get install -y curl · root");
    assert_eq!(exec.outcome, "ok");
    assert_eq!(
        exec.job_id.as_deref(),
        started["result"]["structuredContent"]["job_id"].as_str()
    );

    let unknown = find(&rows, "no_such_tool");
    assert_eq!(
        (unknown.kind.as_str(), unknown.outcome.as_str()),
        ("change", "refused")
    );
    assert_eq!(unknown.error.as_deref(), Some("Unknown Thoric tool"));
    // An argument the schema refuses is a refusal of a destructive tool.
    let refused = find(&rows, "delete_volume");
    assert_eq!(
        (refused.kind.as_str(), refused.outcome.as_str()),
        ("destructive", "refused")
    );
    assert_eq!(refused.error.as_deref(), Some("Unknown argument: force"));
}

#[tokio::test]
async fn a_token_a_tool_returns_is_never_recorded() {
    let app = mcp_app();
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome"))
        .unwrap();
    let created = call(
        &app,
        "create_mcp_endpoint",
        json!({"name": "chrome", "grants": [{"container_id": "c1", "scopes": ["view"], "servers": "all"}]}),
    )
    .await;
    let replaced = call(&app, "create_mcp_endpoint_token", json!({"name": "chrome"})).await;
    let tokens: Vec<&str> = [&created, &replaced]
        .iter()
        .map(|body| {
            body["result"]["structuredContent"]["token"]
                .as_str()
                .unwrap()
        })
        .collect();
    let rows = recorded(&app, 2).await;
    let text = serde_json::to_string(&rows).unwrap();
    for token in tokens {
        assert!(token.starts_with("bsk_") && !text.contains(token));
    }
    assert_eq!(find(&rows, "create_mcp_endpoint").summary, "chrome");
}

#[tokio::test]
async fn every_tool_has_an_argument_policy() {
    // A tool added without one keeps every argument as its size only, which hides what it did.
    let (_, body) = mcp_request(mcp_app(), "tools/list", json!({})).await;
    let tools = body["result"]["tools"].as_array().unwrap();
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        let Some(policy) = mask::policy(name) else {
            panic!("{name} has no argument policy in core/manager/src/activity/mask.rs");
        };
        // A policy names arguments the tool takes, so a renamed one is not silently masked.
        let properties = tool["inputSchema"]["properties"].as_object().unwrap();
        for (key, _, _) in policy {
            assert!(properties.contains_key(*key), "{name} takes no {key}");
        }
    }
}

#[test]
fn a_calls_kind_follows_its_annotations_and_a_desktop_server_never_reads() {
    let tool = |name: &str| crate::mcp::listed_tool(name).unwrap();
    let kind = |name: &str| Kind::of(tool(name).annotations.as_ref());
    assert_eq!(kind("list_containers"), Kind::Read);
    assert_eq!(kind("create_container"), Kind::Change);
    assert_eq!(kind("delete_container"), Kind::Destructive);
    // MCP's defaults: a tool that says nothing may destroy.
    assert_eq!(Kind::of(None), Kind::Destructive);
    let read = ToolAnnotations::new().read_only(true);
    assert_eq!(Kind::of_server(Some(&read)), Kind::Change);
    let additive = ToolAnnotations::new().read_only(false).destructive(false);
    assert_eq!(Kind::of_server(Some(&additive)), Kind::Change);
    let destructive = ToolAnnotations::new().destructive(true);
    assert_eq!(Kind::of_server(Some(&destructive)), Kind::Destructive);
}

#[test]
fn a_client_is_named_by_itself_and_never_by_the_sdks_stand_in() {
    assert_eq!(
        client(Some(Implementation::new("claude-code", "2.1.0"))),
        Some("claude-code 2.1.0".into())
    );
    assert_eq!(client(Some(Implementation::default())), None);
    assert_eq!(client(None), None);
    // No control character reaches the log.
    assert_eq!(
        client(Some(Implementation::new("evil\nINFO forged", "1"))),
        Some("evilINFO forged 1".into())
    );
}

/// A GET of the dashboard's API, with its status and JSON body.
async fn get(app: &Arc<App>, uri: &str) -> (axum::http::StatusCode, Value) {
    use tower::ServiceExt;
    let response = crate::router(app.clone())
        .oneshot(
            axum::http::Request::builder()
                .uri(uri)
                .header("host", "localhost:8080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn the_route_lists_the_newest_calls_with_their_filters_and_a_cursor() {
    // Without sign-in, so the dashboard's session is not needed here.
    let app = crate::test_support::app(false);
    for (tool, container) in [("a", "c1"), ("b", "c2"), ("c", "c1")] {
        let entry = Entry {
            credential: "mcp_token".into(),
            source: "tool".into(),
            tool: tool.into(),
            container_id: Some(container.into()),
            kind: "read".into(),
            arguments: json!({}),
            outcome: "ok".into(),
            ..Default::default()
        };
        store::insert(&app.db, &entry, KEPT).unwrap();
    }
    let (status, body) = get(&app, "/api/activity?container=c1&limit=1").await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    assert_eq!(body["items"][0]["tool"], "c");
    assert_eq!(body["kept"], KEPT);
    let next = body["next"].as_i64().unwrap();
    let (_, body) = get(&app, &format!("/api/activity?container=c1&before={next}")).await;
    assert_eq!(body["items"][0]["tool"], "a");
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["next"], Value::Null);
    let (status, body) = get(&app, "/api/activity?kind=everything").await;
    assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("kind"));
}

#[test]
fn the_guides_say_what_is_recorded_and_how_much_is_kept() {
    let api = include_str!("../../../docs/API.md");
    assert!(
        api.contains("| GET | `/api/activity` |"),
        "docs/API.md must list the route"
    );
    for filter in [
        "`container`",
        "`endpoint`",
        "`caller`",
        "`tool`",
        "`kind`",
        "`outcome`",
        "`before`",
        "`limit`",
    ] {
        assert!(api.contains(filter), "docs/API.md must explain {filter}");
    }
    // The download and what a repeated read leaves, which this release added.
    assert!(
        api.contains("| GET | `/api/activity/export` |")
            && api.contains("`format`")
            && api.contains("`jsonl`"),
        "docs/API.md must describe the download"
    );
    for column in export::COLUMNS.trim_end().split(',') {
        assert!(
            api.contains(&format!("`{column}`")),
            "docs/API.md must name the CSV column {column}"
        );
    }
    assert!(api.contains("| GET | `/api/scale-sets/{name}/activity` |"));
    let guide = include_str!("../../../docs/MCP.md");
    assert!(
        guide.contains("`ACTIVITY_KEEP`")
            && guide.contains(&format!(
                "within {} minutes",
                ["zero", "one", "two", "three", "four", "five", "six"]
                    [(merge::WINDOW / 60_000) as usize]
            )),
        "docs/MCP.md must say how many rows are kept and which reads share one"
    );
    assert!(
        include_str!("../../../docs/INSTALLATION.md").contains("| `ACTIVITY_KEEP` | `10000` |"),
        "docs/INSTALLATION.md must explain ACTIVITY_KEEP"
    );
    assert!(guide.contains("## Agent activity"));
    let kept = format!("{},{:03}", KEPT / 1000, KEPT % 1000);
    assert!(
        guide.contains(&kept),
        "docs/MCP.md must say {kept} calls are kept"
    );
    for limit in [mask::SCRIPT, mask::TEXT] {
        let shown = if limit >= 1000 {
            format!("{},{:03}", limit / 1000, limit % 1000)
        } else {
            limit.to_string()
        };
        assert!(
            guide.contains(&format!("{shown} characters")),
            "docs/MCP.md must say {shown}"
        );
    }
    assert!(include_str!("../../../docs/USAGE.md").contains("**Activity**"));
}

fn job(id: &str, resource: Option<&str>) -> crate::model::Job {
    crate::model::Job {
        id: id.into(),
        kind: "exec".into(),
        resource: resource.map(Into::into),
        status: "completed".into(),
        created_at: 1,
        finished_at: Some(2),
        log: String::new(),
        result: None,
        progress: None,
        stages: Vec::new(),
        script: None,
        name: None,
    }
}

#[tokio::test]
async fn a_jobs_read_and_cancel_name_the_container_the_job_belongs_to() {
    let mut app = mcp_app();
    Arc::get_mut(&mut app).unwrap().lifecycle =
        crate::lifecycle::Activity::started(std::time::Duration::from_secs(3600));
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Unity"))
        .unwrap();
    for (id, resource) in [
        ("script", Some("container:c1")),
        // Its container was deleted since.
        ("gone", Some("container:c9")),
        ("build", Some("image:i1")),
        ("loose", None),
    ] {
        app.db.put("job", id, &job(id, resource)).unwrap();
    }
    for id in ["script", "gone", "build", "loose", "missing"] {
        call(&app, "get_job", json!({"job_id": id})).await;
    }
    call(&app, "cancel_job", json!({"job_id": "script"})).await;
    let rows = recorded(&app, 6).await;
    let of = |tool: &str, id: &str| {
        let row = rows
            .iter()
            .find(|row| row.tool == tool && row.arguments["job_id"] == id)
            .unwrap_or_else(|| panic!("{tool} {id} was not recorded: {rows:?}"));
        (row.container_id.clone(), row.container_name.clone())
    };
    let unity = (Some("c1".to_owned()), Some("Unity".to_owned()));
    assert_eq!(of("get_job", "script"), unity);
    // A cancel that is refused, since the job ended, still says whose job it was.
    assert_eq!(of("cancel_job", "script"), unity);
    assert_eq!(of("get_job", "gone"), (Some("c9".into()), None));
    for id in ["build", "loose", "missing"] {
        assert_eq!(of("get_job", id), (None, None), "{id}");
    }
    // So the log filtered by the container keeps the polls of its jobs.
    let filter = store::Filter {
        container: Some("c1".into()),
        ..Default::default()
    };
    let tools: Vec<String> = store::query(&app.db, &filter)
        .unwrap()
        .into_iter()
        .map(|row| row.tool)
        .collect();
    assert_eq!(tools, ["cancel_job", "get_job"]);
    // Reading a job does not use its container, which an idle stop goes by.
    assert!(app.lifecycle.idle_for("c1").unwrap() >= std::time::Duration::from_secs(3000));
}

#[tokio::test]
async fn a_poll_over_mcp_is_one_row_and_a_call_in_between_starts_another() {
    let app = mcp_app();
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Unity"))
        .unwrap();
    app.db
        .put("job", "script", &job("script", Some("container:c1")))
        .unwrap();
    for _ in 0..5 {
        call(&app, "get_job", json!({"job_id": "script"})).await;
    }
    // The rows are written after each answer, so the last one is waited for.
    let polled = loop {
        let rows = recorded(&app, 1).await;
        if rows[0].repeats == 4 {
            break rows;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    assert_eq!(polled.len(), 1);
    assert_eq!(polled[0].container_id.as_deref(), Some("c1"));
    assert!(polled[0].last_at.is_some_and(|last| last >= polled[0].at));
    // A read of something else leaves the run alone, and the caller's next call in that
    // container ends it.
    call(&app, "list_jobs", json!({})).await;
    recorded(&app, 2).await;
    call(&app, "list_windows", json!({"container_id": "c1"})).await;
    recorded(&app, 3).await;
    call(&app, "get_job", json!({"job_id": "script"})).await;
    let rows = recorded(&app, 4).await;
    assert_eq!(
        rows.iter()
            .map(|row| (row.tool.as_str(), row.repeats))
            .collect::<Vec<_>>(),
        [
            ("get_job", 0),
            ("list_windows", 0),
            ("list_jobs", 0),
            ("get_job", 4)
        ]
    );
}

#[tokio::test]
async fn the_route_says_how_many_rows_the_setting_keeps() {
    let mut app = crate::test_support::app(false);
    Arc::get_mut(&mut app).unwrap().config.activity_keep = 250;
    let (_, body) = get(&app, "/api/activity").await;
    assert_eq!(body["kept"], 250);
}

#[tokio::test]
async fn a_scale_sets_route_counts_the_calls_of_each_instance() {
    use crate::scale::{Instance, SETS, Scaling, new_set};
    let app = crate::test_support::app(false);
    let mut set = new_set("re", &crate::test_support::spec(), &Scaling::default());
    for number in 1..=3u32 {
        set.instances.insert(
            number,
            Instance {
                container_id: format!("c{number}"),
                note: String::new(),
            },
        );
    }
    app.db.put(SETS, "re", &set).unwrap();
    let row = |container: &str, tool: &str, at: u64, outcome: &str| Entry {
        at,
        endpoint: Some("re".into()),
        credential: "endpoint_token".into(),
        source: "tool".into(),
        tool: tool.into(),
        container_id: Some(container.into()),
        kind: "read".into(),
        arguments: json!({}),
        outcome: outcome.into(),
        ..Default::default()
    };
    // Instance 1 polled a job six times and ran a script that failed; instance 2 made one
    // call; instance 3 none; and a container of no set is not the set's.
    for at in (1_000..=6_000).step_by(1_000) {
        app.calls
            .record(&app.db, &row("c1", "get_job", at, "ok"), KEPT)
            .unwrap();
    }
    for entry in [
        row("c1", "exec_container", 7_000, "error"),
        row("c2", "screenshot", 500, "ok"),
        row("elsewhere", "screenshot", 9_000, "refused"),
    ] {
        app.calls.record(&app.db, &entry, KEPT).unwrap();
    }
    let (status, body) = get(&app, "/api/scale-sets/re/activity").await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({"instances": [
            {"instance": 1, "container_id": "c1", "calls": 7, "errors": 1, "last_at": 7_000},
            {"instance": 2, "container_id": "c2", "calls": 1, "errors": 0, "last_at": 500},
            {"instance": 3, "container_id": "c3", "calls": 0, "errors": 0, "last_at": null},
        ]})
    );
    // The six polls are one row.
    assert_eq!(recorded(&app, 4).await.len(), 4);
    let (status, _) = get(&app, "/api/scale-sets/none/activity").await;
    assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
}
