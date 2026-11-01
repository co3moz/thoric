//! Docker's own events, the changes no job of this manager made: a desktop that exits or turns
//! unhealthy, a container an agent stopped with `docker`, an image pulled or a volume removed by
//! hand. One `GET /events` on the Engine socket for the manager's life, filtered by the daemon to
//! what the lists show; a burst of them becomes one `changed` event that names the lists to read
//! again. Whether they flow is told too, so a page polls those lists slowly only while they do.
use crate::{App, model::LABEL, usage::engine};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Notify;

/// What changes a container's row: its state, its health and its name.
const CONTAINER_ACTIONS: &[&str] = &[
    "create",
    "start",
    "die",
    "destroy",
    "rename",
    "pause",
    "unpause",
    "health_status",
];
/// What changes whether an image is there and what it is called.
const IMAGE_ACTIONS: &[&str] = &["pull", "tag", "untag", "delete", "import", "load"];
const VOLUME_ACTIONS: &[&str] = &["create", "destroy"];

/// What Docker reports that changes a list the dashboard shows: by object type, the list and the
/// actions. `exec_*`, `attach`, network and mount events, which every script and every start
/// send, never leave the daemon.
const WATCHED: [(&str, &str, &[&str]); 3] = [
    ("container", "containers", CONTAINER_ACTIONS),
    ("image", "images", IMAGE_ACTIONS),
    ("volume", "volumes", VOLUME_ACTIONS),
];

/// How long a burst, such as a container recreated, gathers before its lists are named once.
const SETTLE: Duration = Duration::from_millis(250);
/// Docker says nothing while nothing happens, so a quiet stream is not a lost one.
const IDLE: Duration = Duration::MAX;
/// The longest wait between two attempts to reach Docker's events.
const RETRY_MAX: Duration = Duration::from_secs(30);

/// Follow Docker's events where the Engine socket is reachable; elsewhere, such as with a
/// `tcp://` `DOCKER_HOST`, pages keep polling as before.
pub(crate) fn spawn(app: Arc<App>) {
    let Some(socket) = app.config.docker_socket.clone() else {
        return;
    };
    let changes = Arc::new(Changes::default());
    tokio::spawn(publish(app.clone(), changes.clone(), SETTLE));
    tokio::spawn(watch(app, socket, changes));
}

/// The Engine API path of the event stream, filtered to [`WATCHED`]. The daemon matches
/// `health_status` against every `health_status: <state>`.
pub(crate) fn path() -> String {
    let types: Vec<&str> = WATCHED.iter().map(|(kind, ..)| *kind).collect();
    let actions: BTreeSet<&str> = WATCHED
        .iter()
        .flat_map(|(.., actions)| actions.iter().copied())
        .collect();
    let filters = json!({"type": types, "event": actions}).to_string();
    format!("/events?filters={}", crate::pulls::query_value(&filters))
}

/// The list an event changes, or `None` for one the dashboard does not show, such as a container
/// this manager does not own.
pub(crate) fn changed_list(event: &Value) -> Option<&'static str> {
    let kind = event["Type"].as_str()?;
    // A health check names its new state after the action: `health_status: healthy`.
    let action = event["Action"].as_str()?.split(':').next()?.trim();
    let (_, list, actions) = WATCHED.iter().find(|(watched, ..)| *watched == kind)?;
    if !actions.contains(&action) {
        return None;
    }
    if kind == "container" && event["Actor"]["Attributes"][LABEL] != "true" {
        return None;
    }
    Some(*list)
}

/// Whether an event is one of this manager's desktops starting, whose proxy, when it has one,
/// has to be started and given its settings, and whose screen may need the resolution its
/// settings name.
pub(crate) fn starts_a_desktop(event: &Value) -> bool {
    event["Type"] == "container"
        && event["Action"] == "start"
        && event["Actor"]["Attributes"][LABEL] == "true"
}

/// How long to wait before the `attempt`th try: one second, doubling up to [`RETRY_MAX`].
fn retry_delay(attempt: u32) -> Duration {
    Duration::from_secs(1 << attempt.min(5)).min(RETRY_MAX)
}

/// The lists Docker changed since the last `changed` event, and a wake-up for its publisher.
#[derive(Default)]
struct Changes {
    lists: Mutex<BTreeSet<&'static str>>,
    added: Notify,
}

impl Changes {
    fn add(&self, list: &'static str) {
        self.lists.lock().unwrap().insert(list);
        self.added.notify_one();
    }
}

/// Name the changed lists once a burst settled: one event however many Docker sent.
async fn publish(app: Arc<App>, changes: Arc<Changes>, settle: Duration) {
    loop {
        tokio::select! {
            () = changes.added.notified() => {}
            () = app.events.closed() => return,
        }
        tokio::time::sleep(settle).await;
        let lists = std::mem::take(&mut *changes.lists.lock().unwrap());
        if !lists.is_empty() {
            app.events.changed(&lists);
        }
    }
}

/// Keep Docker's event stream open for the manager's life, opening it again with a growing delay
/// whenever it ends, as it does when Docker restarts.
async fn watch(app: Arc<App>, socket: PathBuf, changes: Arc<Changes>) {
    let mut attempt = 0;
    loop {
        let result = tokio::select! {
            result = follow(&app, &socket, &changes, &mut attempt) => result,
            () = app.events.closed() => return,
        };
        app.events.set_docker_events(false);
        if let Err(error) = result {
            tracing::debug!("Docker events: {error:#}");
        }
        let delay = retry_delay(attempt);
        attempt = attempt.saturating_add(1);
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            () = app.events.closed() => return,
        }
    }
}

/// Read one event stream until it ends.
async fn follow(app: &Arc<App>, socket: &Path, changes: &Changes, attempt: &mut u32) -> Result<()> {
    let io = crate::pulls::connect(socket)
        .await
        .with_context(|| format!("Docker Engine socket {}", socket.display()))?;
    engine::stream(
        io,
        &path(),
        IDLE,
        || {
            *attempt = 0;
            app.events.set_docker_events(true);
            // Desktops Docker started while no stream was open were not seen starting.
            crate::resolution::after_gap(app.clone());
        },
        |line| {
            let event = serde_json::from_slice::<Value>(line).ok();
            if let Some(event) = event.as_ref().filter(|event| starts_a_desktop(event)) {
                app.outbound.wake();
                if let Some(id) = event["Actor"]["Attributes"]["thoric.environment"].as_str() {
                    crate::resolution::after_start(app.clone(), id.to_owned());
                }
            }
            if let Some(list) = event.as_ref().and_then(changed_list) {
                changes.add(list);
            }
        },
    )
    .await
}

#[cfg(test)]
#[path = "docker_tests.rs"]
mod tests;
