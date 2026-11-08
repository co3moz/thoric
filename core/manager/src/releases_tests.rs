//! What the manager makes of GitHub's answer, when the dashboard may start an update, also for a
//! manager that follows `edge`, and how an update job follows `thoric update` through the folder
//! the WSL package watches, also once the update has replaced the manager that started it. An
//! update first copies the database, keeps the newest copies, and does not start without one.
use super::*;
use crate::package_requests::active;
use axum::{body::Body, http::Request};
use std::path::PathBuf;
use tower::ServiceExt;

const FAST: Timing = Timing {
    take: Duration::from_millis(500),
    poll: Duration::from_millis(20),
    limit: Duration::from_secs(10),
};

fn folder() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("thoric-update-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The manager's data folder beside the update folder, where an update copies the database.
fn data(dir: &Path) -> PathBuf {
    dir.join("data")
}

fn app_in(dir: &Path, check: bool) -> Arc<App> {
    let mut app = crate::test_support::app(false);
    let config = &mut Arc::get_mut(&mut app).unwrap().config;
    config.update_check = check;
    config.update_dir = dir.into();
    config.data = data(dir);
    std::fs::create_dir_all(&config.data).unwrap();
    app
}

/// The copies of the database in the data folder, by name.
fn copies(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(data(dir))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn job(id: &str, kind: &str, status: &str) -> Job {
    serde_json::from_value(json!({
        "id": id, "kind": kind, "status": status, "created_at": 0, "finished_at": null,
        "log": "", "result": null,
    }))
    .unwrap()
}

/// What a look at `edge` finds for a manager created from the tag.
fn edge_seen(newer: bool) -> edge::Look {
    edge::Look::Following {
        image: EDGE.into(),
        newer: Ok(newer),
    }
}
const EDGE: &str = "ghcr.io/co3moz/thoric:edge";

async fn call(app: Arc<App>, method: &str) -> (StatusCode, Value) {
    request(app, method, "/api/update").await
}

async fn request(app: Arc<App>, method: &str, uri: &str) -> (StatusCode, Value) {
    let response = crate::router(app)
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("host", "localhost:8080")
                .header("x-thoric-request", "1")
                .body(Body::empty())
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
        if !active(&job) {
            return job;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("job {id} did not finish");
}

/// The distribution's path unit and service as the package's units behave: they take the
/// request, write the output in pieces and then how the command ended. Returns the request.
fn host(
    dir: &Path,
    output: &'static [&'static str],
    ended: &'static str,
) -> tokio::task::JoinHandle<String> {
    let dir = dir.to_owned();
    tokio::spawn(async move {
        while !dir.join(REQUEST).exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let request = std::fs::read_to_string(dir.join(REQUEST)).unwrap();
        std::fs::rename(dir.join(REQUEST), dir.join(STARTED)).unwrap();
        let mut log = String::new();
        for piece in output {
            log.push_str(piece);
            std::fs::write(dir.join(LOG), &log).unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        std::fs::write(dir.join(EXIT), format!("{ended}\n")).unwrap();
        request
    })
}

#[test]
fn the_newest_release_is_the_version_of_its_tag() {
    assert_eq!(
        newest(br#"{"tag_name": "v0.7.0", "name": "Thoric v0.7.0"}"#).unwrap(),
        "0.7.0"
    );
    assert_eq!(newest(br#"{"tag_name": "1.2.3"}"#).unwrap(), "1.2.3");
    for body in [
        &b"{}"[..],
        br#"{"tag_name": "nightly"}"#,
        br#"{"tag_name": "v1.2"}"#,
        br#"{"tag_name": "v1.2.3-rc.1"}"#,
        br#"{"tag_name": "v1..3"}"#,
        br#"{"tag_name": "v+1.2.3"}"#,
        b"<html>rate limited</html>",
    ] {
        assert!(newest(body).is_err(), "{}", String::from_utf8_lossy(body));
    }
}

#[test]
fn only_a_later_release_is_newer() {
    assert!(newer("0.7.0", "0.6.0"));
    assert!(newer("0.10.0", "0.9.9"));
    assert!(newer("1.0.0", "0.99.99"));
    assert!(!newer("0.6.0", "0.6.0"));
    assert!(!newer("0.5.9", "0.6.0"));
    assert!(!newer("0.7.0-rc.1", "0.6.0"));
    assert!(!newer("0.7.0", "development"));
}

#[test]
fn a_failed_check_keeps_the_release_seen_before() {
    let releases = Releases::default();
    releases.record(Ok("999.0.0".into()));
    releases.record(Err(anyhow::anyhow!("GitHub is unreachable")));
    assert_eq!(releases.available().as_deref(), Some("999.0.0"));
    let check = releases.last.lock().unwrap().clone();
    assert_eq!(check.error.as_deref(), Some("GitHub is unreachable"));
    releases.record(Ok(VERSION.into()));
    assert_eq!(releases.available(), None);
}

#[tokio::test]
async fn a_check_asked_for_during_another_takes_its_answer() {
    let releases = Arc::new(Releases::default());
    let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let ask = |version: &'static str, answer: Option<tokio::sync::oneshot::Receiver<()>>| {
        let asked = asked.clone();
        async move {
            asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if let Some(answer) = answer {
                answer.await.unwrap();
            }
            anyhow::Ok(version.to_owned())
        }
    };
    let (answer, answered) = tokio::sync::oneshot::channel();
    let first = tokio::spawn({
        let (releases, ask) = (releases.clone(), ask("999.0.0", Some(answered)));
        async move {
            releases
                .check(ask, async { edge::Look::NotFollowing })
                .await
        }
    });
    while asked.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    // A second click while GitHub has not answered the first waits for that answer.
    let second = tokio::spawn({
        let (releases, ask) = (releases.clone(), ask("1000.0.0", None));
        async move {
            releases
                .check(ask, async { edge::Look::NotFollowing })
                .await
        }
    });
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    answer.send(()).unwrap();
    // The look that saw the release first is the one that announces it.
    assert_eq!(first.await.unwrap().as_deref(), Some("999.0.0"));
    assert_eq!(second.await.unwrap(), None);
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(releases.available().as_deref(), Some("999.0.0"));
    // One asked for afterwards asks again.
    assert_eq!(
        releases
            .check(ask("1000.0.0", None), async { edge::Look::NotFollowing })
            .await
            .as_deref(),
        Some("1000.0.0")
    );
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(releases.available().as_deref(), Some("1000.0.0"));
    // A release seen before is not announced again.
    assert_eq!(
        releases
            .check(ask("1000.0.0", None), async { edge::Look::NotFollowing })
            .await,
        None
    );
}

#[tokio::test]
async fn a_look_asks_about_edge_beside_the_release() {
    let releases = Releases::default();
    let release = || async { anyhow::Ok("999.0.0".to_owned()) };
    releases.check(release(), async { edge_seen(true) }).await;
    assert_eq!(
        releases.edge(),
        Some(edge::Edge {
            image: EDGE.into(),
            newer: true,
            error: None
        })
    );
    // GitHub failing does not drop what Docker said, nor the other way round.
    releases
        .check(async { Err(anyhow::anyhow!("rate limited")) }, async {
            edge::Look::Following {
                image: EDGE.into(),
                newer: Err(anyhow::anyhow!("the registry is unreachable")),
            }
        })
        .await;
    let check = releases.last.lock().unwrap().clone();
    assert_eq!(check.newest.as_deref(), Some("999.0.0"));
    assert_eq!(check.error.as_deref(), Some("rate limited"));
    let edge = check.edge.unwrap();
    assert!(edge.newer);
    assert_eq!(edge.error.as_deref(), Some("the registry is unreachable"));
    releases
        .check(release(), async { edge::Look::NotFollowing })
        .await;
    assert_eq!(releases.edge(), None);
}

#[tokio::test]
async fn a_release_is_announced_once_also_across_restarts() {
    let dir = folder();
    let started = || {
        let mut app = app_in(&dir, true);
        Arc::get_mut(&mut app).unwrap().config.notify =
            crate::notify::settings(&|key: &str| match key {
                "NOTIFY_URL" => Some("https://example.com/hook".into()),
                "NOTIFY_EVENTS" => Some("update_available".into()),
                _ => None,
            })
            .unwrap();
        app
    };
    let first = started();
    announce(&first, Some("999.0.0".into())).await;
    let sent = first.notifier.queued();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].title, "Thoric 999.0.0 is available");
    assert_eq!(
        sent[0].link.as_deref(),
        Some("https://github.com/co3moz/thoric/releases/tag/v999.0.0")
    );
    // The next manager, which sees the same release first again, stays quiet about it.
    let next = started();
    announce(&next, Some("999.0.0".into())).await;
    assert!(next.notifier.queued().is_empty());
    announce(&next, Some("999.0.1".into())).await;
    assert_eq!(next.notifier.queued().len(), 1);
    // Without the kind chosen nothing is sent, nor kept as sent.
    let quiet = app_in(&dir, true);
    announce(&quiet, Some("999.1.0".into())).await;
    assert!(quiet.notifier.queued().is_empty());
    assert_eq!(
        std::fs::read_to_string(data(&dir).join(ANNOUNCED)).unwrap(),
        "999.0.1"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_check_now_is_refused_while_checks_are_turned_off() {
    let dir = folder();
    let (status, body) = request(app_in(&dir, false), "POST", "/api/update/check").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("UPDATE_CHECK=false")
    );
    assert!(include_str!("../../../docs/API.md").contains("| POST | `/api/update/check` |"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn only_a_service_that_succeeded_completes_the_job() {
    assert_eq!(outcome("success 0\n").unwrap(), json!({"version": VERSION}));
    for ended in ["exit-code 1", "timeout TERM", "signal KILL", ""] {
        let error = outcome(ended).unwrap_err().to_string();
        assert!(error.contains(ended), "{error}");
    }
}

#[tokio::test]
async fn the_dashboard_updates_only_a_watched_package_to_a_newer_release() {
    let dir = folder();
    // Turned off, nothing is offered and nothing starts.
    let off = app_in(&dir, false);
    off.releases.record(Ok("999.0.0".into()));
    std::fs::write(dir.join(WATCHED), "").unwrap();
    let (status, body) = call(off.clone(), "GET").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (&body["enabled"], &body["installable"], &body["current"]),
        (&json!(false), &json!(false), &json!(VERSION))
    );
    assert_eq!(body["commit"], json!(crate::catalog::edge_commit()));
    assert_eq!(body["edge"], Value::Null);
    let (status, body) = call(off, "POST").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("UPDATE_CHECK=false")
    );

    // Any other installation hears of the release, without a way to update from here.
    std::fs::remove_file(dir.join(WATCHED)).unwrap();
    let app = app_in(&dir, true);
    app.releases.record(Ok("999.0.0".into()));
    let (_, body) = call(app.clone(), "GET").await;
    assert_eq!(body["available"], true);
    assert_eq!(body["newest"], "999.0.0");
    assert_eq!(
        body["release_url"],
        "https://github.com/co3moz/thoric/releases/tag/v999.0.0"
    );
    assert_eq!(body["installable"], false);
    assert_eq!(body["job_id"], Value::Null);
    let (status, body) = call(app.clone(), "POST").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"].as_str().unwrap().contains("WSL package"));

    // The package's folder, but no newer release.
    std::fs::write(dir.join(WATCHED), "").unwrap();
    app.releases.record(Ok(VERSION.into()));
    let (_, body) = call(app.clone(), "GET").await;
    assert_eq!(
        (&body["available"], &body["installable"]),
        (&json!(false), &json!(true))
    );
    let (status, _) = call(app.clone(), "POST").await;
    assert_eq!(status, StatusCode::CONFLICT);

    // A newer one waits for the jobs the restart would cut short.
    app.releases.record(Ok("999.0.0".into()));
    app.db
        .put("job", "busy", &job("busy", "build", "running"))
        .unwrap();
    let (status, body) = call(app.clone(), "POST").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"].as_str().unwrap().contains("running jobs"));
    assert!(!dir.join(REQUEST).exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn the_dashboard_updates_a_manager_that_follows_edge_to_a_newer_build_of_it() {
    let dir = folder();
    std::fs::write(dir.join(WATCHED), "").unwrap();
    let app = app_in(&dir, true);
    // A newer release does not count: thoric update downloads the tag again whatever it is.
    app.releases.record(Ok("999.0.0".into()));
    app.releases.record_edge(edge_seen(false));
    let (_, body) = call(app.clone(), "GET").await;
    assert_eq!(body["available"], true);
    assert_eq!(
        body["edge"],
        json!({
            "image": EDGE,
            "available": false,
            "error": null,
            "changes_url": crate::catalog::edge_commit()
                .map(|commit| format!("https://github.com/co3moz/thoric/compare/{commit}...master")),
        })
    );
    let (status, body) = call(app.clone(), "POST").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains(&format!("the newest build of {EDGE}")),
        "{body}"
    );

    // A newer build is offered without a newer release, and the update asks for it.
    app.releases.record(Ok(VERSION.into()));
    app.releases.record_edge(edge_seen(true));
    let (_, body) = call(app.clone(), "GET").await;
    assert_eq!(
        (&body["available"], &body["edge"]["available"]),
        (&json!(false), &json!(true))
    );
    let host = host(&dir, &["Pulling the manager\n"], "success 0");
    let (status, body) = call(app.clone(), "POST").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = body["job_id"].as_str().unwrap().to_owned();
    assert_eq!(host.await.unwrap(), format!("{id}\n"));
    let job = finished(&app, &id).await;
    assert_eq!(job.status, "completed", "{}", job.log);
    assert!(
        job.log.contains(&format!(
            "thoric update, which downloads {EDGE} again and recreates the manager"
        )),
        "{}",
        job.log
    );
    // The copy is named after the tag, so the next update of edge keeps it beside its own.
    let names = copies(&dir);
    let copy = names
        .iter()
        .find(|name| name.ends_with("-to-edge.sqlite"))
        .unwrap_or_else(|| panic!("{names:?}"));
    assert!(
        job.log.contains(&format!("Copied the database to {copy}")),
        "{}",
        job.log
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn an_update_follows_the_service_through_the_folder() {
    let dir = folder();
    // What the last update left behind.
    std::fs::write(dir.join(STARTED), "earlier\n").unwrap();
    std::fs::write(dir.join(LOG), "earlier output\n").unwrap();
    std::fs::write(dir.join(EXIT), "success 0\n").unwrap();
    let app = app_in(&dir, true);
    app.releases.record(Ok("999.0.0".into()));
    let host = host(
        &dir,
        &[
            "Pulling the manager\n",
            "The manager moves from 0.6.0 to 999.0.0.\nhalf a li",
            "ne\nthe last line, without its end",
        ],
        "success 0",
    );
    let (id, outcome) = jobs::start(app.clone(), JOB, JOB.into(), true, |app, id| async move {
        run(&app, &id, FAST).await
    })
    .unwrap();
    assert_eq!(outcome.await.unwrap(), Ok(json!({"version": VERSION})));
    assert_eq!(host.await.unwrap(), format!("{id}\n"));
    let job: Job = app.db.job(&id).unwrap();
    assert_eq!(job.status, "completed");
    assert!(job.log.contains("to 999.0.0"), "{}", job.log);
    assert!(
        job.log.contains(
            "Pulling the manager\nThe manager moves from 0.6.0 to 999.0.0.\nhalf a line\nthe last line, without its end"
        ),
        "{}",
        job.log
    );
    assert!(!job.log.contains("earlier output"), "{}", job.log);
    assert!(!dir.join(REQUEST).exists());
    // The database was copied before the request was left, and the copy holds this job.
    let [copy] = &copies(&dir)[..] else {
        panic!("{:?}", copies(&dir));
    };
    assert!(
        copy.starts_with("state-") && copy.ends_with("-to-999.0.0.sqlite"),
        "{copy}"
    );
    let copied = job.log.find(&format!("Copied the database to {copy}"));
    let asked = job.log.find("Asked the WSL distribution");
    assert!(copied.is_some() && copied < asked, "{}", job.log);
    let copy = crate::db::Db::open(&data(&dir).join(copy)).unwrap();
    assert_eq!(copy.job(&id).unwrap().status, "running");
    drop(copy);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn an_update_keeps_the_newest_copies_of_the_database() {
    let dir = folder();
    let app = app_in(&dir, true);
    let now = std::time::SystemTime::now();
    let older = ["state-0.1.0-to-0.2.0.sqlite", "state-0.2.0-to-0.3.0.sqlite"];
    let newer = ["state-0.3.0-to-0.4.0.sqlite", "state-0.4.0-to-0.5.0.sqlite"];
    for (age, name) in older.iter().chain(&newer).rev().enumerate() {
        let file = std::fs::File::create(data(&dir).join(name)).unwrap();
        file.set_modified(now - Duration::from_secs(3600 * (age as u64 + 1)))
            .unwrap();
    }
    // Not copies: the database itself, its journal and a file of someone else's.
    for other in [crate::db::FILE, "state.sqlite-wal", "notes.txt"] {
        std::fs::write(data(&dir).join(other), "").unwrap();
    }
    let copy = copy_database(&app, Some("999.0.0")).await.unwrap();
    let mut kept = vec![
        copy,
        newer[0].to_owned(),
        newer[1].to_owned(),
        crate::db::FILE.to_owned(),
        "state.sqlite-wal".to_owned(),
        "notes.txt".to_owned(),
    ];
    kept.sort();
    assert_eq!(copies(&dir), kept);
    // The same update once more replaces its copy rather than failing on it.
    copy_database(&app, Some("999.0.0")).await.unwrap();
    assert_eq!(copies(&dir), kept);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn an_update_does_not_start_without_a_copy_of_the_database() {
    let dir = folder();
    let mut app = app_in(&dir, true);
    Arc::get_mut(&mut app).unwrap().config.data = dir.join("missing");
    app.releases.record(Ok("999.0.0".into()));
    let (id, outcome) = jobs::start(app.clone(), JOB, JOB.into(), true, |app, id| async move {
        run(&app, &id, FAST).await
    })
    .unwrap();
    let error = outcome.await.unwrap().unwrap_err();
    assert!(
        error.contains("Could not copy the database before the update, so nothing was updated"),
        "{error}"
    );
    let job: Job = app.db.job(&id).unwrap();
    assert!(
        job.log.contains("ERROR: Could not copy the database"),
        "{}",
        job.log
    );
    assert!(!dir.join(REQUEST).exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_failed_update_fails_its_job() {
    let dir = folder();
    let app = app_in(&dir, true);
    let host = host(
        &dir,
        &["Could not find the newest release; nothing was updated.\n"],
        "exit-code 1",
    );
    let (id, outcome) = jobs::start(app.clone(), JOB, JOB.into(), true, |app, id| async move {
        run(&app, &id, FAST).await
    })
    .unwrap();
    let error = outcome.await.unwrap().unwrap_err();
    assert!(error.contains("exit-code 1"), "{error}");
    host.await.unwrap();
    let job: Job = app.db.job(&id).unwrap();
    assert_eq!(job.status, "failed");
    assert!(job.log.contains("nothing was updated"), "{}", job.log);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_request_nobody_takes_is_withdrawn() {
    let dir = folder();
    let app = app_in(&dir, true);
    app.db
        .put("job", "lonely", &job("lonely", JOB, "running"))
        .unwrap();
    let error = run(&app, "lonely", FAST).await.unwrap_err().to_string();
    assert!(error.contains("thoric-update.path"), "{error}");
    // A request taken later would start an update that no job follows.
    assert!(!dir.join(REQUEST).exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn the_next_manager_follows_an_update_that_replaced_the_one_that_started_it() {
    let dir = folder();
    let app = app_in(&dir, true);
    let mut record = job("abc", JOB, "running");
    record.log = "Asked the WSL distribution\nPulling the man".into();
    app.db.put("job", "abc", &record).unwrap();
    // Only a running update job whose request the distribution took goes on.
    assert!(!resumable(&dir, &record));
    std::fs::write(dir.join(STARTED), "other\n").unwrap();
    assert!(!resumable(&dir, &record));
    std::fs::write(dir.join(STARTED), "abc\n").unwrap();
    assert!(resumable(&dir, &record));
    assert!(!resumable(&dir, &job("abc", JOB, "completed")));
    assert!(!resumable(&dir, &job("abc", "build", "running")));

    std::fs::write(dir.join(LOG), "Pulling the manager\nRecreating it\n").unwrap();
    std::fs::write(dir.join(EXIT), "success 0\n").unwrap();
    resume(app.clone(), record);
    let job = finished(&app, "abc").await;
    assert_eq!(job.status, "completed");
    assert_eq!(job.result, Some(json!({"version": VERSION})));
    // The log is read again from its start, so the part the first manager saw is not doubled.
    assert!(
        job.log.contains("Pulling the manager\nRecreating it"),
        "{}",
        job.log
    );
    assert!(!job.log.contains("Pulling the man\n"), "{}", job.log);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_package_mounts_this_folder_and_its_units_write_these_names() {
    let read = |path: &str| {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(path),
        )
        .unwrap()
    };
    let folder = "/opt/thoric/update";
    assert!(read("compose.ghcr.yml").contains(&format!("      - ./update:{UPDATE_DIR}\n")));
    assert!(
        read("packaging/wsl/thoric-update.path")
            .contains(&format!("\nPathExists={folder}/{REQUEST}\n"))
    );
    let service = read("packaging/wsl/thoric-update.service");
    for written in [
        format!(" {folder}/{REQUEST} {folder}/{STARTED}\n"),
        format!(">{folder}/{LOG} 2>&1'"),
        format!(" {folder}/{EXIT}'"),
    ] {
        assert!(service.contains(&written), "{written}");
    }
    assert!(read("packaging/wsl/Dockerfile").contains(&format!("touch {folder}/{WATCHED}")));
    assert!(read("packaging/wsl/thoric").contains(&format!("install_units \"$image\" {WATCHED} ")));
}
