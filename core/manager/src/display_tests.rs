//! What a command on a desktop's display ends with when the display does not answer: a conflict
//! naming the newest programs connected to it that it does not keep waiting, with the application
//! the manager launched them as, and how to let it go. A command that finishes in time never has
//! the display looked at, and one on a display that answers, or whose look fails, is left to
//! finish.
use super::*;
use serde_json::json;

fn client(
    command: &str,
    pid: u32,
    age_seconds: u64,
    waiting: bool,
    application: Option<&str>,
) -> Client {
    Client {
        pid,
        command: command.into(),
        age_seconds,
        waiting,
        application_id: application.map(Into::into),
    }
}

#[test]
fn what_the_helper_prints_reads_as_the_display_it_found() {
    let read = |value: serde_json::Value| serde_json::from_value::<Display>(value).unwrap();
    assert_eq!(read(json!({"display": "answers"})), Display::Answers);
    assert_eq!(read(json!({"display": "gone"})), Display::Gone);
    assert_eq!(
        read(json!({"display": "held", "clients": [
            {"pid": 3683, "command": "import", "age_seconds": 412, "waiting": false,
             "application_id": "66bfce6a"},
        ]})),
        Display::Held {
            clients: vec![client("import", 3683, 412, false, Some("66bfce6a"))]
        }
    );
    // A listing that failed still says the display is held.
    assert_eq!(
        read(json!({"display": "held", "clients": [], "error": "Could not list"})),
        Display::Held { clients: vec![] }
    );
}

#[test]
fn a_held_display_names_the_newest_programs_it_does_not_keep_waiting() {
    // What re1 held on 2026-11-04: import waited for a click after its window had closed, while
    // the health check's xdpyinfo waited for the display.
    let clients = [
        client("xdpyinfo", 4490, 2, true, None),
        client("xdpyinfo", 4481, 16, true, None),
        client(
            "import",
            3683,
            412,
            false,
            Some("66bfce6ac73d4fdda56b648f9284db67"),
        ),
        client("Setup.exe", 3612, 420, false, None),
        client("IKernel.exe", 3617, 421, false, None),
        client("xfwm4", 163, 1740, false, None),
    ];
    let text = held(&clients);
    assert!(
        text.contains("Connected to it, newest first: import (pid 3683, application 66bfce6ac73d4fdda56b648f9284db67, started 7 min ago); Setup.exe (pid 3612, started 7 min ago); IKernel.exe (pid 3617, started 7 min ago)."),
        "{text}"
    );
    assert!(
        !text.contains("xfwm4"),
        "only the newest three are named: {text}"
    );
    assert!(!text.contains("xdpyinfo"), "{text}");
    assert!(text.contains(" 2 other programs wait for it."), "{text}");
    assert!(text.contains("stop_application"), "{text}");
    assert!(text.contains("one click on the desktop"), "{text}");
    assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'));
}

#[test]
fn a_held_display_without_programs_to_name_still_says_how_to_let_it_go() {
    let text = held(&[client("xdpyinfo", 9, 1, true, None)]);
    assert!(!text.contains("newest first"), "{text}");
    assert!(text.contains(" 1 other program waits for it."), "{text}");
    assert!(text.contains("stop_application"), "{text}");
}

#[test]
fn ages_read_in_the_unit_that_suits_them() {
    assert_eq!(age(0), "0 s");
    assert_eq!(age(89), "89 s");
    assert_eq!(age(90), "2 min");
    assert_eq!(age(412), "7 min");
    assert_eq!(age(5399), "90 min");
    assert_eq!(age(5400), "2 h");
}

#[test]
fn a_display_that_answers_refuses_nothing_and_one_that_does_not_is_a_conflict() {
    assert!(refusal(&Display::Answers).is_none());
    for display in [Display::Gone, Display::Held { clients: vec![] }] {
        let crate::error::Error(status, message) = refusal(&display).unwrap().into();
        assert_eq!(status, StatusCode::CONFLICT, "{message}");
        assert!(
            message.starts_with("The desktop's display is not "),
            "{message}"
        );
    }
}

/// An installation whose Docker CLI is a shell script that answers the display check with
/// `answer`, and the file it notes its calls in.
#[cfg(unix)]
fn desktop(answer: &str) -> (std::sync::Arc<App>, std::path::PathBuf) {
    let dir =
        std::env::temp_dir().join(format!("thoric-display-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let docker = dir.join("docker");
    let script = format!(
        "#!/bin/sh\necho \"$1 $2 $3 $4 $5 $6 $7\" >> '{calls}'\ncase \"$1 $2 $3 $4 $5 $6\" in\n  'exec --user desktop desktop-id python3 -I') {answer} ;;\n  *) exit 1 ;;\nesac\n",
        calls = dir.join("calls").display(),
    );
    crate::test_support::write_script(&docker, &script);
    let mut app = crate::test_support::app(false);
    std::sync::Arc::get_mut(&mut app).unwrap().config.docker_bin = docker.display().to_string();
    (app, dir)
}

#[cfg(unix)]
#[tokio::test]
async fn a_command_that_finishes_in_time_never_has_the_display_looked_at() {
    let (app, dir) = desktop("echo '{\"display\":\"gone\"}'");
    let done: anyhow::Result<u8> = watched(&app, "desktop-id", async { Ok(7) }).await;
    assert_eq!(done.unwrap(), 7);
    assert!(!dir.join("calls").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_command_on_a_held_display_ends_with_the_programs_that_hold_it() {
    let answer = json!({"display": "held", "clients": [
        {"pid": 3683, "command": "import", "age_seconds": 30, "waiting": false,
         "application_id": "66bfce6a"},
    ]});
    let (app, dir) = desktop(&format!("echo '{answer}'"));
    let started = std::time::Instant::now();
    let stuck = async {
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok::<_, crate::error::Error>(())
    };
    let crate::error::Error(status, message) =
        watched(&app, "desktop-id", stuck).await.unwrap_err();
    assert!(started.elapsed() < PATIENCE + Duration::from_secs(5));
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        message.contains("import (pid 3683, application 66bfce6a, started 30 s ago)"),
        "{message}"
    );
    let calls = std::fs::read_to_string(dir.join("calls")).unwrap();
    assert!(
        calls.starts_with("exec --user desktop desktop-id python3 -I -c"),
        "{calls}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_slow_command_on_a_display_that_answers_or_cannot_be_looked_at_finishes() {
    for answer in [
        "echo '{\"display\":\"answers\"}'",
        "echo 'No python3' >&2; exit 126",
    ] {
        let (app, dir) = desktop(answer);
        let slow = async {
            tokio::time::sleep(PATIENCE + Duration::from_millis(700)).await;
            Ok::<_, anyhow::Error>("done")
        };
        assert_eq!(
            watched(&app, "desktop-id", slow).await.unwrap(),
            "done",
            "{answer}"
        );
        assert!(dir.join("calls").exists(), "{answer}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
