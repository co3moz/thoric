//! Notices that reach a person away from the dashboard, such as an agent's request for a person
//! while nobody has the dashboard open. `NOTIFY_URL` names where they go, one or several
//! addresses of ntfy, Discord, Slack, Telegram or any JSON webhook ([`target`]), and
//! `NOTIFY_EVENTS` which [`Kind`]s are sent.
//!
//! Other code calls [`send`], which never waits and never fails: a notice goes into a bounded
//! queue, and one task sends each to every address with a single POST through `curl`
//! ([`deliver`]), which reads the address and its headers on stdin, so no credential is on a
//! command line. A notice that finds the queue full is dropped. A notice with a [`Notice::key`]
//! is sent once, so an agent that waits on its request for a person again, or asks it again,
//! sends nothing more. A failing address is logged at warn when it begins failing or its reason
//! changes and at info once it works again, never with the address itself. `NOTIFY_URL` holds its
//! credentials, so it is a secret like `MCP_TOKEN`: the settings page shows only whether it is
//! set, and the dashboard's test reports each address by its format and host.
mod deliver;
pub(crate) mod notices;
mod payload;
mod target;

pub(crate) use target::{Format, Target};

use crate::{
    App,
    error::{Error, Result},
    model::now,
    reported::Reported,
};
use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::mpsc;

/// Notices that may wait to be sent; more are dropped until the sender catches up.
const QUEUE: usize = 64;
/// How many keys of sent notices are remembered, against sending one twice.
const SEEN: usize = 512;
/// The longest title and body a notice has, in characters; longer ones are cut.
pub(crate) const TITLE_LIMIT: usize = 120;
pub(crate) const BODY_LIMIT: usize = 1000;
const NAME_LIMIT: usize = 128;

/// What a notice is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    HumanRequest,
    JobFailed,
    UpdateAvailable,
    ContainerLifecycle,
    /// The Management page's test, which is sent whatever `NOTIFY_EVENTS` says.
    Test,
}

/// The kinds `NOTIFY_EVENTS` chooses from: each one's name there and in the JSON payload, whether
/// it is sent while that setting is empty, and its priority on ntfy, from 1 to 5. A new kind is
/// a variant of [`Kind`] and its line here.
const EVENTS: &[(Kind, &str, bool, u8)] = &[
    (Kind::HumanRequest, "human_request", true, 4),
    (Kind::JobFailed, "job_failed", false, 3),
    (Kind::UpdateAvailable, "update_available", false, 2),
    (Kind::ContainerLifecycle, "container_lifecycle", false, 2),
];

impl Kind {
    pub(crate) fn name(self) -> &'static str {
        EVENTS
            .iter()
            .find(|(kind, ..)| *kind == self)
            .map_or("test", |(_, name, ..)| name)
    }
    fn priority(self) -> u8 {
        EVENTS
            .iter()
            .find(|(kind, ..)| *kind == self)
            .map_or(3, |(.., priority)| *priority)
    }
}

/// What happened, for a person: a title, a body, the container it concerns and where to look.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Notice {
    pub kind: Kind,
    pub title: String,
    pub body: String,
    /// The name of the container it concerns.
    pub container: Option<String>,
    /// A dashboard page such as `#/desktop/<id>`, which becomes a link once `DASHBOARD_URL` says
    /// where the dashboard is, or a whole `https://` address.
    pub link: Option<String>,
    /// Sent once: a later notice with the same key is dropped.
    pub key: Option<String>,
}

impl Notice {
    /// The notice within the limits every target takes.
    fn bounded(mut self) -> Self {
        self.title = cut(&self.title.replace(['\r', '\n'], " "), TITLE_LIMIT);
        self.body = cut(&self.body, BODY_LIMIT);
        self.container = self.container.map(|name| cut(&name, NAME_LIMIT));
        self
    }
}

/// `text` without the spaces at its ends, at most `limit` characters long, ending in an ellipsis
/// when it was cut.
fn cut(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(limit - 1).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('\u{2026}');
    cut
}

/// `NOTIFY_URL`, `NOTIFY_EVENTS` and `DASHBOARD_URL`, as the manager started with them.
#[derive(Clone, Debug, Default)]
pub(crate) struct Settings {
    pub targets: Vec<Target>,
    pub events: Vec<Kind>,
    /// The address the dashboard is reached at, without a slash at the end.
    pub dashboard: Option<String>,
}

impl Settings {
    /// `NOTIFY_EVENTS` as the manager reads it.
    pub(crate) fn events_text(&self) -> String {
        if self.events.is_empty() {
            return "none".into();
        }
        let names: Vec<&str> = self.events.iter().map(|kind| kind.name()).collect();
        names.join(",")
    }
    /// Where a notice's `link` leads, or `None` for a dashboard page without `DASHBOARD_URL`.
    fn link(&self, link: Option<&str>) -> Option<String> {
        let link = link?;
        if link.starts_with("https://") || link.starts_with("http://") {
            return Some(link.to_owned());
        }
        Some(format!("{}/{link}", self.dashboard.as_deref()?))
    }
}

/// Read the settings, as [`crate::config::Config::parse`] gives them.
pub(crate) fn settings(var: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<Settings> {
    Ok(Settings {
        targets: target::targets(var("NOTIFY_URL").as_deref())?,
        events: events(var("NOTIFY_EVENTS").as_deref())?,
        dashboard: target::dashboard_url(var("DASHBOARD_URL").as_deref())?,
    })
}

/// `NOTIFY_EVENTS`: kinds separated by commas, empty for those on by default, or `none`.
fn events(value: Option<&str>) -> anyhow::Result<Vec<Kind>> {
    let value = value.unwrap_or_default().trim();
    if value.is_empty() {
        return Ok(EVENTS
            .iter()
            .filter(|(.., default, _)| *default)
            .map(|(kind, ..)| *kind)
            .collect());
    }
    if value == "none" {
        return Ok(Vec::new());
    }
    let mut kinds = Vec::new();
    for name in value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        let Some((kind, ..)) = EVENTS.iter().find(|(_, known, ..)| *known == name) else {
            let known: Vec<&str> = EVENTS.iter().map(|(_, name, ..)| *name).collect();
            anyhow::bail!(
                "NOTIFY_EVENTS takes {} separated by commas, or none; {name:.40} is none of them",
                known.join(", ")
            );
        };
        if !kinds.contains(kind) {
            kinds.push(*kind);
        }
    }
    Ok(kinds)
}

/// The queue between [`send`] and the task that sends.
pub(crate) struct Notifier {
    queue: mpsc::Sender<Notice>,
    waiting: Mutex<Option<mpsc::Receiver<Notice>>>,
    /// The keys of the notices sent lately, oldest first.
    seen: Mutex<VecDeque<String>>,
    /// Whether notices are being dropped, which is logged when it begins and ends.
    full: AtomicBool,
    /// Held while the Management page's test runs, so a second click waits for the first.
    testing: tokio::sync::Mutex<()>,
    /// The curl that sends, which tests replace.
    program: String,
}

impl Default for Notifier {
    fn default() -> Self {
        Self::with_program(deliver::PROGRAM)
    }
}

impl Notifier {
    pub(crate) fn with_program(program: &str) -> Self {
        let (queue, waiting) = mpsc::channel(QUEUE);
        Self {
            queue,
            waiting: Mutex::new(Some(waiting)),
            seen: Mutex::default(),
            full: AtomicBool::new(false),
            testing: tokio::sync::Mutex::new(()),
            program: program.into(),
        }
    }

    /// Queue `notice` unless one with its key was queued before; says whether it was queued.
    fn enqueue(&self, notice: Notice) -> bool {
        if let Some(key) = &notice.key {
            let mut seen = self.seen.lock().unwrap();
            if seen.contains(key) {
                return false;
            }
            if seen.len() >= SEEN {
                seen.pop_front();
            }
            seen.push_back(key.clone());
        }
        match self.queue.try_send(notice.bounded()) {
            Ok(()) => {
                if self.full.swap(false, Ordering::Relaxed) {
                    tracing::info!("Notifications are queued again");
                }
                true
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                if !self.full.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        "{QUEUE} notifications wait to be sent; new ones are dropped until they have gone"
                    );
                }
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }
}

#[cfg(test)]
impl Notifier {
    /// The notices queued and not sent yet, oldest first, for the tests of the code that sends.
    pub(crate) fn queued(&self) -> Vec<Notice> {
        let mut waiting = self.waiting.lock().unwrap();
        let waiting = waiting.as_mut().unwrap();
        std::iter::from_fn(|| waiting.try_recv().ok()).collect()
    }
}

/// Whether a notice of `kind` goes anywhere, for a caller whose notice costs work to make.
pub(crate) fn wants(app: &App, kind: Kind) -> bool {
    let settings = &app.config.notify;
    !settings.targets.is_empty() && settings.events.contains(&kind)
}

/// Send `notice` to every address of `NOTIFY_URL` if `NOTIFY_EVENTS` asks for its kind, without
/// waiting for it; nothing the sending meets reaches the caller.
pub(crate) fn send(app: &App, notice: Notice) {
    if wants(app, notice.kind) {
        app.notifier.enqueue(notice);
    }
}

/// What the log calls an address, by its place in `NOTIFY_URL`, since two addresses can share a
/// format and a host.
fn subject(place: usize, target: &Target) -> String {
    format!(
        "Notifications to address {}, {}",
        place + 1,
        target.describe()
    )
}

/// Send what [`send`] queues for as long as the manager runs, when `NOTIFY_URL` names anywhere.
pub(crate) fn spawn(app: Arc<App>) {
    let Some(mut waiting) = app.notifier.waiting.lock().unwrap().take() else {
        return;
    };
    if app.config.notify.targets.is_empty() {
        return;
    }
    tokio::spawn(async move {
        let mut reported = Reported::default();
        while let Some(notice) = waiting.recv().await {
            let deliveries = deliver_all(&app, &notice).await;
            let targets = app.config.notify.targets.iter().enumerate();
            for ((place, target), delivery) in targets.zip(deliveries) {
                let result = match delivery.error {
                    None => Ok(()),
                    Some(error) => Err(anyhow::anyhow!(error)),
                };
                reported.outcome(&subject(place, target), &result);
            }
        }
    });
}

/// Send `notice` to every address at once, and say how each answered, in their order.
async fn deliver_all(app: &App, notice: &Notice) -> Vec<deliver::Delivery> {
    let settings = &app.config.notify;
    let link = settings.link(notice.link.as_deref());
    let time = now();
    futures_util::future::join_all(settings.targets.iter().map(|target| {
        let request = payload::request(target, notice, link.as_deref(), time);
        async move { deliver::deliver(&app.notifier.program, target, &request).await }
    }))
    .await
}

/// How one address took the test notice.
#[derive(Serialize, schemars::JsonSchema)]
pub struct Tried {
    /// How the address is sent to: `ntfy`, `discord`, `slack`, `telegram` or `json`.
    format: Format,
    /// The host of the address; the rest of it, which can hold a credential, is never shown.
    host: String,
    ok: bool,
    /// The HTTP status the address answered with, when it answered.
    status: Option<u16>,
    /// The start of its answer, without any part of the address.
    answer: String,
    /// Why it did not take the notice.
    error: Option<String>,
}

/// The answer of `POST /api/settings/notify/test`, one result per address of `NOTIFY_URL`.
#[derive(Serialize, schemars::JsonSchema)]
pub struct TestReport {
    results: Vec<Tried>,
}

/// `POST /api/settings/notify/test`: send a test notice to every address now and say how each
/// answered.
pub async fn test(State(app): State<Arc<App>>) -> Result<Json<TestReport>> {
    if app.config.notify.targets.is_empty() {
        return Err(Error(
            StatusCode::CONFLICT,
            "NOTIFY_URL is not set; set it in .env, with thoric settings in the WSL package".into(),
        ));
    }
    let notice = notices::test().bounded();
    let _turn = app.notifier.testing.lock().await;
    let deliveries = deliver_all(&app, &notice).await;
    let results = app
        .config
        .notify
        .targets
        .iter()
        .zip(deliveries)
        .map(|(target, delivery)| Tried {
            format: target.format,
            host: target.host.clone(),
            ok: delivery.error.is_none(),
            status: delivery.status,
            answer: delivery.answer,
            error: delivery.error,
        })
        .collect();
    Ok(Json(TestReport { results }))
}

#[cfg(test)]
#[path = "notify_tests.rs"]
mod tests;
