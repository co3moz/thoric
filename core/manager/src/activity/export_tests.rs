//! Pins down the log's download: every kept row that the filters leave, across the pages it is
//! read in, as CSV or JSON lines; arguments as the log masks them and nothing a tool returned; a
//! cell a spreadsheet would run as a formula made text; and a format or filter that is not one
//! refused before anything is sent.
use super::*;
use crate::test_support::{mcp_app, mcp_request};
use axum::body::to_bytes;
use serde_json::{Value, json};

/// A GET of the dashboard's API, with its status, headers and body as text.
async fn get(app: &Arc<App>, uri: &str) -> (StatusCode, axum::http::HeaderMap, String) {
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
    let (parts, body) = response.into_parts();
    let bytes = to_bytes(body, 16 << 20).await.unwrap();
    (
        parts.status,
        parts.headers,
        String::from_utf8(bytes.to_vec()).unwrap(),
    )
}

fn row(tool: &str, container: &str) -> Entry {
    Entry {
        at: 1_760_000_000_123,
        credential: "mcp_token".into(),
        source: "tool".into(),
        tool: tool.into(),
        container_id: Some(container.into()),
        kind: "read".into(),
        arguments: json!({}),
        outcome: "ok".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn the_download_holds_every_row_the_filters_leave_across_its_pages() {
    let app = crate::test_support::app(false);
    // More than two pages, and rows of another container between them.
    let count = 2 * PAGE_LIMIT + 37;
    for n in 0..count {
        store::insert(&app.db, &row(&format!("t{n}"), "c1"), store::KEPT).unwrap();
        if n % 50 == 0 {
            store::insert(&app.db, &row("elsewhere", "c2"), store::KEPT).unwrap();
        }
    }
    let (status, headers, body) = get(&app, "/api/activity/export?container=c1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(headers["content-type"], "text/csv; charset=utf-8");
    let name = headers["content-disposition"].to_str().unwrap();
    assert!(
        name.starts_with("attachment; filename=\"thoric-activity-") && name.ends_with(".csv\""),
        "{name}"
    );
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines[0], COLUMNS.trim_end());
    assert_eq!(lines.len(), count + 1);
    // Newest first, each once, to the oldest.
    let tools: Vec<&str> = lines[1..]
        .iter()
        .map(|line| line.split(',').nth(9).unwrap())
        .collect();
    let expected: Vec<String> = (0..count).rev().map(|n| format!("t{n}")).collect();
    assert_eq!(tools, expected);
    assert!(
        lines[1].contains(",2025-10-09T08:53:20.123Z,"),
        "{}",
        lines[1]
    );

    let (status, headers, body) = get(&app, "/api/activity/export?format=jsonl&container=c2").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(headers["content-type"], "application/x-ndjson");
    assert!(
        headers["content-disposition"]
            .to_str()
            .unwrap()
            .ends_with(".jsonl\"")
    );
    let rows: Vec<Value> = body
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), count.div_ceil(50));
    assert!(rows.iter().all(|row| row["tool"] == "elsewhere"));
    assert!(rows[0]["id"].as_i64() > rows[1]["id"].as_i64());

    // Nothing is sent for a format or a filter that is not one.
    for uri in [
        "/api/activity/export?format=xlsx",
        "/api/activity/export?kind=everything",
        "/api/activity/export?limit=5",
    ] {
        let (status, _, body) = get(&app, uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
    }
}

#[tokio::test]
async fn the_download_keeps_arguments_masked_and_no_result() {
    let agents = mcp_app();
    let (_, answer) = mcp_request(
        agents.clone(),
        "tools/call",
        json!({"name": "write_file", "arguments": {
            "container_id": "c1",
            "path": "/home/desktop/.netrc",
            "data": "password hunter2-secret",
        }}),
    )
    .await;
    let recorded = super::super::recorded(&agents, 1).await;
    // The same rows in an installation without sign-in, whose route needs no session here.
    let app = crate::test_support::app(false);
    for row in &recorded {
        store::insert(&app.db, row, store::KEPT).unwrap();
    }
    for format in ["csv", "jsonl"] {
        let (status, _, body) = get(&app, &format!("/api/activity/export?format={format}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains("write_file") && body.contains("/home/desktop/.netrc"));
        assert!(!body.contains("hunter2-secret"), "{body}");
        assert!(body.contains("$masked"), "{body}");
        // A row has no column and no field for what a tool answered.
        assert!(!body.contains("structuredContent") && !body.contains("\"result\""));
    }
    // The answer itself is an error text here, which is all of a result the log keeps.
    assert_eq!(answer["result"]["isError"], true, "{answer}");
}

#[test]
fn a_cell_is_quoted_where_it_must_be_and_never_runs_as_a_formula() {
    assert_eq!(cell("get_job"), "get_job");
    assert_eq!(cell(""), "");
    assert_eq!(cell("a,b"), "\"a,b\"");
    assert_eq!(cell("say \"hi\""), "\"say \"\"hi\"\"\"");
    assert_eq!(cell("two\nlines"), "\"two\nlines\"");
    // What a spreadsheet would take for a formula, such as a script an agent wrote.
    assert_eq!(
        cell("=HYPERLINK(\"http://x\")"),
        "\"'=HYPERLINK(\"\"http://x\"\")\""
    );
    assert_eq!(cell("+1"), "\"'+1\"");
    assert_eq!(cell("-rf /"), "\"'-rf /\"");
    assert_eq!(cell("@SUM(A1)"), "\"'@SUM(A1)\"");
    assert_eq!(cell("\tcmd"), "\"'\tcmd\"");
    // JSON never begins as one.
    assert_eq!(cell("{}"), "{}");
}

#[test]
fn a_line_carries_the_row_as_the_list_answers_it() {
    let mut entry = row("get_job", "c1");
    entry.id = 7;
    entry.endpoint = Some("set".into());
    entry.container_name = Some("Worker, one".into());
    entry.arguments = json!({"job_id": "j1", "data": {"$masked": "string", "length": 3}});
    entry.summary = "j1".into();
    entry.duration_ms = 12;
    entry.repeats = 1000;
    entry.last_at = Some(1_760_000_600_000);
    let mut out = String::new();
    line(&mut out, &entry);
    assert_eq!(
        out,
        "7,2025-10-09T08:53:20.123Z,2025-10-09T09:03:20.000Z,1000,set,mcp_token,,tool,,get_job,c1,\"Worker, one\",read,ok,,12,,j1,\"{\"\"data\"\":{\"\"$masked\"\":\"\"string\"\",\"\"length\"\":3},\"\"job_id\"\":\"\"j1\"\"}\"\n"
    );
}

#[test]
fn moments_are_utc_dates() {
    assert_eq!(moment(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(moment(951_782_400_000), "2000-02-29T00:00:00.000Z");
    assert_eq!(moment(1_798_761_599_999), "2026-12-31T23:59:59.999Z");
    assert_eq!(stamp(1_798_761_599), "20261231-235959");
}
