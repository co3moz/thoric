//! Pins down the job history as the routes and the agent see it: a job that ends is counted in
//! its hour by class and by how it ended, the list route and `list_jobs` leave the scripts out or
//! name kinds while their default stays every kind, a list's kinds are checked, the counts' route
//! answers the hours of its range, and a read of a job that was pruned says so instead of a bare
//! `Record not found`, also over MCP.
use super::*;
use crate::test_support::{app, mcp_app, mcp_request};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

fn saved(app: &App, id: &str, kind: &str) {
    let job = Job {
        id: id.into(),
        kind: kind.into(),
        resource: None,
        name: None,
        status: "completed".into(),
        created_at: 1,
        finished_at: Some(2),
        log: String::new(),
        result: None,
        progress: None,
        stages: Vec::new(),
        script: None,
    };
    app.db.put("job", id, &job).unwrap();
}

async fn get(app: Arc<App>, path: &str) -> (StatusCode, Value) {
    let response = crate::router(app)
        .oneshot(
            Request::get(path)
                .header("host", "localhost:8080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    // A query the route cannot read is refused in plain text.
    (status, serde_json::from_slice(&body).unwrap_or_default())
}

fn listed(jobs: &Value) -> Vec<&str> {
    jobs.as_array()
        .unwrap()
        .iter()
        .map(|job| job["id"].as_str().unwrap())
        .collect()
}

#[test]
fn only_a_script_is_of_the_script_class() {
    assert_eq!(Class::of("exec"), Class::Script);
    for kind in ["build", "create", "snapshot", "update", "idle-stop", ""] {
        assert_eq!(Class::of(kind), Class::Other, "{kind}");
    }
    // The dashboard tells a script by the same kinds.
    let dashboard = include_str!("../../../web/src/lib/job-history.ts");
    assert!(
        dashboard.contains(r#"export const SCRIPT_KINDS = ["exec"]"#),
        "core/web/src/lib/job-history.ts must name the manager's SCRIPT_KINDS"
    );
    // The migration that began the counts spelled the class and the kind out; they stay.
    assert_eq!(SCRIPT_KINDS, ["exec"]);
    assert_eq!(
        (Class::Script.name(), Class::Other.name()),
        ("script", "other")
    );
}

#[test]
fn a_lists_kinds_are_names_separated_by_commas() {
    let filter = |kind: Option<&str>, scripts| {
        Listed {
            kind: kind.map(str::to_owned),
            scripts,
        }
        .filter()
    };
    assert_eq!(filter(None, None).unwrap(), Filter::default());
    assert_eq!(filter(Some(""), None).unwrap(), Filter::default());
    let named = filter(Some("build, snapshot,,idle-stop"), Some(false)).unwrap();
    assert_eq!(named.kinds, ["build", "snapshot", "idle-stop"]);
    assert_eq!(named.class, Some(Class::Other));
    assert_eq!(filter(None, Some(true)).unwrap().class, Some(Class::Script));
    for refused in ["Build", "exec' OR 1=1", "a b", &"x".repeat(65)] {
        let error = filter(Some(refused), None).unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_REQUEST, "{refused}");
    }
    let many = vec!["build"; MAX_KINDS + 1].join(",");
    assert!(filter(Some(&many), None).is_err());
}

#[tokio::test]
async fn a_job_that_ends_is_counted_by_class_and_by_how_it_ended() {
    let app = app(false);
    let before = now();
    let run = |kind: &'static str, fails: bool| {
        let (_, outcome) = crate::jobs::start(
            app.clone(),
            kind,
            "container:one".into(),
            false,
            move |_, _| async move {
                if fails {
                    anyhow::bail!("It failed");
                }
                Ok(json!({}))
            },
        )
        .unwrap();
        outcome
    };
    for outcome in [
        run("exec", false),
        run("exec", false),
        run("exec", true),
        run("build", false),
    ] {
        let _ = outcome.await.unwrap();
    }
    let counts = app.db.job_counts(before).unwrap();
    let total = |class: &str| {
        counts
            .iter()
            .filter(|count| count.class == class)
            .fold((0, 0, 0), |sum, count| {
                (
                    sum.0 + count.completed,
                    sum.1 + count.failed,
                    sum.2 + count.interrupted,
                )
            })
    };
    assert_eq!(total("script"), (2, 1, 0));
    assert_eq!(total("other"), (1, 0, 0));
    // The route answers them, each hour by when it began.
    let (status, body) = get(app.clone(), "/api/jobs/history?days=7").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["days"], 7);
    assert_eq!(body["kept_days"], 90);
    let hours = body["hours"].as_array().unwrap();
    assert_eq!(
        hours
            .iter()
            .map(|hour| hour["completed"].as_u64().unwrap())
            .sum::<u64>(),
        3
    );
    assert!(hours.iter().all(|hour| {
        let at = hour["at"].as_u64().unwrap();
        at % 3600 == 0 && at >= body["since"].as_u64().unwrap() && at <= now()
    }));
    for refused in ["days=0", "days=91", "days=many"] {
        let (status, _) = get(app.clone(), &format!("/api/jobs/history?{refused}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    }
    assert_eq!(get(app, "/api/jobs/history").await.1["days"], 30);
}

#[tokio::test]
async fn a_list_leaves_the_scripts_out_when_asked() {
    let app = app(false);
    for (id, kind) in [
        ("b", "build"),
        ("e1", "exec"),
        ("s", "snapshot"),
        ("e2", "exec"),
    ] {
        saved(&app, id, kind);
    }
    let list = |query: &'static str| {
        let app = app.clone();
        async move { get(app, &format!("/api/jobs{query}")).await }
    };
    // Without a filter, every kind, as it always was.
    assert_eq!(listed(&list("").await.1), ["e2", "s", "e1", "b"]);
    assert_eq!(listed(&list("?scripts=false").await.1), ["s", "b"]);
    assert_eq!(listed(&list("?scripts=true").await.1), ["e2", "e1"]);
    assert_eq!(listed(&list("?kind=build,exec").await.1), ["e2", "e1", "b"]);
    assert_eq!(listed(&list("?kind=snapshot&scripts=false").await.1), ["s"]);
    let (status, body) = list("?kind=Not%20a%20kind").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn an_agent_lists_without_scripts_and_reads_that_a_job_was_pruned() {
    let app = mcp_app();
    for (id, kind) in [
        ("b", "build"),
        ("e1", "exec"),
        ("e2", "exec"),
        ("e3", "exec"),
    ] {
        saved(&app, id, kind);
    }
    let call = |name: &str, arguments: Value| {
        let app = app.clone();
        let params = json!({"name": name, "arguments": arguments});
        async move {
            let (_, body) = mcp_request(app, "tools/call", params).await;
            body["result"].clone()
        }
    };
    let text = |result: &Value| result["content"][0]["text"].as_str().unwrap().to_owned();
    let ids = |result: &Value| -> Vec<String> {
        let jobs: Value = serde_json::from_str(&text(result)).unwrap();
        listed(&jobs["items"])
            .into_iter()
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(
        ids(&call("list_jobs", json!({})).await),
        ["e3", "e2", "e1", "b"]
    );
    assert_eq!(
        ids(&call("list_jobs", json!({"scripts": false})).await),
        ["b"]
    );
    assert_eq!(
        ids(&call("list_jobs", json!({"kind": "exec"})).await),
        ["e3", "e2", "e1"]
    );
    // Two scripts go; the read of one says what happened to it.
    app.db
        .prune_jobs(Kept {
            scripts: 1,
            others: 1,
        })
        .unwrap();
    let read = call("get_job", json!({"job_id": "b"})).await;
    assert_ne!(read["isError"], true, "{read}");
    let pruned = call("get_job", json!({"job_id": "e1"})).await;
    assert_eq!(pruned["isError"], true, "{pruned}");
    let message = text(&pruned);
    assert!(
        message.contains("The job e1 ended and its record is no longer kept")
            && message.contains("newest 1000 finished script jobs")
            && message.contains("newest 500 finished jobs of every other kind"),
        "{message}"
    );
    // The route says the same, as a missing record.
    let app = app.clone();
    let error = crate::jobs::read::answer(&app, "e2", &Default::default(), None)
        .await
        .unwrap_err();
    assert_eq!(error.0, StatusCode::NOT_FOUND);
    assert!(error.1.contains("no longer kept"), "{}", error.1);
    // An id of a job's shape the manager does not remember, such as one pruned before a
    // restart, may have been pruned; anything else is only missing.
    let unknown = "0123456789abcdef0123456789abcdef";
    let error = kept_error(&app, unknown);
    assert_eq!(error.0, StatusCode::NOT_FOUND);
    assert!(
        error.1.contains(&format!("No job {unknown} is kept")) && error.1.contains("If it ran"),
        "{}",
        error.1
    );
    for id in [
        "nothing",
        "0123456789ABCDEF0123456789ABCDEF",
        "0123456789abcdef",
    ] {
        assert_eq!(kept_error(&app, id).1, "Record not found", "{id}");
    }
    assert!(kept(&app, "b").is_ok());
}

fn kept_error(app: &App, id: &str) -> Error {
    kept(app, id).unwrap_err()
}
