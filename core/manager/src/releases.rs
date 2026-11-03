//! Newer releases of Thoric, and the update the dashboard asks the WSL package to run.
//!
//! Unless `UPDATE_CHECK=false`, the manager asks GitHub for the newest release when it starts and
//! every six hours after, with the `curl` its image carries for the health check, and at once when
//! the settings page asks. The dashboard offers a release that is newer than this manager.
//!
//! Only the WSL package updates from the dashboard, with the same `thoric update` a terminal
//! runs: it moves `MANAGER_IMAGE` in `/opt/thoric/.env` to the release, brings that release's
//! installation files and command, and recreates this container. None of that is reachable from
//! inside the container, so the manager asks the package through the folder it mounts at
//! [`UPDATE_DIR`], with a [`WATCHED`] file in it once the units below take requests; see
//! [`crate::package_requests`]. The manager leaves a [`REQUEST`] naming its job there, and
//! `thoric-update.path` in the distribution starts `thoric-update.service`, which renames
//! the request to [`STARTED`], runs the command as root with its output in [`LOG`] and writes how
//! it ended to [`EXIT`]. The next manager follows the update to its end; see [`resumable`].
//!
//! Before it leaves the request, the manager writes a copy of its database beside it, named after
//! the two releases, since the next manager may migrate the database so that this one no longer
//! opens it. The newest [`KEPT_COPIES`] stay; a copy that cannot be written stops the update.
use crate::{
    App,
    error::{Error, Result},
    jobs,
    model::{Job, now},
    package_requests::{self, Channel},
};
use anyhow::{Context, bail};
use axum::{Json, extract::State, http::StatusCode};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

pub(crate) use package_requests::Timing;

/// This manager's release.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The newest published release. Drafts and prereleases are left out, as `thoric update`
/// leaves them out.
const RELEASES: &str = "https://api.github.com/repos/co3moz/thoric/releases/latest";
/// Where a release's notes are, followed by its version.
const RELEASE_PAGE: &str = "https://github.com/co3moz/thoric/releases/tag/v";
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// curl gives up within its own limits below; the process is stopped here in any case.
const CHECK_LIMIT: Duration = Duration::from_secs(30);

/// The package's `/opt/thoric/update` in the manager container.
pub(crate) const UPDATE_DIR: &str = "/update";
/// Put there by the package and by `thoric update` once the path unit watches the folder.
pub(crate) const WATCHED: &str = "watched";
pub(crate) const REQUEST: &str = "request";
pub(crate) const STARTED: &str = "started";
pub(crate) const LOG: &str = "log";
pub(crate) const EXIT: &str = "exit";
/// The kind of the job that follows an update.
pub(crate) const JOB: &str = "update";
/// The update's files in the folder.
pub(crate) const UPDATE: Channel = Channel {
    job: JOB,
    request: REQUEST,
    started: STARTED,
    log: LOG,
    exit: EXIT,
    path_unit: "thoric-update.path",
    command: "thoric update",
};

/// How many copies of the database the dashboard's updates keep beside it; older ones go.
const KEPT_COPIES: usize = 3;
/// A copy's name is `state-<this release>-to-<the next>.sqlite`.
const COPY_PREFIX: &str = "state-";
const COPY_SUFFIX: &str = ".sqlite";

/// The service's own limit is two hours; a little more covers reading what it wrote last.
pub(crate) const TIMING: Timing = Timing {
    take: Duration::from_secs(30),
    poll: Duration::from_secs(1),
    limit: Duration::from_secs(2 * 3600 + 300),
};
/// What the last look at GitHub found.
#[derive(Default)]
pub(crate) struct Releases {
    last: Mutex<Check>,
    /// Held while GitHub is asked, so one look runs at a time.
    asking: tokio::sync::Mutex<()>,
}
#[derive(Clone, Default)]
struct Check {
    /// The newest release seen, kept when a later look fails.
    newest: Option<String>,
    checked_at: Option<u64>,
    error: Option<String>,
    /// How many looks have ended.
    looks: u64,
}

impl Releases {
    /// Ask GitHub with `ask`, unless a look that ended while this one waited for its turn already
    /// did: a second click, or the six-hourly look, answers for this one too. Returns a release
    /// newer than this manager that this look is the first to see.
    pub(crate) async fn check(
        &self,
        ask: impl Future<Output = anyhow::Result<String>>,
    ) -> Option<String> {
        let before = self.last.lock().unwrap().looks;
        let _turn = self.asking.lock().await;
        if self.last.lock().unwrap().looks == before {
            return self.record(ask.await);
        }
        None
    }
    fn record(&self, result: anyhow::Result<String>) -> Option<String> {
        let mut check = self.last.lock().unwrap();
        check.looks += 1;
        check.checked_at = Some(now());
        match result {
            Ok(version) => {
                let seen =
                    newer(&version, VERSION) && check.newest.as_deref() != Some(version.as_str());
                if seen {
                    tracing::info!("Thoric {version} is available; this manager is {VERSION}");
                }
                check.newest = Some(version.clone());
                check.error = None;
                seen.then_some(version)
            }
            Err(error) => {
                tracing::debug!("Release check: {error:#}");
                check.error = Some(format!("{error:#}"));
                None
            }
        }
    }
    /// The newest release when it is newer than this manager.
    fn available(&self) -> Option<String> {
        let check = self.last.lock().unwrap();
        check.newest.clone().filter(|newest| newer(newest, VERSION))
    }
}

/// Look for new releases for as long as the manager runs, unless the installation turned it off.
pub(crate) fn spawn(app: Arc<App>) {
    if !app.config.update_check {
        return;
    }
    tokio::spawn(async move {
        loop {
            let seen = app.releases.check(newest_release()).await;
            announce(&app, seen).await;
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// The release whose notice was sent last, in the manager's data volume, so that a manager that
/// starts again, as every settings change makes it, does not send it again.
const ANNOUNCED: &str = "announced-release";

/// Send the notice of a release a look was the first to see, unless one was sent for it before,
/// also by an earlier manager.
async fn announce(app: &App, seen: Option<String>) {
    let Some(version) = seen else {
        return;
    };
    if !crate::notify::wants(app, crate::notify::Kind::UpdateAvailable) {
        return;
    }
    let file = app.config.data.join(ANNOUNCED);
    if tokio::fs::read_to_string(&file)
        .await
        .is_ok_and(|sent| sent.trim() == version)
    {
        return;
    }
    if let Err(error) = tokio::fs::write(&file, &version).await {
        tracing::debug!("Could not keep the release announced: {error}");
    }
    let notes = format!("{RELEASE_PAGE}{version}");
    crate::notify::send(
        app,
        crate::notify::notices::update_available(&version, &notes),
    );
}

async fn newest_release() -> anyhow::Result<String> {
    let mut command = tokio::process::Command::new("curl");
    command
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--max-time",
            "20",
            "--max-filesize",
            "1048576",
            "--header",
            "Accept: application/vnd.github+json",
            RELEASES,
        ])
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(CHECK_LIMIT, command.output())
        .await
        .context("GitHub did not answer in time")?
        .context("Could not run curl")?;
    if !output.status.success() {
        bail!(
            "Could not read the newest release from GitHub: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    newest(&output.stdout)
}

/// The version of the release GitHub describes, without the v of its tag.
pub(crate) fn newest(body: &[u8]) -> anyhow::Result<String> {
    let release: Value =
        serde_json::from_slice(body).context("GitHub did not answer with a release")?;
    let tag = release["tag_name"].as_str().unwrap_or_default();
    let version = tag.strip_prefix('v').unwrap_or(tag);
    if parse(version).is_none() {
        bail!("The newest release has a tag that is not a version: {tag:.40}");
    }
    Ok(version.into())
}

fn parse(version: &str) -> Option<[u64; 3]> {
    let mut parts = version.split('.').map(|part| {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse::<u64>().ok()
    });
    let numbers = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some(numbers)
}

/// Whether `candidate` is a later release than `current`. Anything other than
/// `major.minor.patch` is never newer, nor older.
pub(crate) fn newer(candidate: &str, current: &str) -> bool {
    matches!((parse(candidate), parse(current)), (Some(candidate), Some(current)) if candidate > current)
}

/// Whether the WSL distribution takes update requests: its package mounts the folder with
/// [`WATCHED`] in it. Elsewhere the folder is missing or empty.
pub(crate) fn installable(dir: &Path) -> bool {
    dir.join(WATCHED).is_file()
}

/// `GET /api/update`: this release, the commit of a build of `master`, the newest release and
/// whether the dashboard can update to it.
pub async fn status(State(app): State<Arc<App>>) -> Result<Json<Value>> {
    let check = app.releases.last.lock().unwrap().clone();
    let running = app
        .db
        .active_jobs()?
        .into_iter()
        .find(|job| job.kind == JOB);
    Ok(Json(json!({
        "enabled": app.config.update_check,
        "current": VERSION,
        "commit": crate::catalog::edge_commit(),
        "newest": check.newest,
        "available": check.newest.as_deref().is_some_and(|newest| newer(newest, VERSION)),
        "release_url": check.newest.as_ref().map(|version| format!("{RELEASE_PAGE}{version}")),
        "checked_at": check.checked_at,
        "error": check.error,
        "installable": app.config.update_check && installable(&app.config.update_dir),
        "job_id": running.map(|job| job.id),
    })))
}

/// `POST /api/update/check`: ask GitHub now rather than at the next six-hourly look, and answer
/// as `GET /api/update` does once it has answered.
pub async fn check(State(app): State<Arc<App>>) -> Result<Json<Value>> {
    if !app.config.update_check {
        return Err(Error(
            StatusCode::CONFLICT,
            "Release checks are turned off with UPDATE_CHECK=false".into(),
        ));
    }
    let seen = app.releases.check(newest_release()).await;
    announce(&app, seen).await;
    status(State(app)).await
}

/// `POST /api/update`: ask the WSL distribution to run `thoric update`; returns a job.
pub async fn start(State(app): State<Arc<App>>) -> Result<Json<Value>> {
    let conflict = |message: &str| Error(StatusCode::CONFLICT, message.into());
    if !app.config.update_check {
        return Err(conflict(
            "Release checks are turned off with UPDATE_CHECK=false",
        ));
    }
    if !installable(&app.config.update_dir) {
        return Err(conflict(
            "Only the WSL package updates from the dashboard; update this installation the way it was installed",
        ));
    }
    if app.releases.available().is_none() {
        return Err(conflict("This manager is the newest release"));
    }
    // Every job of this manager ends with it, so none may be running.
    if !app.db.active_jobs()?.is_empty() {
        return Err(conflict(
            "The update restarts the manager; start it once the running jobs have finished",
        ));
    }
    jobs::launch(app, JOB, JOB.into(), |app, id| async move {
        run(&app, &id, TIMING).await
    })
}

/// Copy the database, leave the request, wait for the distribution to take it and follow the
/// update.
pub(crate) async fn run(app: &Arc<App>, id: &str, timing: Timing) -> anyhow::Result<Value> {
    let newest = app.releases.available();
    // The new manager may migrate the database so that this release no longer opens it; the
    // copy is the way back. Without it the update does not start.
    let copy = copy_database(app, newest.as_deref()).await.map_err(|error| {
        anyhow::anyhow!(
            "Could not copy the database before the update, so nothing was updated ({error:#}). Make room in the manager's data volume, or run 'thoric update' in a Thoric terminal, which updates without the copy"
        )
    })?;
    app.append_log(
        id,
        &format!("Copied the database to {copy} in the manager's data volume, the way back to this release's records if the update has to be undone."),
    )?;
    let target = newest.unwrap_or_else(|| "the newest release".into());
    app.append_log(
        id,
        &format!("Asked the WSL distribution to run thoric update, which moves this installation to {target}."),
    )?;
    let ended = package_requests::request(app, &UPDATE, id, &[], timing).await?;
    outcome(&ended)
}

/// Write a copy of the database beside it, named after this manager and the release the update
/// moves to, such as `state-0.8.4-to-0.9.0.sqlite`, and keep only the newest [`KEPT_COPIES`] of
/// them. Returns its name.
async fn copy_database(app: &Arc<App>, target: Option<&str>) -> anyhow::Result<String> {
    // An edge build carries the version of the release before it, so its commit names it.
    let from = match crate::catalog::edge_commit() {
        Some(commit) => format!("sha-{}", &commit[..commit.len().min(12)]),
        None => VERSION.into(),
    };
    let name = format!(
        "{COPY_PREFIX}{from}-to-{}{COPY_SUFFIX}",
        target.unwrap_or("newest")
    );
    let app = app.clone();
    tokio::task::spawn_blocking(move || {
        let dir = &app.config.data;
        // Written under another name and renamed, so a copy cut short never passes for one.
        let partial = dir.join(format!("{name}.partial"));
        match std::fs::remove_file(&partial) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(error).context("Could not remove an earlier partial copy");
            }
            _ => {}
        }
        app.db.copy_to(&partial)?;
        std::fs::rename(&partial, dir.join(&name))?;
        if let Err(error) = prune_copies(dir, KEPT_COPIES) {
            tracing::warn!("Could not remove older copies of the database: {error:#}");
        }
        Ok(name)
    })
    .await?
}

/// Remove the copies of the database beyond the newest `keep`.
fn prune_copies(dir: &Path, keep: usize) -> anyhow::Result<()> {
    let mut copies = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(COPY_PREFIX) && name.ends_with(COPY_SUFFIX) && name.contains("-to-") {
            copies.push((entry.metadata()?.modified()?, entry.path()));
        }
    }
    copies.sort_by(|a, b| b.cmp(a));
    for (_, path) in copies.into_iter().skip(keep) {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Whether an update job outlived the manager that started it; see
/// [`package_requests::resumable`].
pub(crate) fn resumable(dir: &Path, job: &Job) -> bool {
    package_requests::resumable(&UPDATE, dir, job)
}

/// Follow an update the previous manager started, to its end.
pub(crate) fn resume(app: Arc<App>, job: Job) {
    package_requests::resume(app, &UPDATE, job, TIMING, outcome);
}

/// The job's result from what the service wrote when it ended: systemd's result for the
/// service and the command's exit status.
pub(crate) fn outcome(ended: &str) -> anyhow::Result<Value> {
    if package_requests::succeeded(ended) {
        return Ok(json!({"version": VERSION}));
    }
    bail!(
        "thoric update did not succeed in the WSL distribution ({:.80})",
        ended.trim()
    )
}

#[cfg(test)]
#[path = "releases_tests.rs"]
mod tests;
