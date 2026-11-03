//! Cancelling a job. A queued job of any kind is dropped before it runs, since nothing of it has
//! happened yet. A running job is stopped only when stopping it anywhere leaves what a failure
//! leaves: a script, a build, whose cleanup still runs, and an image's download or source build.
//! Work that changes a container, an image record or a volume, such as a reset, a snapshot or an
//! import, finishes once it started, as a half-made change would need cleaning up by hand.
//!
//! Dropping a job's work kills the Docker commands it runs, which are spawned with
//! `kill_on_drop`, but not a process a `docker exec` started inside a desktop: a job that starts
//! one says how to stop it with [`Cancels::on_stop`], as a script does.
use crate::{
    App,
    error::{Error, Result},
    model::Job,
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

/// The kinds of job that may be stopped while they run.
pub(crate) const STOPPABLE: [&str; 4] = ["exec", "build", "image-pull", "source-build"];
/// What a cancelled job fails with.
pub(crate) const CANCELLED: &str = "Cancelled by request";
/// How long a job's own way to stop may take, such as signalling a script in its desktop.
const STOP_LIMIT: Duration = Duration::from_secs(15);

type Stop = Pin<Box<dyn Future<Output = ()> + Send>>;

/// A job that has not ended yet, which a cancel reaches.
struct Pending {
    kind: String,
    running: bool,
    token: CancellationToken,
    stop: Option<Stop>,
}

/// The jobs of this manager that have not ended, by ID.
#[derive(Default)]
pub(crate) struct Cancels(Mutex<HashMap<String, Pending>>);

/// What a cancel found.
#[derive(Debug, PartialEq)]
pub(crate) enum Found {
    /// The job will end as cancelled.
    Cancelled,
    /// A running job of this kind that is not stopped halfway.
    Unstoppable(String),
    /// No job of this manager that has not ended.
    Ended,
}

impl Cancels {
    /// A queued job, which a cancel reaches through the returned token until [`Cancels::forget`].
    pub(crate) fn queued(&self, id: &str, kind: &str) -> CancellationToken {
        let token = CancellationToken::new();
        self.0.lock().unwrap().insert(
            id.to_owned(),
            Pending {
                kind: kind.to_owned(),
                running: false,
                token: token.clone(),
                stop: None,
            },
        );
        token
    }
    /// The job began to run.
    pub(crate) fn running(&self, id: &str) {
        if let Some(pending) = self.0.lock().unwrap().get_mut(id) {
            pending.running = true;
        }
    }
    /// How to stop what the job started outside the manager, run once its work was dropped.
    pub(crate) fn on_stop(&self, id: &str, stop: impl Future<Output = ()> + Send + 'static) {
        if let Some(pending) = self.0.lock().unwrap().get_mut(id) {
            pending.stop = Some(Box::pin(stop));
        }
    }
    /// The job ended; a cancel no longer reaches it.
    pub(crate) fn forget(&self, id: &str) {
        self.0.lock().unwrap().remove(id);
    }
    /// How many of the jobs that have not ended wait for their resource, and how many run.
    pub(crate) fn counts(&self) -> (usize, usize) {
        let jobs = self.0.lock().unwrap();
        let running = jobs.values().filter(|pending| pending.running).count();
        (jobs.len() - running, running)
    }
    /// Whether a cancel would reach the job `id` now, which its answer says as `cancellable`.
    pub(crate) fn cancellable(&self, id: &str) -> bool {
        self.0.lock().unwrap().get(id).is_some_and(|pending| {
            !pending.token.is_cancelled()
                && (!pending.running || STOPPABLE.contains(&pending.kind.as_str()))
        })
    }
    /// Cancel the job `id` when it is queued, or running and of a kind that may be stopped.
    pub(crate) fn cancel(&self, id: &str) -> Found {
        let jobs = self.0.lock().unwrap();
        match jobs.get(id) {
            None => Found::Ended,
            Some(pending) if pending.running && !STOPPABLE.contains(&pending.kind.as_str()) => {
                Found::Unstoppable(pending.kind.clone())
            }
            Some(pending) => {
                pending.token.cancel();
                Found::Cancelled
            }
        }
    }
    /// Run the job's own way to stop, once.
    pub(crate) async fn stop(&self, id: &str) {
        let stop = self
            .0
            .lock()
            .unwrap()
            .get_mut(id)
            .and_then(|pending| pending.stop.take());
        if let Some(stop) = stop
            && tokio::time::timeout(STOP_LIMIT, stop).await.is_err()
        {
            tracing::warn!("Job {id} did not stop in {} seconds", STOP_LIMIT.as_secs());
        }
    }
}

/// `POST /api/jobs/{id}/cancel`: end a queued job before it runs, or stop a running script,
/// build or image download. The job ends as failed with [`CANCELLED`] shortly after.
pub async fn cancel(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<Json<Value>> {
    let job: Job = app.db.get("job", &id)?;
    match app.cancels.cancel(&id) {
        Found::Cancelled => Ok(Json(json!({"job_id": id, "cancelled": true}))),
        Found::Unstoppable(kind) => Err(Error(
            StatusCode::CONFLICT,
            format!(
                "A running {kind} job is not stopped halfway, since it changes what it works on; wait for it to end"
            ),
        )),
        Found::Ended if job.finished_at.is_some() => Err(Error(
            StatusCode::CONFLICT,
            format!("The {} job has already ended", job.kind),
        )),
        // An update or a settings change the manager resumed after a restart.
        Found::Ended => Err(Error(
            StatusCode::CONFLICT,
            format!("The {} job cannot be cancelled", job.kind),
        )),
    }
}

#[cfg(test)]
#[path = "cancel_tests.rs"]
mod tests;
