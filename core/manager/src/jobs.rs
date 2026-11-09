use crate::{
    App,
    error::Result,
    model::{Job, now},
};
use axum::Json;
use futures_util::FutureExt;
use serde_json::{Value, json};
use std::{
    future::Future,
    sync::{Arc, Weak},
};

pub(crate) mod cancel;
pub(crate) mod history;
pub(crate) mod read;
pub(crate) mod retry;

/// A job's result for a caller that waits for it: the value it returned, or its error message.
pub(crate) type Outcome = tokio::sync::oneshot::Receiver<std::result::Result<Value, String>>;

/// A resource's lock. Work that changes or replaces the resource, such as a container's reset,
/// its settings or its delete, takes it alone with [`ResourceLock::lock`]; work that only uses
/// it, such as a script, desktop input or a file in a running desktop, shares it with
/// [`ResourceLock::share_owned`] with other such work, and waits only for the work that takes it
/// alone. Waiting is fair: work that takes it alone is not starved by a stream of sharers.
#[derive(Clone, Default)]
pub struct ResourceLock(Arc<tokio::sync::RwLock<()>>);
/// The resource held alone until it is dropped.
pub(crate) type Exclusive = tokio::sync::OwnedRwLockWriteGuard<()>;
/// The resource shared until it is dropped.
pub(crate) type Shared = tokio::sync::OwnedRwLockReadGuard<()>;

impl ResourceLock {
    pub async fn lock(&self) -> tokio::sync::RwLockWriteGuard<'_, ()> {
        self.0.write().await
    }
    pub async fn lock_owned(self) -> Exclusive {
        self.0.write_owned().await
    }
    pub fn try_lock(
        &self,
    ) -> std::result::Result<tokio::sync::RwLockWriteGuard<'_, ()>, tokio::sync::TryLockError> {
        self.0.try_write()
    }
    pub fn try_lock_owned(self) -> std::result::Result<Exclusive, tokio::sync::TryLockError> {
        self.0.try_write_owned()
    }
    pub async fn share_owned(self) -> Shared {
        self.0.read_owned().await
    }
}

/// What a job holds of its resource while it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hold {
    /// The resource alone, as work that changes or replaces it does.
    Exclusive,
    /// The resource beside other work that only uses it, such as a script in a desktop.
    Shared,
    /// Nothing, as a job that only reads a resource, such as an export, runs alongside.
    Nothing,
}

/// What a running job holds of its resource.
struct Held {
    shared: Option<Shared>,
    alone: Option<Exclusive>,
}

tokio::task_local! {
    /// The running job's hold, which [`share_from_here`] reaches from inside its work.
    static HELD: Arc<std::sync::Mutex<Held>>;
}

/// Share the resource the running job holds alone for the rest of the job, as work that only
/// uses it does: what remains, such as the wait for a desktop the job has just started, goes on
/// beside input, files and scripts, while work that changes the resource still waits for the job.
/// Nothing else can take the resource alone in between. Outside a job, or in one that does not
/// hold its resource alone, nothing changes.
pub(crate) fn share_from_here() {
    let _ = HELD.try_with(|held| {
        let mut held = held.lock().unwrap();
        if let Some(alone) = held.alone.take() {
            held.shared = Some(alone.downgrade());
        }
    });
}

impl App {
    pub fn resource_lock(&self, resource: &str) -> ResourceLock {
        if resource == "global" {
            return self.operations.clone();
        }
        let mut locks = self.resource_locks.lock().unwrap();
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(resource).and_then(Weak::upgrade) {
            return ResourceLock(lock);
        }
        let lock = ResourceLock::default();
        locks.insert(resource.into(), Arc::downgrade(&lock.0));
        lock
    }
}
pub fn launch<F, Fut>(app: Arc<App>, kind: &str, resource: String, work: F) -> Result<Json<Value>>
where
    F: FnOnce(Arc<App>, String) -> Fut + Send + 'static,
    Fut: Future<Output = anyhow::Result<Value>> + Send + 'static,
{
    launch_named(app, kind, resource, None, work)
}

/// [`launch`] for a job that makes a container or an image called `name`, which the job carries
/// so the lists can show what is on its way before its record exists.
pub fn launch_named<F, Fut>(
    app: Arc<App>,
    kind: &str,
    resource: String,
    name: Option<String>,
    work: F,
) -> Result<Json<Value>>
where
    F: FnOnce(Arc<App>, String) -> Fut + Send + 'static,
    Fut: Future<Output = anyhow::Result<Value>> + Send + 'static,
{
    let (id, _) = start_named(app, kind, resource, name, true, work)?;
    Ok(Json(json!({"job_id":id})))
}

/// [`launch`] for a job that only uses its resource, such as a script in a running desktop: it
/// shares the resource's lock, so input, files and other such work go on while it runs, and only
/// work that changes or replaces the resource waits for it.
pub fn launch_shared<F, Fut>(
    app: Arc<App>,
    kind: &str,
    resource: String,
    work: F,
) -> Result<Json<Value>>
where
    F: FnOnce(Arc<App>, String) -> Fut + Send + 'static,
    Fut: Future<Output = anyhow::Result<Value>> + Send + 'static,
{
    let (id, _) = queue(app, kind, resource, None, Hold::Shared, None, None, work)?;
    Ok(Json(json!({"job_id":id})))
}

/// [`launch_named`] for a job that can be started again once it fails or is interrupted:
/// `request` is what its route was asked, kept beside the job until it completes.
pub(crate) fn launch_retryable<F, Fut>(
    app: Arc<App>,
    kind: &str,
    resource: String,
    name: Option<String>,
    request: retry::Request,
    work: F,
) -> Result<Json<Value>>
where
    F: FnOnce(Arc<App>, String) -> Fut + Send + 'static,
    Fut: Future<Output = anyhow::Result<Value>> + Send + 'static,
{
    let (id, _) = queue(
        app,
        kind,
        resource,
        name,
        Hold::Exclusive,
        Some(request),
        None,
        work,
    )?;
    Ok(Json(json!({"job_id":id})))
}

/// Queue a job and return its ID with its eventual outcome. An `exclusive` job first takes its
/// resource's lock; one that only reads the resource, such as an image export, runs alongside.
pub(crate) fn start<F, Fut>(
    app: Arc<App>,
    kind: &str,
    resource: String,
    exclusive: bool,
    work: F,
) -> Result<(String, Outcome)>
where
    F: FnOnce(Arc<App>, String) -> Fut + Send + 'static,
    Fut: Future<Output = anyhow::Result<Value>> + Send + 'static,
{
    start_named(app, kind, resource, None, exclusive, work)
}

/// [`start`] for a job that makes something called `name`, as [`launch_named`] describes.
pub(crate) fn start_named<F, Fut>(
    app: Arc<App>,
    kind: &str,
    resource: String,
    name: Option<String>,
    exclusive: bool,
    work: F,
) -> Result<(String, Outcome)>
where
    F: FnOnce(Arc<App>, String) -> Fut + Send + 'static,
    Fut: Future<Output = anyhow::Result<Value>> + Send + 'static,
{
    let hold = if exclusive {
        Hold::Exclusive
    } else {
        Hold::Nothing
    };
    queue(app, kind, resource, name, hold, None, None, work)
}

/// [`start`] for work that holds its resource alone already, as the lifecycle's stop of a desktop
/// it found idle does: the job runs on with that hold, so nothing takes the resource between the
/// check that decided on the work and the work.
pub(crate) fn start_holding<F, Fut>(
    app: Arc<App>,
    kind: &str,
    resource: String,
    held: Exclusive,
    work: F,
) -> Result<(String, Outcome)>
where
    F: FnOnce(Arc<App>, String) -> Fut + Send + 'static,
    Fut: Future<Output = anyhow::Result<Value>> + Send + 'static,
{
    queue(
        app,
        kind,
        resource,
        None,
        Hold::Exclusive,
        None,
        Some(held),
        work,
    )
}

/// [`start_named`] that keeps the `request` of a job that can be retried before the job can end,
/// and runs with the hold `given` when its caller took it already.
#[allow(clippy::too_many_arguments)] // Each caller fixes a different few of them.
fn queue<F, Fut>(
    app: Arc<App>,
    kind: &str,
    resource: String,
    name: Option<String>,
    hold: Hold,
    request: Option<retry::Request>,
    given: Option<Exclusive>,
    work: F,
) -> Result<(String, Outcome)>
where
    F: FnOnce(Arc<App>, String) -> Fut + Send + 'static,
    Fut: Future<Output = anyhow::Result<Value>> + Send + 'static,
{
    let id = uuid::Uuid::new_v4().simple().to_string();
    let job = Job {
        id: id.clone(),
        kind: kind.into(),
        resource: Some(resource.clone()),
        name,
        status: "queued".into(),
        created_at: now(),
        finished_at: None,
        log: String::new(),
        result: None,
        progress: None,
        stages: Vec::new(),
        script: None,
    };
    app.db.put("job", &id, &job)?;
    if let Some(request) = &request {
        app.db.put(retry::REQUESTS, &id, request)?;
    }
    app.events.job(&job, None);
    let cancelled = app.cancels.queued(&id, kind);
    let lock = (hold != Hold::Nothing).then(|| app.resource_lock(&resource));
    let (done, outcome) = tokio::sync::oneshot::channel();
    let task_id = id.clone();
    // A job on a container uses it from its queueing to its end (see `lifecycle`).
    let using = resource
        .strip_prefix("container:")
        .map(|container| app.lifecycle.open(container));
    tokio::spawn(async move {
        let _using = using;
        let held = async {
            let shared = match &lock {
                Some(lock) if hold == Hold::Shared => Some(lock.clone().share_owned().await),
                _ => None,
            };
            let alone = match (given, lock) {
                (Some(given), _) => Some(given),
                (None, Some(lock)) if hold == Hold::Exclusive => Some(lock.lock_owned().await),
                _ => None,
            };
            Held { shared, alone }
        };
        // A job cancelled while it waits for its resource never runs.
        let held = tokio::select! {
            biased;
            () = cancelled.cancelled() => None,
            held = held => Some(held),
        };
        let mut job: Job = match app.db.get("job", &task_id) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!("{e}");
                app.cancels.forget(&task_id);
                return;
            }
        };
        let Some(held) = held else {
            app.cancels.forget(&task_id);
            let _ = done.send(finish(
                &app,
                &task_id,
                job,
                Err(anyhow::anyhow!(cancel::CANCELLED)),
            ));
            return;
        };
        job.status = "running".into();
        if let Err(e) = app.db.put("job", &task_id, &job) {
            tracing::error!("{e}");
            app.cancels.forget(&task_id);
            return;
        }
        app.cancels.running(&task_id);
        app.events.job(&job, None);
        // Held until the job has ended, while the work may share what it held alone.
        let held = Arc::new(std::sync::Mutex::new(held));
        let work = HELD.scope(held.clone(), async {
            work(app.clone(), task_id.clone()).await
        });
        let work = std::panic::AssertUnwindSafe(work).catch_unwind();
        // A cancel drops the work, which kills the commands it runs, and then stops what it
        // started elsewhere.
        let result = tokio::select! {
            result = work => result.unwrap_or_else(|_| Err(anyhow::anyhow!("Job ended unexpectedly"))),
            () = cancelled.cancelled() => {
                app.cancels.stop(&task_id).await;
                Err(anyhow::anyhow!(cancel::CANCELLED))
            }
        };
        app.cancels.forget(&task_id);
        // Includes failures during context preparation and panics; cleanup cannot depend on the script succeeding.
        if job.kind == "build" {
            // A failed build's last stage stays the one it failed in.
            if result.is_ok() {
                let _ = app.begin_stage(&task_id, "cleanup");
            }
            if let Err(e) = app.cleanup_build(&task_id).await {
                let _ = app.append_log(&task_id, &format!("Build cleanup: {e:#}"));
            }
        }
        // Nobody may be waiting; most callers return the job ID at once.
        let _ = done.send(finish(&app, &task_id, job, result));
    });
    Ok((id, outcome))
}

/// A job's failure that carries a result of its own, such as a script's exit code and the end of
/// its output, which the failed job keeps as its `result` beside its error.
#[derive(Debug)]
pub(crate) struct Failed {
    message: String,
    result: Value,
}
impl Failed {
    pub(crate) fn new(message: impl Into<String>, result: Value) -> Self {
        Self {
            message: message.into(),
            result,
        }
    }
}
impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Failed {}

/// Record how a job ended. `job` stands in for the saved record when that cannot be read.
pub(crate) fn finish(
    app: &App,
    id: &str,
    job: Job,
    result: anyhow::Result<Value>,
) -> std::result::Result<Value, String> {
    let mut job = app.db.get("job", id).unwrap_or(job);
    let finished = now();
    job.finished_at = Some(finished);
    job.finish_stages(finished);
    job.progress = None;
    let reported = match result {
        Ok(value) => {
            job.status = "completed".into();
            job.result = Some(value.clone());
            // Only a job that failed or was interrupted is started again.
            if let Err(e) = app.db.remove(retry::REQUESTS, id) {
                tracing::debug!("Job request: {e:#}");
            }
            Ok(value)
        }
        Err(e) => {
            job.status = "failed".into();
            job.result = e
                .downcast_ref::<Failed>()
                .map(|failed| failed.result.clone());
            Err(format!("{e:#}"))
        }
    };
    // A failed job's error is in its log by the time anyone reads that it failed.
    let saved = match &reported {
        Ok(_) => app.db.put("job", id, &job),
        Err(error) => app
            .db
            .put_job_logged(&job, &format!("\nERROR: {error}\n"), false),
    };
    if let Err(e) = saved {
        tracing::error!("Could not save job result: {e}");
    }
    app.events
        .job(&job, reported.as_ref().err().map(String::as_str));
    app.metrics.job_ended(&job.kind, &job.status);
    // A job someone cancelled, or a download whose browser went away, did not fail.
    if reported.as_ref().is_err_and(|error| {
        error != cancel::CANCELLED && error != crate::archives::stall::CANCELLED
    }) && crate::notify::wants(app, crate::notify::Kind::JobFailed)
    {
        crate::notify::send(app, crate::notify::notices::job_failed(app, &job));
    }
    history::ended(app, &job);
    reported
}

impl App {
    /// Add a line to the job's log; only the new text is written.
    pub fn append_log(&self, id: &str, line: &str) -> anyhow::Result<()> {
        self.db.append_job_log(id, &format!("{line}\n"))
    }
    /// Replace the job's measured progress; `None` once the measured step is over.
    pub fn set_progress(&self, id: &str, progress: Option<Value>) -> anyhow::Result<()> {
        let mut job: Job = self.db.get("job", id)?;
        job.progress = progress;
        self.db.put("job", id, &job)?;
        self.events.progress(id, job.progress.as_ref());
        Ok(())
    }
}

/// How long a request waits for a resource a job holds before answering that it is busy.
pub(crate) const HOLD_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

impl App {
    /// Hold a resource alone for the length of a request. A job such as a snapshot, an upload
    /// or an export can hold a container or a volume for an hour; a request waits a few seconds
    /// for it and then answers 409, instead of hanging its client for the job's length.
    pub(crate) async fn hold(&self, resource: &str) -> Result<Exclusive> {
        self.hold_for(resource, HOLD_WAIT).await
    }
    /// Share a resource for the length of a request that only uses it, such as a file read or
    /// an application's launch in a running desktop, beside a script that runs there. It waits
    /// as [`App::hold`] does for work that holds the resource alone.
    pub(crate) async fn share(&self, resource: &str) -> Result<Shared> {
        self.share_for(resource, HOLD_WAIT).await
    }
    async fn hold_for(&self, resource: &str, wait: std::time::Duration) -> Result<Exclusive> {
        let lock = self.resource_lock(resource);
        tokio::time::timeout(wait, lock.lock_owned())
            .await
            .map_err(|_| busy(resource))
    }
    pub(crate) async fn share_for(
        &self,
        resource: &str,
        wait: std::time::Duration,
    ) -> Result<Shared> {
        let lock = self.resource_lock(resource);
        tokio::time::timeout(wait, lock.share_owned())
            .await
            .map_err(|_| busy(resource))
    }
}

/// The answer to a request that waited for a resource in vain.
pub(crate) fn busy(resource: &str) -> crate::error::Error {
    let what = match resource.split_once(':') {
        Some(("container", _)) => "the container",
        Some(("volume", _)) => "the volume",
        Some(("image", _)) => "the image",
        _ => "this resource",
    };
    crate::error::Error(
        axum::http::StatusCode::CONFLICT,
        format!("Another operation on {what} is still running; retry when it has finished"),
    )
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod tests;
