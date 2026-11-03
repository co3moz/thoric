//! The notifier: `NOTIFY_EVENTS` chooses the kinds, a notice is bounded and queued without
//! waiting, a request for a person sends one notice however often it is waited on, a full queue
//! drops and says so once, a failing address is logged when it begins and ends and never with
//! its text, and the Management page's test reports each address by format and host alone.
use super::*;
use crate::{
    human::Caller,
    model::{CONTAINERS, Container},
};
use serde_json::json;

const NTFY: &str = "https://ntfy.sh/thoric-alerts-of-somebody?token=tk_secretaccesstoken";

/// Settings from `values`, as the manager reads them at startup.
fn read(values: &[(&str, &str)]) -> anyhow::Result<Settings> {
    settings(&|key: &str| {
        values
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| (*value).to_owned())
    })
}

/// [`crate::test_support::app`] with `NOTIFY_URL` and `NOTIFY_EVENTS` as given, and a container
/// named Unity.
fn app_with(values: &[(&str, &str)]) -> Arc<App> {
    let mut app = crate::test_support::app(true);
    Arc::get_mut(&mut app).unwrap().config.notify = read(values).unwrap();
    let container = Container {
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
    app.db.put(CONTAINERS, "c1", &container).unwrap();
    app
}

fn queued(app: &App) -> Vec<Notice> {
    app.notifier.queued()
}

fn notice(kind: Kind, key: Option<&str>) -> Notice {
    Notice {
        kind,
        title: "Title".into(),
        body: "Body".into(),
        container: None,
        link: None,
        key: key.map(str::to_owned),
    }
}

#[test]
fn notify_events_chooses_the_kinds_sent() {
    // Empty sends what is on by default: a request for a person.
    assert_eq!(read(&[]).unwrap().events, [Kind::HumanRequest]);
    assert_eq!(read(&[("NOTIFY_EVENTS", "none")]).unwrap().events, []);
    let both = read(&[("NOTIFY_EVENTS", " job_failed, human_request,job_failed ")]).unwrap();
    assert_eq!(both.events, [Kind::JobFailed, Kind::HumanRequest]);
    assert_eq!(both.events_text(), "job_failed,human_request");
    let error = read(&[("NOTIFY_EVENTS", "human_request,everything")])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("human_request, job_failed, update_available")
            && error.contains("everything"),
        "{error}"
    );
    // The test is never a kind to choose.
    assert!(read(&[("NOTIFY_EVENTS", "test")]).is_err());
    // Every kind to choose has a name of its own.
    let mut names: Vec<&str> = EVENTS.iter().map(|(_, name, ..)| *name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), EVENTS.len());
}

#[test]
fn every_kind_is_documented_and_named_on_the_management_page() {
    let read = |path: &str| {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(path),
        )
        .unwrap()
    };
    let (example, settings, catalog, usage) = (
        read(".env.example"),
        read("core/web/src/lib/settings.ts"),
        read("core/web/src/locales/en.ts"),
        read("docs/USAGE.md"),
    );
    for (_, name, ..) in EVENTS {
        assert!(example.contains(name), ".env.example must name {name}");
        assert!(
            settings.contains(&format!("  {name}: \"notify.event.{name}\",")),
            "core/web/src/lib/settings.ts must label {name} in NOTIFY_EVENTS"
        );
        assert!(
            catalog.contains(&format!("\"notify.event.{name}\":")),
            "core/web/src/locales/en.ts must name {name}"
        );
        assert!(
            usage.contains(&format!("`{name}`")),
            "docs/USAGE.md must say what {name} sends"
        );
    }
}

#[test]
fn a_notice_is_bounded_before_it_is_queued() {
    let app = app_with(&[("NOTIFY_URL", NTFY)]);
    send(
        &app,
        Notice {
            title: format!("A title\nover two lines {}", "x".repeat(400)),
            body: "y".repeat(5000),
            container: Some("z".repeat(500)),
            ..notice(Kind::HumanRequest, None)
        },
    );
    let notice = queued(&app).remove(0);
    assert!(!notice.title.contains('\n'), "{}", notice.title);
    assert_eq!(notice.title.chars().count(), TITLE_LIMIT);
    assert!(notice.title.ends_with('\u{2026}'));
    assert_eq!(notice.body.chars().count(), BODY_LIMIT);
    assert_eq!(notice.container.unwrap().chars().count(), NAME_LIMIT);
    assert_eq!(cut("  short  ", 10), "short");
}

#[test]
fn only_the_chosen_kinds_go_and_only_with_an_address() {
    let app = app_with(&[]);
    send(&app, notice(Kind::HumanRequest, None));
    assert!(queued(&app).is_empty(), "no NOTIFY_URL, no notice");
    let app = app_with(&[("NOTIFY_URL", NTFY)]);
    send(&app, notice(Kind::JobFailed, None));
    send(&app, notice(Kind::HumanRequest, None));
    let kinds: Vec<Kind> = queued(&app).iter().map(|notice| notice.kind).collect();
    assert_eq!(kinds, [Kind::HumanRequest]);
}

#[tokio::test]
async fn a_request_for_a_person_sends_one_notice_however_often_it_is_waited_on() {
    let app = app_with(&[("NOTIFY_URL", NTFY)]);
    let ask = json!({"message": "Solve the CAPTCHA in Chrome", "wait_seconds": 0});
    let asked = crate::human::request(&app, "c1", ask.clone(), Caller::default())
        .await
        .unwrap();
    let id = asked.structured_content.unwrap()["request_id"]
        .as_str()
        .unwrap()
        .to_owned();
    // Asked again with the same message, the open request is joined; waited on by its id,
    // nothing is asked at all.
    crate::human::request(&app, "c1", ask, Caller::default())
        .await
        .unwrap();
    crate::human::request(
        &app,
        "c1",
        json!({"request_id": id, "wait_seconds": 0}),
        Caller::default(),
    )
    .await
    .unwrap();
    let sent = queued(&app);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(
        sent[0],
        Notice {
            kind: Kind::HumanRequest,
            title: "Unity needs a person".into(),
            body: "Solve the CAPTCHA in Chrome".into(),
            container: Some("Unity".into()),
            link: Some("#/desktop/c1".into()),
            key: Some(format!("human_request:{id}")),
        }
    );
    // Another request is another notice.
    crate::human::request(
        &app,
        "c1",
        json!({"message": "Sign in to Unity Hub", "wait_seconds": 0}),
        Caller::default(),
    )
    .await
    .unwrap();
    assert_eq!(queued(&app).len(), 1);
}

#[test]
fn a_failed_job_sends_its_kind_and_subject_but_a_cancelled_one_nothing() {
    let app = app_with(&[("NOTIFY_URL", NTFY), ("NOTIFY_EVENTS", "job_failed")]);
    let job = |id: &str| crate::model::Job {
        id: id.into(),
        kind: "exec".into(),
        resource: Some("container:c1".into()),
        name: None,
        status: "running".into(),
        created_at: 1,
        finished_at: None,
        log: String::new(),
        result: None,
        progress: None,
        stages: Vec::new(),
        script: None,
    };
    for (id, error) in [
        ("failed", "The script ended with 1; secret-in-the-output"),
        ("cancelled", crate::jobs::cancel::CANCELLED),
        ("abandoned", crate::archives::stall::CANCELLED),
    ] {
        app.db.put("job", id, &job(id)).unwrap();
        let _ = crate::jobs::finish(&app, id, job(id), Err(anyhow::anyhow!(error)));
    }
    let sent = queued(&app);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].kind, Kind::JobFailed);
    assert_eq!(sent[0].title, "Job failed: exec");
    assert_eq!(
        sent[0].body,
        "The exec job of Unity failed. Its log is on the Jobs page."
    );
    assert_eq!(sent[0].container.as_deref(), Some("Unity"));
    // The error, which a script's output can fill, stays in the job's log.
    assert!(!format!("{sent:?}").contains("secret-in-the-output"));
}

#[test]
fn a_key_is_sent_once() {
    let notifier = Notifier::default();
    assert!(notifier.enqueue(notice(Kind::HumanRequest, Some("human_request:a"))));
    assert!(!notifier.enqueue(notice(Kind::HumanRequest, Some("human_request:a"))));
    assert!(notifier.enqueue(notice(Kind::HumanRequest, Some("human_request:b"))));
    // A notice without a key, such as the test, is never taken for another.
    assert!(notifier.enqueue(notice(Kind::Test, None)));
    assert!(notifier.enqueue(notice(Kind::Test, None)));
}

#[test]
fn a_full_queue_drops_notices_and_says_so_when_it_begins_and_ends() {
    let (written, _guard) = crate::test_support::capture();
    let notifier = Notifier::default();
    let queued = (0..QUEUE + 5)
        .filter(|index| notifier.enqueue(notice(Kind::HumanRequest, Some(&format!("k{index}")))))
        .count();
    assert_eq!(queued, QUEUE);
    let log = written.text();
    assert_eq!(log.matches("new ones are dropped").count(), 1, "{log}");
    notifier
        .waiting
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .try_recv()
        .unwrap();
    assert!(notifier.enqueue(notice(Kind::HumanRequest, None)));
    assert!(written.text().contains("Notifications are queued again"));
    // The keys remembered are bounded too.
    for index in 0..SEEN * 2 {
        notifier.enqueue(notice(Kind::HumanRequest, Some(&format!("more{index}"))));
    }
    assert_eq!(notifier.seen.lock().unwrap().len(), SEEN);
}

#[test]
fn a_link_needs_the_dashboard_address_unless_it_is_whole() {
    let without = read(&[]).unwrap();
    assert_eq!(without.link(Some("#/desktop/c1")), None);
    assert_eq!(
        without.link(Some("https://github.com/co3moz/thoric/releases/tag/v1.0.0")),
        Some("https://github.com/co3moz/thoric/releases/tag/v1.0.0".into())
    );
    let with = read(&[("DASHBOARD_URL", "https://thoric.lan:8443/")]).unwrap();
    assert_eq!(
        with.link(Some("#/desktop/c1")),
        Some("https://thoric.lan:8443/#/desktop/c1".into())
    );
    assert_eq!(with.link(None), None);
}

#[tokio::test]
async fn the_test_needs_an_address() {
    let app = app_with(&[]);
    let Err(error) = test(State(app)).await else {
        panic!("a test without NOTIFY_URL was sent");
    };
    assert_eq!(error.0, StatusCode::CONFLICT);
    assert!(error.1.contains("NOTIFY_URL"), "{}", error.1);
}

/// A curl stand-in that records its arguments and its configuration under `dir`, and a line in
/// `dir/ended` once it has decided how to end: for an address that names `fails-here`, while
/// `dir/fail` is there, as curl does when it cannot connect, with what that file says, and
/// otherwise with `ok` and 200.
#[cfg(unix)]
fn stand_in(dir: &std::path::Path) -> String {
    std::fs::create_dir_all(dir).unwrap();
    let program = dir.join("curl");
    crate::test_support::write_script(
        &program,
        &format!(
            "#!/bin/sh\nd='{dir}'\necho \"$*\" >> \"$d/arguments\"\ncat > \"$d/config.$$\"\ncat \"$d/config.$$\" >> \"$d/configs\"\nif [ -f \"$d/fail\" ] && grep -q fails-here \"$d/config.$$\"; then cat \"$d/fail\" >&2; echo >> \"$d/ended\"; exit 7; fi\necho >> \"$d/ended\"\nprintf 'ok\\n200'\n",
            dir = dir.display()
        ),
    );
    program.display().to_string()
}

#[cfg(unix)]
#[tokio::test]
async fn the_test_reports_each_address_by_format_and_host_alone() {
    let dir = std::env::temp_dir().join(format!("thoric-notify-{}", uuid::Uuid::new_v4().simple()));
    let mut app = app_with(&[(
        "NOTIFY_URL",
        "https://ntfy.sh/thoric-alerts-of-somebody?token=tk_secretaccesstoken,https://hooks.slack.com/services/T0001/B0002/slacksecretpart",
    )]);
    Arc::get_mut(&mut app).unwrap().notifier = Notifier::with_program(&stand_in(&dir));
    let Ok(Json(report)) = test(State(app)).await else {
        panic!("the test was refused");
    };
    let report = serde_json::to_value(report).unwrap();
    assert_eq!(
        report,
        json!({"results": [
            {"format": "ntfy", "host": "ntfy.sh", "ok": true, "status": 200, "answer": "ok", "error": null},
            {"format": "slack", "host": "hooks.slack.com", "ok": true, "status": 200, "answer": "ok", "error": null},
        ]})
    );
    // The addresses and the token reached curl on stdin alone.
    let arguments = std::fs::read_to_string(dir.join("arguments")).unwrap();
    for secret in [
        "thoric-alerts-of-somebody",
        "tk_secretaccesstoken",
        "slacksecretpart",
    ] {
        assert!(!arguments.contains(secret), "{arguments}");
    }
    let configs = std::fs::read_to_string(dir.join("configs")).unwrap();
    assert!(configs.contains("url = \"https://ntfy.sh/\""), "{configs}");
    assert!(
        configs.contains("header = \"Authorization: Bearer tk_secretaccesstoken\""),
        "{configs}"
    );
    assert!(
        configs.contains("url = \"https://hooks.slack.com/services/T0001/B0002/slacksecretpart\""),
        "{configs}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_failing_address_is_logged_when_it_begins_and_ends_without_its_text() {
    let dir = std::env::temp_dir().join(format!("thoric-notify-{}", uuid::Uuid::new_v4().simple()));
    // Two addresses of one host, of which only the first fails, are logged apart.
    const FAILING: &str = "https://ntfy.sh/fails-here?token=tk_secretaccesstoken";
    let mut app = app_with(&[("NOTIFY_URL", &format!("{FAILING},https://ntfy.sh/works"))]);
    Arc::get_mut(&mut app).unwrap().notifier = Notifier::with_program(&stand_in(&dir));
    let (written, _guard) = crate::test_support::capture();
    let ended = || {
        std::fs::read_to_string(dir.join("ended"))
            .unwrap_or_default()
            .lines()
            .count()
    };
    // curl's own error names the address, which the log must not.
    std::fs::write(
        dir.join("fail"),
        format!("curl: (7) Failed to connect to {FAILING}"),
    )
    .unwrap();
    spawn(app.clone());
    for round in 1..=3 {
        if round == 3 {
            std::fs::remove_file(dir.join("fail")).unwrap();
        }
        send(&app, notice(Kind::HumanRequest, None));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while ended() < round * 2 {
            assert!(
                std::time::Instant::now() < deadline,
                "notice {round} was not sent"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !written.text().contains("works again") {
        assert!(std::time::Instant::now() < deadline, "{}", written.text());
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let log = written.text();
    assert_eq!(
        log.matches(
            "Notifications to address 1, ntfy at ntfy.sh: curl: (7) Failed to connect to [hidden]"
        )
        .count(),
        1,
        "{log}"
    );
    assert_eq!(
        log.matches("Notifications to address 1, ntfy at ntfy.sh works again")
            .count(),
        1,
        "{log}"
    );
    assert!(!log.contains("address 2"), "{log}");
    for secret in ["fails-here", "tk_secretaccesstoken"] {
        assert!(!log.contains(secret), "{log}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}
