//! The reaper: which containers a round finds due, what counts as use there, and what `act`
//! does under the container's lock, against a Docker CLI stand-in: the stop of an idle desktop,
//! the delete and the stop at an expiry, and nothing at all when the lock is busy or the
//! container is in use again.
use super::*;
use crate::{
    lifecycle::{Activity, Lifecycle},
    model::{Container, Job},
    usage::stats::Measure,
};

const MINUTE: Duration = Duration::from_secs(60);

fn record(id: &str, lifecycle: Lifecycle) -> Container {
    let mut spec = crate::test_support::spec();
    spec.name = format!("Desk {id}");
    spec.lifecycle = lifecycle;
    Container {
        id: id.into(),
        container: format!("thoric-{id}"),
        private_network: format!("thoric-{id}-private"),
        egress_network: format!("thoric-{id}-egress"),
        source_image_id: "sha256:desktop".into(),
        created_at: 1,
        spec,
        last_error: None,
        desktop: None,
        working_image: None,
        setup: None,
    }
}

fn idle(minutes: u32) -> Lifecycle {
    Lifecycle {
        idle_stop_minutes: minutes,
        ..Lifecycle::default()
    }
}

fn expired(on_expiry: OnExpiry) -> Lifecycle {
    Lifecycle {
        expires_at: Some(crate::model::now() - 1),
        on_expiry,
        ..Lifecycle::default()
    }
}

fn save(app: &App, record: &Container) {
    app.db.put(CONTAINERS, &record.id, record).unwrap();
}

fn running(names: &[&str]) -> HashSet<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn cpu(app: &App, id: &str, cores: f32) {
    for _ in 0..5 {
        app.usage.record(
            id,
            &Measure {
                cpu: cores,
                memory: 0,
                memory_limit: 0,
                online_cpus: 4,
                disk_read: 0.0,
                disk_write: 0.0,
                net_rx: 0.0,
                net_tx: 0.0,
            },
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_round_finds_the_running_desktops_idle_for_their_time() {
    let app = crate::test_support::app(false);
    let records = [
        record("idle", idle(1)),
        record("stopped", idle(1)),
        record("never", Lifecycle::default()),
        record("patient", idle(30)),
    ];
    let up = running(&["thoric-idle", "thoric-never", "thoric-patient"]);
    let reaper = Reaper::default();
    let now = crate::model::now();
    assert!(reaper.due(&app, &records, &[], &up, now).is_empty());
    tokio::time::advance(MINUTE).await;
    assert_eq!(reaper.due(&app, &records, &[], &up, now), ["idle"]);
    // A use that lasts, such as an open viewer, keeps it.
    let viewer = app.lifecycle.open("idle");
    tokio::time::advance(10 * MINUTE).await;
    assert!(reaper.due(&app, &records, &[], &up, now).is_empty());
    drop(viewer);
    tokio::time::advance(MINUTE).await;
    assert_eq!(reaper.due(&app, &records, &[], &up, now), ["idle"]);
}

#[tokio::test(start_paused = true)]
async fn a_scale_sets_idle_instances_go_highest_first_and_its_kept_ones_stay() {
    let app = crate::test_support::app(false);
    let records = [
        record("one", Lifecycle::default()),
        record("two", Lifecycle::default()),
        record("three", Lifecycle::default()),
        record("alone", Lifecycle::default()),
    ];
    let mut set: crate::scale::ScaleSet = serde_json::from_value(serde_json::json!({
        "name": "re", "created_at": 1, "template": crate::test_support::spec(),
        "min": 1, "max": 4, "idle_delete_minutes": 5,
        "instances": {
            "1": {"container_id": "one"},
            "2": {"container_id": "two"},
            "3": {"container_id": "three"},
        },
    }))
    .unwrap();
    // Instance 3 is stopped, which an idle delete takes too; "alone" is no instance at all.
    let up = running(&["thoric-one", "thoric-two", "thoric-alone"]);
    let reaper = Reaper::default();
    let now = crate::model::now();
    let sets = [set.clone()];
    assert!(reaper.due(&app, &records, &sets, &up, now).is_empty());
    tokio::time::advance(5 * MINUTE).await;
    assert_eq!(
        reaper.due(&app, &records, &sets, &up, now),
        ["three", "two"]
    );
    // An instance in use stays, and without an idle delete none goes.
    let using = app.lifecycle.open("two");
    assert_eq!(reaper.due(&app, &records, &sets, &up, now), ["three"]);
    drop(using);
    set.idle_delete_minutes = 0;
    assert!(reaper.due(&app, &records, &[set], &up, now).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_busy_cpu_counts_as_use_until_it_calms_down() {
    let app = crate::test_support::app(false);
    let records = [record("build", idle(1))];
    let up = running(&["thoric-build"]);
    let reaper = Reaper::default();
    cpu(&app, "build", 2.0);
    tokio::time::advance(5 * MINUTE).await;
    assert!(
        reaper
            .due(&app, &records, &[], &up, crate::model::now())
            .is_empty()
    );
    // A desktop at rest uses a few hundredths of a core.
    let calm = crate::test_support::app(false);
    cpu(&calm, "build", 0.03);
    tokio::time::advance(5 * MINUTE).await;
    assert_eq!(
        reaper.due(&calm, &records, &[], &up, crate::model::now()),
        ["build"]
    );
    // The round that found it busy noted the use, so its idle time starts there.
    assert_eq!(app.lifecycle.idle_for("build"), Some(5 * MINUTE));
}

#[tokio::test(start_paused = true)]
async fn an_expiry_is_due_running_or_not_unless_its_action_failed_lately() {
    let app = crate::test_support::app(false);
    let records = [
        record("gone", expired(OnExpiry::Delete)),
        record("ahead", {
            let mut ahead = expired(OnExpiry::Stop);
            ahead.expires_at = Some(crate::model::now() + 3600);
            ahead
        }),
    ];
    let mut reaper = Reaper::default();
    assert_eq!(
        reaper.due(&app, &records, &[], &running(&[]), crate::model::now()),
        ["gone"]
    );
    reaper.failed.insert("gone".into(), Instant::now());
    assert!(
        reaper
            .due(&app, &records, &[], &running(&[]), crate::model::now())
            .is_empty()
    );
}

/// An installation that started `ago`, with a Docker CLI stand-in that knows the desktop `id`,
/// running or not, stops it, removes it and lists nothing else, no volume of its own either, and
/// records its arguments in the returned file. `stop` fails when `stop_fails`.
#[cfg(unix)]
fn docker(id: &str, ago: Duration, up: bool, stop_fails: bool) -> (Arc<App>, std::path::PathBuf) {
    let mut app = crate::test_support::app(false);
    let dir = std::env::temp_dir().join(format!("thoric-reaper-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let calls = dir.join("calls");
    let info = serde_json::json!([{
        "Id": "cid",
        "State": {"Running": up},
        "Config": {"Labels": {"thoric.managed": "true", "thoric.environment": id}},
    }]);
    let stop = if stop_fails {
        "echo 'Error response from daemon: stuck' >&2; exit 1"
    } else {
        ":"
    };
    crate::test_support::write_script(
        &dir.join("docker"),
        &format!(
            "#!/bin/sh\necho \"$*\" >> '{calls}'\ncase \"$*\" in\n  'container inspect thoric-{id}') echo '{info}' ;;\n  'stop cid') {stop} ;;\n  'rm --force cid') ;;\n  'ps --filter label=thoric.managed=true --format {{{{.Names}}}}') echo thoric-{id} ;;\n  'ps -a --format {{{{.Names}}}}') echo thoric-{id} ;;\n  'network ls --format {{{{.Name}}}}') ;;\n  'volume ls --filter label=thoric.managed=true --filter label=thoric.environment={id} --format {{{{.Name}}}}') ;;\n  *) exit 1 ;;\nesac\n",
            calls = calls.display(),
        ),
    );
    let inner = Arc::get_mut(&mut app).unwrap();
    inner.config.docker_bin = dir.join("docker").display().to_string();
    inner.lifecycle = Activity::started(ago);
    (app, calls)
}

#[cfg(unix)]
fn calls(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[cfg(unix)]
fn job(app: &App, id: &str) -> Job {
    app.db.job(id).unwrap()
}

/// The job `act` started for the container `id` and saw end well, or `None` when it did nothing.
#[cfg(unix)]
async fn acted(app: &Arc<App>, id: &str) -> Option<String> {
    match act(app, id).await.unwrap() {
        Acted::Nothing => None,
        Acted::Ended(outcome) => Some(outcome.unwrap()),
        Acted::Running(job, _) => panic!("job {job} still runs"),
    }
}

#[tokio::test(start_paused = true)]
async fn a_job_that_outlasted_the_wait_is_settled_by_a_later_round() {
    let app = crate::test_support::app(false);
    let records = [record("slow", idle(1))];
    let up = running(&["thoric-slow"]);
    tokio::time::advance(2 * MINUTE).await;
    let mut reaper = Reaper::default();
    let (done, outcome) = tokio::sync::oneshot::channel();
    reaper.pending.insert("slow".into(), outcome);
    // While its job runs, the container is not looked at again.
    assert!(
        reaper
            .due(&app, &records, &[], &up, crate::model::now())
            .is_empty()
    );
    reaper.collect();
    assert!(reaper.pending.contains_key("slow"));
    done.send(Err("docker stop timed out".to_owned())).unwrap();
    reaper.collect();
    assert!(reaper.pending.is_empty());
    // It failed, so it waits for RETRY_AFTER.
    assert!(reaper.failed.contains_key("slow"));
    assert!(
        reaper
            .due(&app, &records, &[], &up, crate::model::now())
            .is_empty()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn an_idle_desktop_is_stopped_by_a_job_that_says_why() {
    let (app, log) = docker("desk", 2 * MINUTE, true, false);
    save(&app, &record("desk", idle(1)));
    let id = acted(&app, "desk").await.expect("a job");
    let job = job(&app, &id);
    assert_eq!(job.kind, IDLE_STOP);
    assert_eq!(job.status, "completed");
    assert_eq!(job.resource.as_deref(), Some("container:desk"));
    assert!(
        job.log.contains("nothing used its desktop for 1 minutes"),
        "{}",
        job.log
    );
    assert!(calls(&log).contains(&"stop cid".to_owned()));
    // The policy stays for its next start.
    assert_eq!(app.container("desk").unwrap().spec.lifecycle, idle(1));
}

#[cfg(unix)]
#[tokio::test]
async fn nothing_happens_while_the_lock_is_busy_or_the_desktop_is_used_again() {
    let (app, log) = docker("desk", 2 * MINUTE, true, false);
    save(&app, &record("desk", idle(1)));
    let held = app.hold("container:desk").await.unwrap();
    assert_eq!(acted(&app, "desk").await, None);
    drop(held);
    // Something began to use it after the round looked.
    let viewer = app.lifecycle.open("desk");
    assert_eq!(acted(&app, "desk").await, None);
    drop(viewer);
    // A settings change cleared the policy after the round looked.
    save(&app, &record("desk", Lifecycle::default()));
    assert_eq!(acted(&app, "desk").await, None);
    assert!(!calls(&log).iter().any(|call| call.starts_with("stop")));
    assert!(app.db.jobs_summary().unwrap().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn an_expired_container_is_deleted_with_its_records() {
    let (app, log) = docker("throwaway", MINUTE, true, false);
    save(&app, &record("throwaway", expired(OnExpiry::Delete)));
    let id = acted(&app, "throwaway").await.expect("a job");
    let job = job(&app, &id);
    assert_eq!(job.kind, EXPIRY_DELETE);
    assert_eq!(job.status, "completed");
    let result = job.result.unwrap();
    assert_eq!(result["deleted"], "throwaway");
    assert_eq!(result["name"], "Desk throwaway");
    assert_eq!(result["volumes_preserved"], true);
    assert!(app.container("throwaway").is_err());
    assert!(calls(&log).contains(&"rm --force cid".to_owned()));
}

#[cfg(unix)]
#[tokio::test]
async fn an_expiry_that_stops_clears_itself_also_on_a_stopped_container() {
    let (app, log) = docker("paused", MINUTE, false, false);
    let mut lifecycle = expired(OnExpiry::Stop);
    lifecycle.idle_stop_minutes = 10;
    save(&app, &record("paused", lifecycle));
    let id = acted(&app, "paused").await.expect("a job");
    let job = job(&app, &id);
    assert_eq!(job.kind, EXPIRY_STOP);
    assert!(job.log.contains("stopped already"), "{}", job.log);
    assert_eq!(app.container("paused").unwrap().spec.lifecycle, idle(10));
    assert!(!calls(&log).iter().any(|call| call.starts_with("stop")));
}

#[cfg(unix)]
#[tokio::test]
async fn a_failed_stop_is_reported_once_and_not_tried_again_at_once() {
    let (app, log) = docker("stuck", 2 * MINUTE, true, true);
    save(&app, &record("stuck", idle(1)));
    let mut reaper = Reaper::default();
    reaper.round(&app).await.unwrap();
    assert!(reaper.failed.contains_key("stuck"));
    reaper.round(&app).await.unwrap();
    let stops = calls(&log)
        .iter()
        .filter(|call| call.starts_with("stop"))
        .count();
    assert_eq!(stops, 1);
    let jobs = app.db.jobs_summary().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, "failed");
}

/// [`docker`]'s installation, sending notices of the lifecycle to an address.
#[cfg(unix)]
fn notifying(id: &str, stop_fails: bool) -> Arc<App> {
    let (mut app, _) = docker(id, 2 * MINUTE, true, stop_fails);
    Arc::get_mut(&mut app).unwrap().config.notify = crate::notify::settings(&|key| match key {
        "NOTIFY_URL" => Some("https://ntfy.sh/thoric-lifecycle-test".into()),
        "NOTIFY_EVENTS" => Some("container_lifecycle".into()),
        _ => None,
    })
    .unwrap();
    app
}

#[cfg(unix)]
#[tokio::test]
async fn a_stop_the_lifecycle_made_sends_one_notice_once_it_is_done() {
    let app = notifying("desk", false);
    save(&app, &record("desk", idle(1)));
    let job = acted(&app, "desk").await.expect("a job");
    let notices = app.notifier.queued();
    assert_eq!(notices.len(), 1);
    let notice = &notices[0];
    assert_eq!(notice.kind, crate::notify::Kind::ContainerLifecycle);
    assert_eq!(notice.title, "Desk desk was stopped");
    assert!(notice.body.contains("for 1 minutes"), "{}", notice.body);
    assert_eq!(notice.container.as_deref(), Some("Desk desk"));
    assert_eq!(notice.link.as_deref(), Some("#/containers"));
    assert_eq!(
        notice.key.as_deref(),
        Some(format!("container_lifecycle:desk:{job}").as_str())
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_delete_at_expiry_says_so_and_a_failed_stop_sends_nothing_of_its_own() {
    let app = notifying("throwaway", false);
    let mut lifecycle = expired(OnExpiry::Delete);
    lifecycle.delete_volumes = true;
    save(&app, &record("throwaway", lifecycle));
    acted(&app, "throwaway").await.expect("a job");
    let notices = app.notifier.queued();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].title, "Desk throwaway was deleted");
    assert!(
        notices[0].body.contains("with the volumes"),
        "{}",
        notices[0].body
    );
    assert_eq!(notices[0].link.as_deref(), Some("#/jobs"));
    // A failed job is a job_failed notice, which this installation does not send.
    let app = notifying("stuck", true);
    save(&app, &record("stuck", idle(1)));
    Reaper::default().round(&app).await.unwrap();
    assert!(app.notifier.queued().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn nothing_is_stopped_or_deleted_while_a_restore_runs() {
    let (app, log) = docker("restored", 2 * MINUTE, true, false);
    save(&app, &record("restored", expired(OnExpiry::Delete)));
    let restore: Job = serde_json::from_value(serde_json::json!({
        "id": "restore", "kind": "restore", "resource": "installation", "status": "running",
        "created_at": 1, "finished_at": null, "log": "", "result": null,
    }))
    .unwrap();
    app.db.put("job", "restore", &restore).unwrap();
    Reaper::default().round(&app).await.unwrap();
    assert!(app.container("restored").is_ok());
    assert!(calls(&log).is_empty(), "{:?}", calls(&log));
}
