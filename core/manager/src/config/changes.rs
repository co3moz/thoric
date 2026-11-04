//! Settings changes from the dashboard's Settings page.
//!
//! `.env` stays the only place settings are kept. The manager checks a change with the code it
//! starts with ([`super::Config::check_changes`]) and then asks the WSL package, through the folder of
//! [`crate::package_requests`], to write it into `/opt/thoric/.env` and recreate this
//! container: `thoric-settings.path` starts `thoric-settings.service`, which runs
//! `thoric apply-settings` as root. That command accepts only the settings of
//! [`super::EDITABLE`], checks each value again, keeps the previous file as `.env.previous`,
//! recreates the manager and puts the previous file back when the new manager does not answer.
//! The next manager follows the job to its end, as it follows an update.
//!
//! Elsewhere, such as an installation with Compose alone, the package's [`SETTINGS_WATCHED`]
//! file is missing, and the Settings page only shows the settings.
use crate::{
    App,
    error::{Error, Result},
    jobs,
    model::Job,
    package_requests::{self, Channel, Timing},
};
use axum::{Json, extract::State, http::StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};

/// Put in the package's folder once its path unit takes settings changes.
pub(crate) const SETTINGS_WATCHED: &str = "settings-watched";
pub(crate) const SETTINGS_REQUEST: &str = "settings-request";
pub(crate) const SETTINGS_STARTED: &str = "settings-started";
pub(crate) const SETTINGS_LOG: &str = "settings-log";
pub(crate) const SETTINGS_EXIT: &str = "settings-exit";
/// The kind of the job that follows a settings change.
pub(crate) const JOB: &str = "settings";
/// A settings change's files in the folder.
pub(crate) const SETTINGS: Channel = Channel {
    job: JOB,
    request: SETTINGS_REQUEST,
    started: SETTINGS_STARTED,
    log: SETTINGS_LOG,
    exit: SETTINGS_EXIT,
    path_unit: "thoric-settings.path",
    command: "thoric apply-settings",
};
/// The service's own limit is 15 minutes; a little more covers reading what it wrote last.
pub(crate) const TIMING: Timing = Timing {
    take: Duration::from_secs(30),
    poll: Duration::from_secs(1),
    limit: Duration::from_secs(16 * 60),
};

/// Whether the WSL distribution takes settings changes from the dashboard.
pub(crate) fn editable(dir: &Path) -> bool {
    dir.join(SETTINGS_WATCHED).is_file()
}

/// `GET /api/configuration`: the installation's settings for the Settings page, without secrets,
/// whether the page can change them, the change that is running, if any, what answers without a
/// credential beyond this computer (`auth::exposure`) and what the Docker Engine lacks
/// (`App::engine_gaps`).
pub async fn configuration(State(app): State<Arc<App>>) -> Result<Json<Value>> {
    let running = app
        .db
        .active_jobs()?
        .into_iter()
        .find(|job| job.kind == JOB);
    let (exposure, docker) = tokio::join!(crate::auth::exposure(&app), app.engine_gaps());
    Ok(Json(json!({
        "variables": app.config.shown(),
        "editable": editable(&app.config.update_dir),
        "job_id": running.map(|job| job.id),
        "exposure": exposure,
        "docker": docker,
    })))
}

/// A settings change: the new text of each setting, as `.env` will hold it.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Change {
    changes: BTreeMap<String, String>,
    /// Check the change and say what is wrong with it, without changing anything.
    #[serde(default)]
    dry_run: bool,
    /// Go on although jobs are running, which the manager's restart interrupts.
    #[serde(default)]
    interrupt_jobs: bool,
}

/// `POST /api/configuration`: check a change and, unless `dry_run`, ask the WSL package to apply
/// it, which restarts the manager; returns a job.
pub async fn change(State(app): State<Arc<App>>, Json(body): Json<Change>) -> Result<Json<Value>> {
    let conflict = |message: String| Error(StatusCode::CONFLICT, message);
    let refused = |message: String| Error(StatusCode::BAD_REQUEST, message);
    if !editable(&app.config.update_dir) {
        return Err(conflict(
            "Only the WSL package changes settings from the dashboard; edit .env and recreate the manager"
                .into(),
        ));
    }
    if body.changes.len() > super::EDITABLE.len() {
        return Err(refused("Too many settings in one change".into()));
    }
    // A value as the manager has it already is no change, and no reason to restart.
    let changes: BTreeMap<String, String> = body
        .changes
        .into_iter()
        .filter(|(key, value)| {
            app.config
                .environment
                .get(key)
                .is_none_or(|now| now != value)
        })
        .collect();
    let problems = app.config.check_changes(&changes);
    let active = app.db.active_jobs()?;
    if body.dry_run {
        return Ok(Json(json!({
            "errors": problems.fields,
            "error": problems.together,
            "running_jobs": active.len(),
        })));
    }
    if changes.is_empty() {
        return Err(refused("The settings already have these values".into()));
    }
    if !problems.is_empty() {
        return Err(refused(problems.to_string()));
    }
    if active
        .iter()
        .any(|job| job.kind == JOB || job.kind == crate::releases::JOB)
    {
        return Err(conflict(
            "An update or another settings change is running; try again once it has finished"
                .into(),
        ));
    }
    if !active.is_empty() && !body.interrupt_jobs {
        return Err(conflict(format!(
            "The manager restarts with the new settings, which interrupts {} running job(s); confirm to go on, or wait until they have finished",
            active.len()
        )));
    }
    jobs::launch(app, JOB, JOB.into(), move |app, id| async move {
        run(&app, &id, &changes, TIMING).await
    })
}

/// Leave the request, wait for the distribution to take it and follow the change.
pub(crate) async fn run(
    app: &App,
    id: &str,
    changes: &BTreeMap<String, String>,
    timing: Timing,
) -> anyhow::Result<Value> {
    let names: Vec<&str> = changes.keys().map(String::as_str).collect();
    app.append_log(
        id,
        &format!(
            "Asked the WSL distribution to write {} to .env and recreate the manager.",
            names.join(", ")
        ),
    )?;
    let lines: Vec<String> = changes
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    let ended = package_requests::request(app, &SETTINGS, id, &lines, timing).await?;
    outcome(&ended)
}

/// Whether a settings job outlived the manager that started it.
pub(crate) fn resumable(dir: &Path, job: &Job) -> bool {
    package_requests::resumable(&SETTINGS, dir, job)
}

/// Follow a settings change the previous manager started, to its end.
pub(crate) fn resume(app: Arc<App>, job: Job) {
    package_requests::resume(app, &SETTINGS, job, TIMING, outcome);
}

/// The job's result from what the service wrote when it ended.
pub(crate) fn outcome(ended: &str) -> anyhow::Result<Value> {
    if package_requests::succeeded(ended) {
        return Ok(json!({"applied": true}));
    }
    anyhow::bail!(
        "thoric apply-settings did not succeed in the WSL distribution ({:.80})",
        ended.trim()
    )
}

#[cfg(test)]
#[path = "changes_tests.rs"]
mod tests;
