//! When the Settings page may change settings, how a change is checked before anything
//! restarts, and how its job follows `thoric apply-settings` through the folder the WSL
//! package watches, also once the manager it started from has been replaced.
use super::*;
use crate::test_support::DESKTOP_CONTEXT;
use axum::{body::Body, http::Request};
use std::path::PathBuf;
use tower::ServiceExt;

const FAST: Timing = Timing {
    take: Duration::from_millis(500),
    poll: Duration::from_millis(20),
    limit: Duration::from_secs(10),
};

fn folder() -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("thoric-settings-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn app_in(dir: &Path) -> Arc<App> {
    let mut app = crate::test_support::app(false);
    let config = &mut Arc::get_mut(&mut app).unwrap().config;
    config.update_dir = dir.into();
    config.environment = BTreeMap::from([
        ("AUTH_MODE".into(), "none".into()),
        ("UPLOAD_MAX_MB".into(), "1024".into()),
        ("ALLOWED_HOSTS".into(), "localhost".into()),
        ("DESKTOP_CONTEXT".into(), DESKTOP_CONTEXT.into()),
    ]);
    app
}

fn job(id: &str, kind: &str, status: &str) -> Job {
    serde_json::from_value(json!({
        "id": id, "kind": kind, "status": status, "created_at": 0, "finished_at": null,
        "log": "", "result": null,
    }))
    .unwrap()
}

async fn request(app: Arc<App>, method: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri("/api/configuration")
        .header("host", "localhost:8080")
        .header("x-thoric-request", "1");
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let response = crate::router(app)
        .oneshot(
            builder
                .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
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

async fn finished(app: &App, id: &str) -> Job {
    for _ in 0..500 {
        let job: Job = app.db.job(id).unwrap();
        if !package_requests::active(&job) {
            return job;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("job {id} did not finish");
}

/// The distribution's path unit and service as the package's units behave. Returns the request.
fn host(dir: &Path, output: &'static str, ended: &'static str) -> tokio::task::JoinHandle<String> {
    let dir = dir.to_owned();
    tokio::spawn(async move {
        while !dir.join(SETTINGS_REQUEST).exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let request = std::fs::read_to_string(dir.join(SETTINGS_REQUEST)).unwrap();
        std::fs::rename(dir.join(SETTINGS_REQUEST), dir.join(SETTINGS_STARTED)).unwrap();
        std::fs::write(dir.join(SETTINGS_LOG), output).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        std::fs::write(dir.join(SETTINGS_EXIT), format!("{ended}\n")).unwrap();
        request
    })
}

#[tokio::test]
async fn the_page_changes_settings_only_where_the_package_takes_them() {
    let dir = folder();
    let app = app_in(&dir);
    let (status, body) = request(app.clone(), "GET", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (&body["editable"], &body["job_id"]),
        (&json!(false), &Value::Null)
    );
    let change = json!({"changes": {"SECURE_COOKIE": "true"}});
    let (status, body) = request(app.clone(), "POST", Some(change.clone())).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"].as_str().unwrap().contains("edit .env"),
        "{body}"
    );
    assert!(!dir.join(SETTINGS_REQUEST).exists());
    // The package's folder, with its path unit watching it.
    std::fs::write(dir.join(SETTINGS_WATCHED), "").unwrap();
    let (_, body) = request(app.clone(), "GET", None).await;
    assert_eq!(body["editable"], true);
    let shown = body["variables"].as_array().unwrap();
    let entry = |name: &str| shown.iter().find(|entry| entry["name"] == name).unwrap();
    // The page edits the text the environment gave, not what the manager made of it.
    assert_eq!(
        (
            &entry("UPLOAD_MAX_MB")["editable"],
            &entry("UPLOAD_MAX_MB")["text"]
        ),
        (&json!(true), &json!("1024"))
    );
    assert_eq!(entry("TEMPLATES_URL")["text"], "");
    assert_eq!(entry("AUTH_MODE")["editable"], false);
    assert!(entry("AUTH_MODE").get("text").is_none());
    assert!(include_str!("../../../../docs/API.md").contains("| POST | `/api/configuration` |"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_change_is_checked_with_the_parser_the_manager_starts_with() {
    let dir = folder();
    std::fs::write(dir.join(SETTINGS_WATCHED), "").unwrap();
    let app = app_in(&dir);
    let check = |changes: Value| {
        let app = app.clone();
        async move {
            let (status, body) = request(
                app,
                "POST",
                Some(json!({"changes": changes, "dry_run": true})),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            body
        }
    };
    let body = check(json!({
        "UPLOAD_MAX_MB": "0",
        "IMAGE_ARCHIVE_MAX_MB": "lots",
        "SECURE_COOKIE": "yes",
        "TEMPLATES_URL": "ftp://example.com/index.json",
        "VOLUME_FILTER": "thoric.managed=false",
        "DESKTOP_IMAGE": "Not An Image",
        "GPU_ADAPTER": "$HOME",
        "ALLOWED_HOSTS": " desktop.lan",
        "ADMIN_PASSWORD": "a-new-password",
        "MANAGER_CONTAINER": "other",
    }))
    .await;
    let errors = body["errors"].as_object().unwrap();
    for (key, says) in [
        ("UPLOAD_MAX_MB", "between 1 and 65536"),
        ("IMAGE_ARCHIVE_MAX_MB", "whole number"),
        ("SECURE_COOKIE", "true or false"),
        ("TEMPLATES_URL", "http or https"),
        ("VOLUME_FILTER", "only with the value true"),
        ("DESKTOP_IMAGE", "DESKTOP_IMAGE"),
        ("GPU_ADAPTER", "without quotes, $"),
        ("ALLOWED_HOSTS", "spaces at either end"),
        ("ADMIN_PASSWORD", "thoric settings"),
        ("MANAGER_CONTAINER", "thoric settings"),
    ] {
        let error = errors[key]
            .as_str()
            .unwrap_or_else(|| panic!("{key}: {body}"));
        assert!(error.contains(says), "{key}: {error}");
    }
    assert_eq!(errors.len(), 10, "{body}");
    // Values the manager would start with pass.
    let body = check(json!({
        "UPLOAD_MAX_MB": "2048",
        "SECURE_COOKIE": "true",
        "TEMPLATES_URL": "none",
        "VOLUME_FILTER": "team=desktops",
        "GPU_ADAPTER": "GeForce RTX",
        "ALLOWED_HOSTS": "desktop.lan,other.lan",
        "DESKTOP_USER_NAMESPACES": "false",
    }))
    .await;
    assert_eq!(body["errors"], json!({}), "{body}");
    assert_eq!(body["error"], Value::Null);
    // Without a dry run, a refused change starts nothing.
    let (status, body) = request(
        app.clone(),
        "POST",
        Some(json!({"changes": {"UPLOAD_MAX_MB": "0"}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("UPLOAD_MAX_MB"));
    // Nor does a value the manager has already.
    let (status, body) = request(
        app.clone(),
        "POST",
        Some(json!({"changes": {"UPLOAD_MAX_MB": "1024"}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap().contains("already"),
        "{body}"
    );
    assert!(!dir.join(SETTINGS_REQUEST).exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_change_waits_for_running_jobs_and_never_runs_beside_an_update() {
    let dir = folder();
    std::fs::write(dir.join(SETTINGS_WATCHED), "").unwrap();
    let app = app_in(&dir);
    let change = |interrupt: bool| {
        Some(json!({"changes": {"SECURE_COOKIE": "true"}, "interrupt_jobs": interrupt}))
    };
    for kind in [crate::releases::JOB, JOB] {
        app.db
            .put("job", "other", &job("other", kind, "running"))
            .unwrap();
        let (status, body) = request(app.clone(), "POST", change(true)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{kind}");
        assert!(
            body["error"].as_str().unwrap().contains("running"),
            "{body}"
        );
    }
    // Another job ends with the manager, so it goes on only when asked to.
    app.db
        .put("job", "other", &job("other", "build", "running"))
        .unwrap();
    let (status, body) = request(app.clone(), "POST", change(false)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("interrupts 1 running job"),
        "{body}"
    );
    let (_, body) = request(
        app.clone(),
        "POST",
        Some(json!({"changes": {"SECURE_COOKIE": "true"}, "dry_run": true})),
    )
    .await;
    assert_eq!(body["running_jobs"], 1);
    assert!(!dir.join(SETTINGS_REQUEST).exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_change_follows_the_service_through_the_folder() {
    let dir = folder();
    std::fs::write(dir.join(SETTINGS_WATCHED), "").unwrap();
    // What the last change left behind.
    std::fs::write(dir.join(SETTINGS_LOG), "earlier output\n").unwrap();
    std::fs::write(dir.join(SETTINGS_EXIT), "success 0\n").unwrap();
    let app = app_in(&dir);
    let host = host(
        &dir,
        "Changing SECURE_COOKIE, UPLOAD_MAX_MB in .env\n",
        "success 0",
    );
    let changes = BTreeMap::from([
        ("SECURE_COOKIE".to_owned(), "true".to_owned()),
        ("UPLOAD_MAX_MB".to_owned(), "2048".to_owned()),
    ]);
    let (id, outcome) = jobs::start(app.clone(), JOB, JOB.into(), true, |app, id| async move {
        run(&app, &id, &changes, FAST).await
    })
    .unwrap();
    assert_eq!(outcome.await.unwrap(), Ok(json!({"applied": true})));
    // The request names the job first, then one setting a line.
    assert_eq!(
        host.await.unwrap(),
        format!("{id}\nSECURE_COOKIE=true\nUPLOAD_MAX_MB=2048\n")
    );
    let job: Job = app.db.job(&id).unwrap();
    assert!(job.log.contains("Changing SECURE_COOKIE"), "{}", job.log);
    assert!(!job.log.contains("earlier output"), "{}", job.log);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_secret_reaches_the_request_alone() {
    let dir = folder();
    std::fs::write(dir.join(SETTINGS_WATCHED), "").unwrap();
    let app = app_in(&dir);
    let url = "https://ntfy.sh/topic-of-the-phone?token=tk_notify";
    let (status, body) = request(
        app.clone(),
        "POST",
        Some(json!({"changes": {"NOTIFY_URL": url}, "dry_run": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["errors"], json!({}), "{body}");
    // A refused address is named by its place in the list, never by its text.
    let refused = "https://ntfy.sh/topic-of-the-phone,ftp://user:pass-of-the-phone@example.com";
    let (_, body) = request(
        app.clone(),
        "POST",
        Some(json!({"changes": {"NOTIFY_URL": refused}, "dry_run": true})),
    )
    .await;
    let error = body["errors"]["NOTIFY_URL"].as_str().unwrap();
    assert!(error.contains("address 2"), "{error}");
    assert!(!error.contains("of-the-phone"), "{error}");
    // Removing it is a change like any other.
    let (_, body) = request(
        app.clone(),
        "POST",
        Some(json!({"changes": {"NOTIFY_URL": ""}, "dry_run": true})),
    )
    .await;
    assert_eq!(body["errors"], json!({}), "{body}");
    let taken = {
        let dir = dir.clone();
        tokio::spawn(async move {
            let path = dir.join(SETTINGS_REQUEST);
            while !path.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&path).unwrap().permissions().mode();
                assert_eq!(mode & 0o777, 0o600, "{mode:o}");
            }
            let text = std::fs::read_to_string(&path).unwrap();
            std::fs::rename(&path, dir.join(SETTINGS_STARTED)).unwrap();
            std::fs::write(dir.join(SETTINGS_LOG), "  NOTIFY_URL=[hidden]\n").unwrap();
            std::fs::write(dir.join(SETTINGS_EXIT), "success 0\n").unwrap();
            text
        })
    };
    let changes = BTreeMap::from([("NOTIFY_URL".to_owned(), url.to_owned())]);
    let (id, outcome) = jobs::start(app.clone(), JOB, JOB.into(), true, |app, id| async move {
        run(&app, &id, &changes, FAST).await
    })
    .unwrap();
    assert_eq!(outcome.await.unwrap(), Ok(json!({"applied": true})));
    assert_eq!(taken.await.unwrap(), format!("{id}\nNOTIFY_URL={url}\n"));
    let job: Job = app.db.job(&id).unwrap();
    assert!(job.log.contains("NOTIFY_URL"), "{}", job.log);
    assert!(!job.log.contains("of-the-phone"), "{}", job.log);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_change_the_package_rolled_back_fails_its_job() {
    let dir = folder();
    let app = app_in(&dir);
    let host = host(
        &dir,
        "The manager did not answer with the new settings; .env is back as it was.\n",
        "exit-code 1",
    );
    let changes = BTreeMap::from([("WSL_GPU".to_owned(), "false".to_owned())]);
    let (id, outcome) = jobs::start(app.clone(), JOB, JOB.into(), true, |app, id| async move {
        run(&app, &id, &changes, FAST).await
    })
    .unwrap();
    let error = outcome.await.unwrap().unwrap_err();
    assert!(
        error.contains("apply-settings") && error.contains("exit-code 1"),
        "{error}"
    );
    host.await.unwrap();
    let job: Job = app.db.job(&id).unwrap();
    assert_eq!(job.status, "failed");
    assert!(job.log.contains("back as it was"), "{}", job.log);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn the_next_manager_follows_a_change_that_replaced_the_one_that_started_it() {
    let dir = folder();
    let app = app_in(&dir);
    let record = job("abc", JOB, "running");
    app.db.put("job", "abc", &record).unwrap();
    assert!(!resumable(&dir, &record));
    std::fs::write(dir.join(SETTINGS_STARTED), "abc\nSECURE_COOKIE=true\n").unwrap();
    assert!(resumable(&dir, &record));
    // An update's request is not a settings change's.
    assert!(!crate::releases::resumable(
        &dir,
        &job("abc", crate::releases::JOB, "running")
    ));
    assert!(!resumable(
        &dir,
        &job("abc", crate::releases::JOB, "running")
    ));
    std::fs::write(dir.join(SETTINGS_LOG), "Recreating the manager\n").unwrap();
    std::fs::write(dir.join(SETTINGS_EXIT), "success 0\n").unwrap();
    resume(app.clone(), record);
    let job = finished(&app, "abc").await;
    assert_eq!(job.status, "completed");
    assert!(job.log.contains("Recreating the manager"), "{}", job.log);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_package_takes_exactly_the_editable_settings_through_these_names() {
    let read = |path: &str| {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(path),
        )
        .unwrap()
    };
    let cli = read("packaging/wsl/thoric");
    // The list the package's command accepts is the one the page edits.
    let list = |name: &str| -> Vec<&str> {
        let start = format!("{name}=(");
        let line = cli
            .lines()
            .find(|line| line.starts_with(&start))
            .unwrap_or_else(|| panic!("{name} in packaging/wsl/thoric"));
        line[start.len()..line.len() - 1]
            .split_whitespace()
            .collect()
    };
    assert_eq!(list("DASHBOARD_SETTINGS"), super::super::EDITABLE);
    // And the values it keeps out of its log are those of the secrets the page sets.
    assert_eq!(list("DASHBOARD_SECRETS"), super::super::EDITABLE_SECRETS);
    let folder = "/opt/thoric/update";
    assert!(
        read("packaging/wsl/thoric-settings.path")
            .contains(&format!("\nPathExists={folder}/{SETTINGS_REQUEST}\n"))
    );
    let service = read("packaging/wsl/thoric-settings.service");
    for written in [
        format!(" {folder}/{SETTINGS_REQUEST} {folder}/{SETTINGS_STARTED}\n"),
        format!(" apply-settings {folder}/{SETTINGS_STARTED} >{folder}/{SETTINGS_LOG} 2>&1'"),
        format!(" {folder}/{SETTINGS_EXIT}'"),
    ] {
        assert!(service.contains(&written), "{written}");
    }
    assert!(read("packaging/wsl/Dockerfile").contains(&format!("{folder}/{SETTINGS_WATCHED}")));
    assert!(cli.contains(&format!("install_units \"$image\" {SETTINGS_WATCHED} ")));
    assert!(cli.contains("\n    apply-settings)\n"));
}

#[test]
fn the_api_guide_names_every_setting_the_page_changes() {
    let guide = include_str!("../../../../docs/API.md");
    let change = &guide[guide
        .find("`POST /api/configuration` changes settings")
        .unwrap()..];
    let change = &change[..change.find("\n\n").unwrap_or(change.len())];
    for name in super::super::EDITABLE {
        assert!(change.contains(&format!("`{name}`")), "{name}");
    }
}
