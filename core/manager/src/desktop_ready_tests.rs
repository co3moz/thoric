//! Pins down when a started desktop counts as ready and when the wait gives up: ready once its
//! session has written the file, and failed with the exit code and the container's last log lines
//! when it stops, when Docker keeps restarting it, counted from the wait's own start, or when it is
//! not ready in time. The job log says that it waits only when it has to.
use super::*;
use crate::model::{CONTAINERS, Job};

/// A manager whose Docker CLI is a shell script in a new directory. The container `e` answers
/// `inspect` with `state.json`, where `LOOK` becomes how many times it was looked at, and reports
/// ready from the look in the file `ready-from` on, never without the file. Its log has a line on
/// each stream. The job `start` exists for the log lines.
#[cfg(unix)]
fn desktop(state: &str) -> (std::sync::Arc<App>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("thoric-ready-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("state.json"), state).unwrap();
    let docker = dir.join("docker");
    crate::test_support::write_script(
        &docker,
        &format!(
            r#"#!/bin/sh
cd '{}'
echo "$*" >> calls
case "$*" in
  'container inspect thoric-e')
    look=$(( $(cat looks 2>/dev/null || echo 0) + 1 )); echo $look > looks
    sed "s/LOOK/$look/" state.json ;;
  'exec --user 1001:1001 c0ffee test -e /run/desktop/ready')
    [ -e ready-from ] && [ "$(cat looks)" -ge "$(cat ready-from)" ] ;;
  'logs --tail 20 c0ffee') echo 'Desktop starting'; echo 'GPU requested, but OpenGL initialization failed.' >&2 ;;
  *) exit 1 ;;
esac
"#,
            dir.display()
        ),
    );
    let mut app = crate::test_support::app(false);
    let config = &mut std::sync::Arc::get_mut(&mut app).unwrap().config;
    config.docker_bin = docker.display().to_string();
    let env = Container {
        id: "e".into(),
        container: "thoric-e".into(),
        private_network: "thoric-e-private".into(),
        egress_network: "thoric-e-egress".into(),
        source_image_id: "sha256:desktop".into(),
        created_at: 1,
        spec: crate::test_support::spec(),
        last_error: None,
        desktop: None,
        working_image: None,
        setup: None,
    };
    app.db.put(CONTAINERS, "e", &env).unwrap();
    let job = Job {
        id: "start".into(),
        kind: "start".into(),
        resource: Some("container:e".into()),
        status: "running".into(),
        created_at: 1,
        finished_at: None,
        log: String::new(),
        result: None,
        progress: None,
        stages: Vec::new(),
        script: None,
        name: None,
    };
    app.db.put("job", "start", &job).unwrap();
    (app, dir)
}

/// A container in Docker's terms, labeled as the manager's container `e`.
#[cfg(unix)]
fn state(state: &str, restarts: &str) -> String {
    format!(
        r#"[{{"Id":"c0ffee","RestartCount":{restarts},"Config":{{"Labels":{{"thoric.managed":"true","thoric.environment":"e"}}}},"State":{state}}}]"#
    )
}

/// Patience with time for a busy machine, and patience a test that times out can wait out.
#[cfg(unix)]
const PATIENT: Patience = Patience {
    limit: Duration::from_secs(60),
    interval: Duration::from_millis(10),
    restarts: 3,
};
#[cfg(unix)]
const QUICK: Patience = Patience {
    limit: Duration::from_millis(300),
    interval: Duration::from_millis(10),
    restarts: 3,
};

#[cfg(unix)]
async fn wait(app: &App, patience: &Patience) -> Result<String> {
    let env: Container = app.db.get(CONTAINERS, "e").unwrap();
    let info = app.wait_ready(&env, "start", patience).await?;
    Ok(info["Id"].as_str().unwrap().to_owned())
}

#[cfg(unix)]
fn looks(dir: &std::path::Path) -> u32 {
    std::fs::read_to_string(dir.join("looks"))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn a_desktop_is_ready_once_its_session_says_so() {
    // Docker restarted it before this wait, which therefore counts none of those restarts.
    let (app, dir) = desktop(&state(r#"{"Running":true,"Restarting":false}"#, "7"));
    std::fs::write(dir.join("ready-from"), "3").unwrap();
    assert_eq!(wait(&app, &PATIENT).await.unwrap(), "c0ffee");
    assert_eq!(looks(&dir), 3);
    assert_eq!(
        app.db.job("start").unwrap().log,
        "Waiting for the desktop to be ready.\nThe desktop is ready.\n"
    );
    // A desktop that is ready at the first look is not waited for, and the log says nothing.
    assert_eq!(wait(&app, &PATIENT).await.unwrap(), "c0ffee");
    assert_eq!(
        app.db.job("start").unwrap().log,
        "Waiting for the desktop to be ready.\nThe desktop is ready.\n"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_desktop_that_stops_fails_with_its_exit_code_and_last_log_lines() {
    let (app, dir) = desktop(&state(
        r#"{"Running":false,"Restarting":false,"ExitCode":1,"Error":""}"#,
        "0",
    ));
    let error = wait(&app, &PATIENT).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "The desktop stopped before it was ready; it last exited with code 1. Its last log lines:\nDesktop starting\nGPU requested, but OpenGL initialization failed."
    );
    // A stopped container is not asked whether it is ready.
    let calls = std::fs::read_to_string(dir.join("calls")).unwrap();
    assert!(!calls.contains("exec"), "{calls}");
    assert_eq!(looks(&dir), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_desktop_docker_keeps_restarting_fails_after_a_few_restarts() {
    // Docker reports a container between two attempts as running and restarting, and counts one
    // more restart at every look.
    let (app, dir) = desktop(&state(
        r#"{"Running":true,"Restarting":true,"ExitCode":1,"Error":""}"#,
        "LOOK",
    ));
    let error = wait(&app, &PATIENT).await.unwrap_err().to_string();
    assert!(
        error.starts_with(
            "The desktop restarted 3 times without becoming ready; it last exited with code 1. Its last log lines:\n"
        ),
        "{error}"
    );
    assert!(error.ends_with("initialization failed."), "{error}");
    // The first look counts what came before; three more restarts end the wait.
    assert_eq!(looks(&dir), 4);
    let calls = std::fs::read_to_string(dir.join("calls")).unwrap();
    assert!(!calls.contains("exec"), "{calls}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_desktop_that_is_never_ready_fails_in_bounded_time() {
    let (app, dir) = desktop(&state(r#"{"Running":true,"Restarting":false}"#, "0"));
    let started = std::time::Instant::now();
    let error = wait(&app, &QUICK).await.unwrap_err().to_string();
    assert!(started.elapsed() >= QUICK.limit, "{:?}", started.elapsed());
    assert!(
        started.elapsed() < QUICK.limit * 20,
        "{:?}",
        started.elapsed()
    );
    assert!(
        error.starts_with("The desktop was not ready within 0 seconds. Its last log lines:\n"),
        "{error}"
    );
    // A desktop that still runs has not exited, whatever Docker kept from an earlier run.
    assert!(!error.contains("exited"), "{error}");
    assert!(looks(&dir) > 1);
    assert_eq!(
        app.db.job("start").unwrap().log,
        "Waiting for the desktop to be ready.\n"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_container_that_is_not_the_managers_is_not_waited_for() {
    let (app, dir) =
        desktop(r#"[{"Id":"c0ffee","Config":{"Labels":{}},"State":{"Running":true}}]"#);
    let error = wait(&app, &PATIENT).await.unwrap_err();
    assert_eq!(error.to_string(), "Container ownership does not match");
    std::fs::remove_dir_all(&dir).unwrap();
}
