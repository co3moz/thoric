//! Pins down how a script's end becomes its job's result: the exit code, whether it ran out of its
//! time, which `timeout` reports as 124, or 137 once the script outlived the TERM, only when the
//! time was up, and why a script that did not succeed failed. And what a request that waits is
//! answered with: the job with its result once the script ends in time, failed or not, the
//! running job with a hint when it does not, and `job_id` in every answer; and the files a script
//! takes with it, written as the desktop user before it runs and refused before anything does.
use super::*;
use std::os::unix::process::ExitStatusExt;

fn ran(code: Option<i32>) -> Ran {
    Ran {
        status: code.map(|code| std::process::ExitStatus::from_raw(code << 8)),
        tail: "last line\n".into(),
    }
}

#[test]
fn a_script_reports_its_exit_code_and_whether_its_time_ran_out() {
    let succeeded = Outcome::of(&ran(Some(0)), false);
    assert_eq!(
        succeeded,
        Outcome {
            exit_code: Some(0),
            timed_out: false,
            output_tail: "last line\n".into(),
        }
    );
    assert_eq!(succeeded.failure(300), None);
    let failed = Outcome::of(&ran(Some(2)), false);
    assert!(!failed.timed_out);
    assert_eq!(failed.failure(300).unwrap(), "Script exited with code 2");
    for code in [124, 137] {
        let stopped = Outcome::of(&ran(Some(code)), true);
        assert!(stopped.timed_out, "{code}");
        assert_eq!(
            stopped.failure(30).unwrap(),
            format!(
                "Script ran out of its time limit of 30 seconds and was stopped (exit code {code})"
            )
        );
        // The script's own exit with that code, before its time, is no timeout.
        assert!(!Outcome::of(&ran(Some(code)), false).timed_out, "{code}");
    }
    // The manager's own limit killed it.
    let killed = Outcome::of(&ran(None), false);
    assert!(killed.timed_out && killed.exit_code.is_none());
}

/// The Docker CLI stand-in of [`desktop`]. `INSPECT` and `CALLS` are filled in.
const DOCKER: &str = r#"#!/bin/sh
case "$1" in
  container) echo 'INSPECT' ;;
  exec)
    if [ "$6" = python3 ]; then
      if [ "${10}" = taken ] && [ "${12}" = false ]; then
        cat >/dev/null
        echo 'CONFLICT: File already exists.' >&2
        exit 17
      fi
      printf 'file %s %s/%s %s %s=' "$4" "$9" "${10}" "${11}" "${12}" >> 'CALLS'
      cat >> 'CALLS'
      echo >> 'CALLS'
    else
      script=$(cat)
      echo "script $4" >> 'CALLS'
      case "$script" in
        *fail*) echo boom; exit 3 ;;
        *slow*) exec sleep 30 ;;
        *) echo "ran $script" ;;
      esac
    fi ;;
  *) exit 1 ;;
esac
"#;

/// An installation whose Docker CLI is a stand-in for one running desktop, `c1`: a file's write
/// is recorded with its user, path, size, `replace` and content, or refused for a file named
/// `taken` without `replace`, and a script is recorded with its user, prints `ran` and itself,
/// exits with 3 when it says `fail` and outlives every wait when it says `slow`. Returns the file
/// the stand-in records in.
fn desktop() -> (Arc<App>, std::path::PathBuf) {
    let mut app = crate::test_support::app(false);
    let dir = std::env::temp_dir().join(format!("thoric-exec-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let calls = dir.join("calls");
    let docker = dir.join("docker");
    let inspect = json!([{
        "Id": "docker-c1",
        "Config": {"Labels": {"thoric.managed": "true", "thoric.environment": "c1"}},
        "State": {"Running": true},
    }]);
    crate::test_support::write_script(
        &docker,
        &DOCKER
            .replace("INSPECT", &inspect.to_string())
            .replace("CALLS", &calls.display().to_string()),
    );
    Arc::get_mut(&mut app).unwrap().config.docker_bin = docker.display().to_string();
    let record = crate::model::Container {
        id: "c1".into(),
        container: "thoric-c1".into(),
        private_network: "thoric-c1-private".into(),
        egress_network: "thoric-c1-egress".into(),
        source_image_id: "sha256:desktop".into(),
        created_at: 1,
        spec: crate::test_support::spec(),
        last_error: None,
        desktop: None,
        working_image: None,
        setup: None,
    };
    app.db.put(crate::model::CONTAINERS, "c1", &record).unwrap();
    (app, calls)
}

fn request(body: Value) -> Execute {
    serde_json::from_value(body).unwrap()
}

async fn answered(app: &Arc<App>, body: Value) -> Value {
    run(app.clone(), "c1".into(), request(body), None)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_script_that_ends_within_the_wait_is_answered_with_its_result() {
    let (app, _) = desktop();
    let job = answered(&app, json!({"script": "echo hi", "wait_seconds": 30})).await;
    assert_eq!(job["status"], "completed", "{job}");
    assert_eq!(job["job_id"], job["id"]);
    assert_eq!(job["result"]["exit_code"], 0);
    assert_eq!(job["result"]["timed_out"], false);
    assert_eq!(job["result"]["output_tail"], "ran echo hi\n");
    // The result carries the output, so the log does not repeat it unless it is asked for.
    assert_eq!(job["log"], "");
    assert_eq!(job["log_start"], job["log_end"]);
    assert!(job.get("hint").is_none());
    let job = answered(
        &app,
        json!({"script": "echo hi", "wait_seconds": 30, "log_tail_bytes": 4096}),
    )
    .await;
    let log = job["log"].as_str().unwrap();
    assert!(
        log.contains("ran echo hi") && log.contains("Script completed successfully."),
        "{log}"
    );
}

#[tokio::test]
async fn a_script_that_fails_within_the_wait_is_answered_as_a_failed_job_not_an_error() {
    let (app, _) = desktop();
    let job = answered(&app, json!({"script": "fail", "wait_seconds": 30})).await;
    assert_eq!(job["status"], "failed", "{job}");
    assert_eq!(job["result"]["exit_code"], 3);
    assert_eq!(job["result"]["timed_out"], false);
    assert_eq!(job["result"]["output_tail"], "boom\n");
    assert!(job["job_id"].is_string());
}

#[tokio::test]
async fn a_script_that_outlives_the_wait_is_answered_as_running_with_its_job() {
    let (app, _) = desktop();
    let started = std::time::Instant::now();
    let job = answered(&app, json!({"script": "slow", "wait_seconds": 1})).await;
    assert!(started.elapsed() >= std::time::Duration::from_secs(1));
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    assert_eq!(job["status"], "running", "{job}");
    assert_eq!(job["job_id"], job["id"]);
    assert!(job["result"].is_null());
    let hint = job["hint"].as_str().unwrap();
    assert!(hint.contains("get_job") && hint.contains("wait_seconds"));
    // The job is the one get_job goes on with.
    let id = job["job_id"].as_str().unwrap();
    assert_eq!(app.db.job(id).unwrap().status, "running");
}

#[tokio::test]
async fn without_a_wait_the_answer_is_the_job_id_alone() {
    let (app, _) = desktop();
    for body in [
        json!({"script": "echo hi"}),
        json!({"script": "echo hi", "wait_seconds": 0}),
    ] {
        let answer = answered(&app, body).await;
        assert_eq!(answer.as_object().unwrap().len(), 1, "{answer}");
        assert!(answer["job_id"].is_string());
    }
}

/// What `/mcp` and an endpoint run for the tool: it waits by itself, and `job_id` is in every
/// answer, as a client that only reads that expects.
#[tokio::test]
async fn the_tool_waits_a_few_seconds_unless_told_and_always_names_the_job() {
    let (app, _) = desktop();
    let call = |arguments: Value| {
        let mcp = crate::mcp::ThoricMcp::new(app.clone());
        async move {
            let caller = crate::human::Caller::default();
            mcp.dispatch_with("exec_container", arguments, caller)
                .await
                .unwrap()
                .structured_content
                .unwrap()
        }
    };
    let ended = call(json!({"container_id": "c1", "script": "echo hi"})).await;
    assert_eq!(ended["status"], "completed", "{ended}");
    assert_eq!(ended["result"]["exit_code"], 0);
    assert!(ended["job_id"].is_string());
    let at_once = call(json!({"container_id": "c1", "script": "slow", "wait_seconds": 0})).await;
    assert_eq!(at_once.as_object().unwrap().len(), 1, "{at_once}");
    assert!(at_once["job_id"].is_string());
    // The catalog offers what the tool takes, with the bounds of get_job's wait.
    let tool = crate::mcp::listed_tool("exec_container").unwrap();
    let properties = &tool.input_schema["properties"];
    assert_eq!(properties["wait_seconds"]["default"], MCP_WAIT);
    assert_eq!(
        properties["wait_seconds"]["maximum"],
        crate::jobs::read::MAX_WAIT
    );
    assert_eq!(properties["log_tail_bytes"]["maximum"], LOG_LIMIT);
    assert_eq!(properties["files"]["maxItems"], FILES);
}

#[tokio::test]
async fn files_are_written_as_the_desktop_user_before_the_script_runs_also_a_root_one() {
    let (app, calls) = desktop();
    let job = answered(
        &app,
        json!({
            "script": "python3 /tmp/work/task.py",
            "root": true,
            "wait_seconds": 30,
            "log_tail_bytes": 4096,
            "files": [
                {"path": "/tmp/work/task.py", "data": "print(1)"},
                {"path": "/home/desktop/in.bin", "data": "AAEC", "encoding": "base64", "replace": true},
            ],
        }),
    )
    .await;
    assert_eq!(job["status"], "completed", "{job}");
    assert_eq!(job["result"]["user"], "root");
    assert_eq!(
        std::fs::read(&calls).unwrap(),
        b"file 1001:1001 /tmp/work/task.py 8 false=print(1)\nfile 1001:1001 /home/desktop/in.bin 3 true=\x00\x01\x02\nscript root\n"
    );
    let log = job["log"].as_str().unwrap();
    assert!(
        log.contains("Wrote /tmp/work/task.py (8 bytes) as the desktop user."),
        "{log}"
    );
    assert!(
        !log.contains("print(1)"),
        "a file's content stays out of the log"
    );
}

#[tokio::test]
async fn a_file_that_is_not_written_fails_the_job_before_the_script_runs() {
    let (app, calls) = desktop();
    let job = answered(
        &app,
        json!({
            "script": "echo hi",
            "wait_seconds": 30,
            "files": [{"path": "/tmp/taken", "data": "x"}],
        }),
    )
    .await;
    assert_eq!(job["status"], "failed", "{job}");
    assert!(job["result"].is_null());
    let log = job["log"].as_str().unwrap();
    assert!(
        log.contains("Could not write /tmp/taken, so the script did not run")
            && log.contains("File already exists")
            && log.contains("Set replace to true"),
        "{log}"
    );
    assert!(!calls.exists(), "neither a file nor the script ran");
}

#[tokio::test]
async fn files_and_waits_are_refused_before_anything_runs() {
    let (app, calls) = desktop();
    let file = |path: &str| json!({"path": path, "data": "x"});
    let many: Vec<Value> = (0..=FILES).map(|i| file(&format!("/tmp/{i}"))).collect();
    let large = "x".repeat(files::FILE_LIMIT + 1);
    let half = "x".repeat(files::FILE_LIMIT);
    for (files, reason) in [
        (json!(many), "at most 16 files"),
        (json!([file("tmp/x")]), "canonical absolute"),
        (json!([file("/tmp/../etc/passwd")]), "canonical absolute"),
        (json!([file("/tmp/x"), file("/tmp/x")]), "twice"),
        (json!([{"path": "/tmp/x", "data": large}]), "512 KiB"),
        (
            json!([{"path": "/tmp/x", "data": "not base64!", "encoding": "base64"}]),
            "valid standard base64",
        ),
        (
            json!([
                {"path": "/tmp/a", "data": half},
                {"path": "/tmp/b", "data": half},
                file("/tmp/c"),
            ]),
            "1 MiB together",
        ),
    ] {
        let body = request(json!({"script": "echo hi", "files": files}));
        let error = run(app.clone(), "c1".into(), body, None).await.unwrap_err();
        assert_eq!(error.0, axum::http::StatusCode::BAD_REQUEST, "{reason}");
        assert!(error.1.contains(reason), "{}: {reason}", error.1);
    }
    // A file takes only the fields the route names, and a wait only what a job's read allows.
    assert!(
        serde_json::from_value::<Execute>(
            json!({"script": "id", "files": [{"path": "/tmp/x", "data": "x", "mode": "755"}]})
        )
        .is_err()
    );
    for body in [
        json!({"script": "id", "wait_seconds": crate::jobs::read::MAX_WAIT + 1}),
        json!({"script": "id", "log_tail_bytes": LOG_LIMIT + 1}),
    ] {
        let refused = run(app.clone(), "c1".into(), request(body), None).await;
        assert!(refused.is_err());
    }
    assert!(app.db.jobs_summary().unwrap().is_empty(), "no job started");
    assert!(!calls.exists());
}
