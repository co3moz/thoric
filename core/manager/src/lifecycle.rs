//! A container's lifecycle policy, which the manager carries out by itself: a desktop nobody has
//! used for `idle_stop_minutes` is stopped, which gives its memory and GPU back, and a container
//! whose `expires_at` has passed is stopped or deleted, as `on_expiry` says, so a throwaway
//! desktop an agent made cleans itself up.
//!
//! The policy is one object on the container's settings, `lifecycle`, and the manager's alone: a
//! change applies in place and never restarts the desktop. A record saved before it existed has
//! none, which reads as never, so an upgrade stops and deletes nothing. A new container gets
//! `IDLE_STOP_DEFAULT` unless its request says otherwise, copied into its settings, so a later
//! change of the default leaves the existing containers as they are.
//!
//! What counts as use, [`Activity`] keeps in memory: a job on the container, from the moment it
//! is queued until it ends, a request to one of its routes other than a read and a download of its
//! files, its VNC, sound and microphone connections, an MCP inspector session with one of its
//! desktop servers, an MCP tool call or a transfer link that names it, and a request to one of its
//! proxied ports, also a WebSocket it keeps open, each until its answer has been sent. Its desktop
//! starting counts too, and so does the manager starting, so a restart never stops a desktop at
//! once. A desktop whose processes use more than [`BUSY_CORES`] of a CPU core
//! over the last [`BUSY_WINDOW`] counts as used while they do, so a build or a training run
//! started by hand inside it is not stopped. The idle time runs from the last use. Reads, such as
//! the dashboard's previews, its usage charts and the container list, are not use: a page left
//! open repeats them and would keep every desktop running.
//!
//! [`reaper`] carries the policies out every 30 seconds, through the stop and delete code the
//! routes use, as jobs of their own kinds, and each that is done sends a `container_lifecycle`
//! notice when `NOTIFY_EVENTS` names it.
mod activity;
mod reaper;

pub(crate) use activity::{Activity, InUse};
pub(crate) use reaper::spawn;

use crate::{App, model::Container};
use anyhow::{Context, Result, bail};
use axum::{
    extract::{Request, State},
    http::Method,
    middleware::Next,
    response::Response,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{sync::Arc, time::Duration};

/// The longest idle stop, a week, in minutes.
pub(crate) const MAX_IDLE_MINUTES: u32 = 10_080;
/// How far ahead an expiry may be: a year, in minutes.
pub(crate) const MAX_EXPIRY_MINUTES: u64 = 366 * 24 * 60;
/// A desktop whose processes use more of a CPU core than this on average over [`BUSY_WINDOW`]
/// counts as used. An idle desktop, its VNC server and a browser on a still page use a few
/// hundredths of one.
pub(crate) const BUSY_CORES: f64 = 0.25;
/// The time CPU use is averaged over.
pub(crate) const BUSY_WINDOW: Duration = Duration::from_secs(120);

/// When the manager stops or deletes a container by itself. The default, which every record saved
/// before the policy existed reads as, never does.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lifecycle {
    /// Minutes the desktop may go unused before the manager stops it; `0` never stops it.
    #[serde(default)]
    pub idle_stop_minutes: u32,
    /// When the container expires, in Unix seconds; `None` never.
    #[serde(default)]
    pub expires_at: Option<u64>,
    /// What happens once it has expired.
    #[serde(default)]
    pub on_expiry: OnExpiry,
    /// With [`OnExpiry::Delete`], the volumes created for the container go too, as a delete's
    /// `delete_volumes` asks; volumes chosen by name and those another container mounts stay.
    #[serde(default)]
    pub delete_volumes: bool,
}

/// What an expired container becomes.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum OnExpiry {
    /// Stopped, and its expiry cleared, so starting it again keeps it running.
    #[default]
    Stop,
    /// Deleted, as `DELETE /api/containers/{id}` deletes it.
    Delete,
}

/// A `lifecycle` as a creation or a settings change gives it. A field it leaves out keeps the
/// value there is, which for a new container is never, or for its idle stop the installation's
/// `IDLE_STOP_DEFAULT`.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "Lifecycle")]
pub struct LifecycleRequest {
    /// Minutes the desktop may go unused before the manager stops it, up to 10080, a week; 0 never
    /// stops it.
    #[schemars(range(max = 10080))]
    pub idle_stop_minutes: Option<u32>,
    /// When the container expires, in Unix seconds, in the future and at most a year ahead; null
    /// for never.
    #[serde(default, deserialize_with = "present")]
    pub expires_at: Option<Option<u64>>,
    /// Expire this many minutes from now, up to a year; instead of `expires_at`.
    #[schemars(range(min = 1, max = 527040))]
    pub expires_in_minutes: Option<u64>,
    /// What happens once it has expired: `stop`, the default, or `delete`.
    pub on_expiry: Option<OnExpiry>,
    /// With `on_expiry` delete, also delete the volumes created for the container, as a delete's
    /// `delete_volumes` does.
    pub delete_volumes: Option<bool>,
}

/// A field that may be `null`, which differs from leaving it out.
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

impl Lifecycle {
    /// Whether the policy never stops or deletes the container.
    pub(crate) fn is_never(&self) -> bool {
        self.idle_stop_minutes == 0 && self.expires_at.is_none()
    }

    /// What every saved policy keeps to; a time in the past is one, since an expiry passes.
    pub(crate) fn validate(&self) -> Result<()> {
        if self.idle_stop_minutes > MAX_IDLE_MINUTES {
            bail!("idle_stop_minutes must be at most {MAX_IDLE_MINUTES}, a week");
        }
        if self.delete_volumes && self.on_expiry != OnExpiry::Delete {
            bail!("delete_volumes needs on_expiry delete");
        }
        Ok(())
    }

    /// This policy with what `request` changes at `now`, in Unix seconds. An expiry the request
    /// sets must lie ahead; one it leaves as it is may have passed already.
    pub(crate) fn requested(&self, request: &Value, now: u64) -> Result<Self> {
        if !request.is_object() {
            bail!(
                "lifecycle must be an object; idle_stop_minutes 0 and expires_at null clear the policy"
            );
        }
        let request: LifecycleRequest =
            serde_json::from_value(request.clone()).context("Invalid lifecycle")?;
        let mut next = self.clone();
        if let Some(minutes) = request.idle_stop_minutes {
            next.idle_stop_minutes = minutes;
        }
        match (request.expires_at, request.expires_in_minutes) {
            (Some(_), Some(_)) => bail!("Give expires_at or expires_in_minutes, not both"),
            (Some(at), None) => next.expires_at = at,
            (None, Some(minutes)) => {
                if !(1..=MAX_EXPIRY_MINUTES).contains(&minutes) {
                    bail!("expires_in_minutes must be from 1 to {MAX_EXPIRY_MINUTES}, a year");
                }
                next.expires_at = Some(now + minutes * 60);
            }
            (None, None) => {}
        }
        if next.expires_at != self.expires_at
            && let Some(at) = next.expires_at
            && (at <= now || at > now + MAX_EXPIRY_MINUTES * 60)
        {
            bail!("expires_at must be in the future and at most a year ahead");
        }
        if let Some(on_expiry) = request.on_expiry {
            next.on_expiry = on_expiry;
        }
        match request.delete_volumes {
            Some(delete) => next.delete_volumes = delete,
            // A stop keeps every volume, so a policy changed to stop no longer deletes any.
            None if next.on_expiry == OnExpiry::Stop => next.delete_volumes = false,
            None => {}
        }
        next.validate()?;
        Ok(next)
    }
}

/// `IDLE_STOP_DEFAULT`: the idle stop a new container gets when its request names none, in
/// minutes; `0`, the default, for none.
pub(crate) fn idle_stop_default(value: Option<&str>) -> Result<u32> {
    let refused = || {
        format!(
            "IDLE_STOP_DEFAULT must be a whole number of minutes from 0, never, to {MAX_IDLE_MINUTES}, a week"
        )
    };
    let minutes = match value {
        None => 0,
        Some(text) => text.parse::<u32>().with_context(refused)?,
    };
    if minutes > MAX_IDLE_MINUTES {
        bail!(refused());
    }
    Ok(minutes)
}

/// The `lifecycle` a creation request gets: the one it asks for, with the installation's idle stop
/// when it names none.
pub(crate) fn for_creation(
    request: Option<&Value>,
    idle_default: u32,
    now: u64,
) -> Result<Lifecycle> {
    let defaults = Lifecycle {
        idle_stop_minutes: idle_default,
        ..Lifecycle::default()
    };
    match request {
        None => Ok(defaults),
        Some(request) => defaults.requested(request, now),
    }
}

/// A container a restore creates keeps its policy, but not an expiry that passed while it was in
/// the backup: the container was asked for, so it stays rather than being stopped or deleted at
/// once. Says whether one was cleared.
pub(crate) fn restored(lifecycle: &mut Lifecycle, now: u64) -> bool {
    let passed = lifecycle.expires_at.is_some_and(|at| at <= now);
    if passed {
        lifecycle.expires_at = None;
    }
    passed
}

/// What the policy asks for now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Due {
    /// The running desktop went unused for this many minutes.
    IdleStop(u32),
    /// The instance of a scale set went unused for this many minutes, its set's idle delete.
    IdleDelete(u32),
    /// The container expired.
    Expiry(OnExpiry),
}

/// What `lifecycle` asks for at `now`, in Unix seconds, of a container that is `running` and has
/// gone unused for `idle`, `None` while it is in use. An expiry comes first, whatever the state:
/// one that stops also clears itself on a container that is stopped already.
pub(crate) fn due(
    lifecycle: &Lifecycle,
    running: bool,
    idle: Option<Duration>,
    now: u64,
) -> Option<Due> {
    if lifecycle.expires_at.is_some_and(|at| at <= now) {
        return Some(Due::Expiry(lifecycle.on_expiry));
    }
    let minutes = lifecycle.idle_stop_minutes;
    let limit = Duration::from_secs(u64::from(minutes) * 60);
    (running && minutes > 0 && idle.is_some_and(|idle| idle >= limit))
        .then_some(Due::IdleStop(minutes))
}

/// What is due for the container `record` at `now`, its own policy and its scale set's idle
/// delete together: an expiry first, then the idle delete, which makes a stop pointless, then the
/// idle stop.
pub(crate) fn due_for(
    app: &App,
    record: &Container,
    sets: &[crate::scale::ScaleSet],
    running: bool,
    now: u64,
) -> Option<Due> {
    let idle = idle(app, &record.id, running);
    match due(&record.spec.lifecycle, running, idle, now) {
        Some(Due::Expiry(on_expiry)) => Some(Due::Expiry(on_expiry)),
        stop => crate::scale::instance_of(sets, &record.id)
            .and_then(|(set, number)| crate::scale::idle_delete_due(set, number, idle))
            .map(Due::IdleDelete)
            .or(stop),
    }
}

/// Whether the container's processes keep a CPU busy, which counts as use.
pub(crate) fn busy(app: &App, id: &str) -> bool {
    app.usage
        .cpu_average(id, BUSY_WINDOW)
        .is_some_and(|cores| cores > BUSY_CORES)
}

/// How long the container has gone unused, `None` while it is in use.
pub(crate) fn idle(app: &App, id: &str, running: bool) -> Option<Duration> {
    if running && busy(app, id) {
        return None;
    }
    app.lifecycle.idle_for(id)
}

/// What a container's entry in the lists says of its use: whether something uses its running
/// desktop now, and when its idle stop comes if nothing uses it from now on, in Unix seconds. That
/// is a whole idle time ahead while it is in use, also by the MCP call that asks, and less once
/// it goes unused; a time that has passed means the stop is on its way. Neither applies to a
/// stopped desktop, and the time not without an idle stop.
pub(crate) fn use_of(app: &App, record: &Container, running: bool) -> (bool, Option<u64>) {
    if !running {
        return (false, None);
    }
    let idle = idle(app, &record.id, running);
    let minutes = record.spec.lifecycle.idle_stop_minutes;
    let stops = (minutes > 0).then(|| {
        let unused = idle.unwrap_or_default().as_secs();
        crate::model::now() + (u64::from(minutes) * 60).saturating_sub(unused)
    });
    (idle.is_none(), stops)
}

/// The container a request to its routes names, `/api/containers/{id}/...`.
fn container_of(path: &str) -> Option<&str> {
    let id = path.strip_prefix("/api/containers/")?.split('/').next()?;
    (!id.is_empty() && id.len() <= 64).then_some(id)
}

/// A request to a container's routes that changes or uses it counts as its use until its answer
/// has been sent, also a download that streams on: any request but a read, and a file or folder
/// download, which a person asks for and nothing repeats. Other reads, GET and HEAD, are left out:
/// the dashboard repeats them on a timer, such as its previews' screenshots, and a page left open
/// would keep every desktop running.
pub(crate) async fn requests(
    State(app): State<Arc<App>>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    let read = matches!(*request.method(), Method::GET | Method::HEAD);
    let using = (!read || path.ends_with("/files/download"))
        .then(|| container_of(path))
        .flatten()
        .map(|id| app.lifecycle.open(id));
    let response = next.run(request).await;
    match using {
        Some(using) => lasting(response, using),
        None => response,
    }
}

/// `response` with a body that keeps its container in use until it has been sent, or until the
/// client goes away.
pub(crate) fn lasting(response: Response, using: InUse) -> Response {
    response.map(|body| {
        axum::body::Body::new(Lasting {
            body,
            _using: using,
        })
    })
}

/// A response body and the use it keeps; it reads as the body it carries.
struct Lasting {
    body: axum::body::Body,
    _using: InUse,
}

impl hyper::body::Body for Lasting {
    type Data = axum::body::Bytes;
    type Error = axum::Error;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<std::result::Result<hyper::body::Frame<Self::Data>, Self::Error>>>
    {
        hyper::body::Body::poll_frame(std::pin::Pin::new(&mut self.get_mut().body), cx)
    }
    fn is_end_stream(&self) -> bool {
        hyper::body::Body::is_end_stream(&self.body)
    }
    fn size_hint(&self) -> hyper::body::SizeHint {
        hyper::body::Body::size_hint(&self.body)
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
