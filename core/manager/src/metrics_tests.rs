//! Pins down who `/metrics` answers and what: nothing while no token is set, also with sign-in
//! off, 401 without the right token and 429 once an address sent too many wrong ones, the Host
//! check every route gets, and a page in the text format whose containers carry only their ID and
//! name, whose counters follow jobs and tool calls, whose GPU is the host's alone and whose every
//! family the installation guide names.
use super::*;
use crate::test_support::{mcp_app, mcp_request, sampled_gpu, spec};
use axum::{body::Body, http::Request};
use serde_json::json;
use tower::ServiceExt;

const TOKEN: &str = "metrics-token-0123456789abcdef";

/// [`mcp_app`] with `METRICS_TOKEN` set, so a test can also make tool calls.
fn app_with_token() -> Arc<App> {
    let mut app = mcp_app();
    Arc::get_mut(&mut app).unwrap().config.metrics_token = Some(TOKEN.into());
    app
}

async fn scrape(
    app: &Arc<App>,
    host: &str,
    bearer: Option<&str>,
) -> (StatusCode, HeaderMap, String) {
    let mut request = Request::builder().uri("/metrics").header("host", host);
    if let Some(token) = bearer {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = crate::router(app.clone())
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (status, headers) = (response.status(), response.headers().clone());
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, headers, String::from_utf8(body.to_vec()).unwrap())
}

/// The samples of the page, without its comments.
fn samples(page: &str) -> Vec<&str> {
    page.lines().filter(|line| !line.starts_with('#')).collect()
}

fn record(id: &str, name: &str) -> Container {
    Container {
        id: id.into(),
        container: format!("thoric-{id}"),
        private_network: format!("thoric-{id}-private"),
        egress_network: format!("thoric-{id}-egress"),
        source_image_id: "sha256:base".into(),
        created_at: 1,
        spec: crate::model::ContainerSpec {
            name: name.into(),
            gpu: true,
            cpus: Some(2.0),
            ..spec()
        },
        last_error: None,
        desktop: None,
        working_image: None,
        setup: None,
    }
}

#[tokio::test]
async fn the_route_is_not_there_while_no_token_is_set() {
    for auth in [true, false] {
        let app = crate::test_support::app(auth);
        for bearer in [None, Some(""), Some(TOKEN)] {
            let (status, ..) = scrape(&app, "localhost:8080", bearer).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "sign-in {auth}, {bearer:?}");
        }
    }
}

#[tokio::test]
async fn a_scrape_needs_the_token_and_wrong_ones_lock_the_address_out() {
    let app = app_with_token();
    let (status, headers, body) = scrape(&app, "localhost:8080", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        headers[header::WWW_AUTHENTICATE],
        "Bearer realm=\"Thoric metrics\""
    );
    // The body every error status carries, as the OpenAPI document says.
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap(),
        json!({"error": "A valid metrics bearer token is required"})
    );
    // Neither the dashboard's session nor the MCP token opens it.
    let (status, ..) = scrape(
        &app,
        "localhost:8080",
        Some("test-token-with-at-least-32-characters"),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, headers, page) = scrape(&app, "localhost:8080", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "text/plain; version=0.0.4; charset=utf-8"
    );
    assert!(page.contains("\nthoric_info{version=\""), "{page}");
    // A right token forgets the address's failures; then it has its whole allowance of wrong ones.
    for _ in 0..20 {
        let (status, ..) = scrape(&app, "localhost:8080", Some("wrong")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, ..) = scrape(&app, "localhost:8080", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    // The sign-in's own limit is not the scrapes'.
    assert_eq!(
        app.login_attempts.begin(None, Instant::now()),
        Verdict::Counted
    );
}

#[tokio::test]
async fn the_host_check_comes_first() {
    let app = app_with_token();
    let (status, ..) = scrape(&app, "metrics.example.com", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn running_containers_carry_their_id_and_name_and_the_gpu_is_the_host_s() {
    let app = app_with_token();
    let quoted = "Unity \"2022\"\nLTS \\ editor";
    app.db.put(CONTAINERS, "a", &record("a", quoted)).unwrap();
    app.db
        .put(CONTAINERS, "b", &record("b", "Stopped"))
        .unwrap();
    app.db.put(CONTAINERS, "c", &record("c", "Gone")).unwrap();
    let docker_id = "4f1c5a0b9e2d3c7a8b6f5e4d3c2b1a0f9e8d7c6b5a4f3e2d1c0b9a8f7e6d5c4b";
    let running = BTreeMap::from([(
        "a".to_owned(),
        crate::usage::engine::Running {
            docker_id: docker_id.into(),
            gpu: true,
        },
    )]);
    let states = BTreeMap::from([
        ("a".to_owned(), "running".to_owned()),
        ("b".to_owned(), "exited".to_owned()),
    ]);
    app.usage.listed(states, &running);
    app.usage.started("a", docker_id, 1_792_141_200.5);
    app.usage.counted(
        "a",
        docker_id,
        Counters {
            cpu_time: 12_500_000_000,
            cpu: 0.25,
            memory: 512 << 20,
            memory_limit: 4 << 30,
            disk_read: 4096,
            disk_write: 8192,
            net_rx: 1000,
            net_tx: 2000,
            pids: Some(42),
        },
    );
    sampled_gpu(&app.usage, 0, "NVIDIA GeForce RTX 4070", 37.0);
    let (status, _, page) = scrape(&app, "localhost:8080", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::OK);
    let samples = samples(&page);
    let labels = r#"{id="a",name="Unity \"2022\"\nLTS \\ editor"}"#;
    for expected in [
        r#"thoric_containers{state="running"} 1"#.to_owned(),
        r#"thoric_containers{state="exited"} 1"#.to_owned(),
        r#"thoric_containers{state="missing"} 1"#.to_owned(),
        r#"thoric_containers{state="unavailable"} 0"#.to_owned(),
        format!("thoric_container_start_time_seconds{labels} 1792141200.5"),
        format!("thoric_container_cpu_seconds_total{labels} 12.5"),
        format!("thoric_container_cpu_cores{labels} 0.25"),
        format!("thoric_container_cpu_limit_cores{labels} 2"),
        format!("thoric_container_memory_bytes{labels} 536870912"),
        format!("thoric_container_memory_limit_bytes{labels} 4294967296"),
        format!("thoric_container_network_receive_bytes_total{labels} 1000"),
        format!("thoric_container_network_transmit_bytes_total{labels} 2000"),
        format!("thoric_container_block_read_bytes_total{labels} 4096"),
        format!("thoric_container_block_write_bytes_total{labels} 8192"),
        format!("thoric_container_pids{labels} 42"),
        r#"thoric_host_gpu_state{state="sampling"} 1"#.to_owned(),
        r#"thoric_host_gpu_state{state="idle"} 0"#.to_owned(),
        r#"thoric_host_gpu_utilization_ratio{gpu="0",name="NVIDIA GeForce RTX 4070"} 0.37"#
            .to_owned(),
        r#"thoric_host_gpu_memory_used_bytes{gpu="0",name="NVIDIA GeForce RTX 4070"} 1073741824"#
            .to_owned(),
        "thoric_usage_available 1".to_owned(),
        "thoric_human_requests_pending 0".to_owned(),
        r#"thoric_jobs{state="running"} 0"#.to_owned(),
    ] {
        assert!(samples.contains(&expected.as_str()), "{expected}\n{page}");
    }
    // A stopped container has a state and no figures.
    assert!(!page.contains("id=\"b\""), "{page}");
    // The GPU is the host's: no container's series names it, and none of its own names a container.
    for line in &samples {
        if line.starts_with("thoric_container_") {
            assert!(!line.contains("gpu"), "{line}");
        }
        if line.starts_with("thoric_host_gpu_") {
            assert!(!line.contains("id=") && !line.contains("Unity"), "{line}");
        }
    }
    // Every family is introduced once, before its samples.
    let mut seen = Vec::new();
    for line in page.lines() {
        if let Some(rest) = line.strip_prefix("# TYPE ") {
            let name = rest.split(' ').next().unwrap();
            assert!(!seen.contains(&name), "{name} twice");
            seen.push(name);
        } else if !line.starts_with('#') {
            let name = line.split(['{', ' ']).next().unwrap();
            assert_eq!(seen.last(), Some(&name), "{line} outside its family");
        }
    }
}

#[tokio::test]
async fn a_docker_that_cannot_be_listed_leaves_every_container_unavailable() {
    let app = app_with_token();
    app.db.put(CONTAINERS, "a", &record("a", "A")).unwrap();
    let (_, _, page) = scrape(&app, "localhost:8080", Some(TOKEN)).await;
    let unlisted = samples(&page);
    assert!(unlisted.contains(&r#"thoric_containers{state="unavailable"} 1"#));
    assert!(unlisted.contains(&r#"thoric_containers{state="running"} 0"#));
    assert!(
        !page.contains("thoric_container_cpu_seconds_total{"),
        "{page}"
    );
    // A listing that fails between two that work hides the figures only while it lasts.
    let docker_id = "4f1c5a0b9e2d3c7a8b6f5e4d3c2b1a0f9e8d7c6b5a4f3e2d1c0b9a8f7e6d5c4b";
    let running = BTreeMap::from([(
        "a".to_owned(),
        crate::usage::engine::Running {
            docker_id: docker_id.into(),
            gpu: false,
        },
    )]);
    let states = BTreeMap::from([("a".to_owned(), "running".to_owned())]);
    app.usage.listed(states.clone(), &running);
    app.usage.started("a", docker_id, 1_792_141_200.0);
    app.usage.unlisted();
    let (_, _, page) = scrape(&app, "localhost:8080", Some(TOKEN)).await;
    assert!(
        !page.contains("thoric_container_start_time_seconds{"),
        "{page}"
    );
    app.usage.listed(states, &running);
    let (_, _, page) = scrape(&app, "localhost:8080", Some(TOKEN)).await;
    assert!(
        samples(&page)
            .contains(&r#"thoric_container_start_time_seconds{id="a",name="A"} 1792141200"#),
        "{page}"
    );
}

#[tokio::test]
async fn the_installation_guide_names_every_family_of_the_page() {
    let app = app_with_token();
    sampled_gpu(&app.usage, 0, "GPU", 1.0);
    let (_, _, page) = scrape(&app, "localhost:8080", Some(TOKEN)).await;
    let guide = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/INSTALLATION.md"),
    )
    .unwrap();
    let section = &guide[guide.find("\n## Prometheus metrics\n").unwrap()..];
    let families: Vec<&str> = page
        .lines()
        .filter_map(|line| line.strip_prefix("# TYPE "))
        .map(|rest| rest.split(' ').next().unwrap())
        .collect();
    assert!(families.len() > 20, "{families:?}");
    for family in families {
        assert!(
            section.contains(&format!("`{family}`")),
            "docs/INSTALLATION.md does not name {family} under Prometheus metrics"
        );
    }
}

#[tokio::test]
async fn jobs_and_tool_calls_are_counted_since_the_manager_started() {
    let app = app_with_token();
    let (_, outcome) = crate::jobs::start(
        app.clone(),
        "exec",
        "container:a".into(),
        false,
        |_, _| async { Ok(json!({})) },
    )
    .unwrap();
    outcome.await.unwrap().unwrap();
    // A refused argument still names a tool of the catalog; an invented name never becomes a label.
    mcp_request(
        app.clone(),
        "tools/call",
        json!({"name": "list_containers", "arguments": {"invented": true}}),
    )
    .await;
    mcp_request(
        app.clone(),
        "tools/call",
        json!({"name": "made_up_tool", "arguments": {}}),
    )
    .await;
    let (_, _, page) = scrape(&app, "localhost:8080", Some(TOKEN)).await;
    let samples = samples(&page);
    for expected in [
        r#"thoric_jobs_finished_total{kind="exec",status="completed"} 1"#,
        r#"thoric_mcp_tool_calls_total{tool="list_containers",outcome="refused"} 1"#,
        r#"thoric_mcp_tool_calls_total{tool="unknown",outcome="refused"} 1"#,
        r#"thoric_mcp_server_tool_calls_total{outcome="ok"} 0"#,
    ] {
        assert!(samples.contains(&expected), "{expected}\n{page}");
    }
    assert!(!page.contains("made_up_tool"), "{page}");
}
