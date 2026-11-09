//! The job history: which ended jobs stay, which a list names, what a read of a pruned job is
//! answered with, and the counts of ended jobs that outlive their records.
//!
//! An agent runs scripts by the thousand, about 1,500 `exec` jobs in six hours on one measured
//! installation, so the script jobs are a class of their own. Each class keeps its own newest
//! jobs: scripts prune scripts, and a build, a creation or an update stays until that many jobs of
//! its own class came after it. A list can leave the scripts out, and the hourly counts of
//! [`COUNTED_DAYS`] days tell how busy the manager was long after the records went.
use crate::{
    App,
    db::{Db, job_history::Filter},
    error::{Error, Result},
    model::{Job, now},
};
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

/// The kinds of the script class: a script an agent or a person runs in a desktop. No other kind
/// comes in such numbers; a new one that does is added here.
pub(crate) const SCRIPT_KINDS: &[&str] = &["exec"];

/// How many finished script jobs are kept, the newest. A script's record is about 1 KiB with its
/// result's `output_tail` of at most 8 KiB, and its log is mostly a few lines, so this is a few
/// megabytes; were every log at its 256 KiB limit it would be 256 MiB.
pub(crate) const KEPT_SCRIPT_JOBS: usize = 1000;
/// How many finished jobs of every other kind are kept, the newest: as many as all jobs
/// together before scripts were a class, at most 128 MiB of logs at their limit.
pub(crate) const KEPT_JOBS: usize = 500;
/// How many days the hourly counts of ended jobs are kept: the longest range the dashboard
/// charts, 90 days, and two more, so its first and last local day are whole in any time zone.
/// That is at most 4,416 small rows.
pub(crate) const COUNTED_DAYS: u64 = 92;
/// The longest range the counts' route answers, and the one it answers without `days`.
const MAX_DAYS: u64 = 90;
const DEFAULT_DAYS: u64 = 30;
/// The kinds one list may name.
const MAX_KINDS: usize = 32;

/// What every class keeps of its finished jobs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Kept {
    pub scripts: usize,
    pub others: usize,
}

/// What the manager keeps.
pub(crate) const KEPT: Kept = Kept {
    scripts: KEPT_SCRIPT_JOBS,
    others: KEPT_JOBS,
};

/// A job's class: the scripts, or everything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    Script,
    Other,
}

impl Class {
    pub(crate) fn of(kind: &str) -> Self {
        if SCRIPT_KINDS.contains(&kind) {
            Self::Script
        } else {
            Self::Other
        }
    }
    /// The class as the counts name it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Script => "script",
            Self::Other => "other",
        }
    }
}

/// The jobs of one class that ended in one hour.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Count {
    /// When the hour began, in seconds since the Unix epoch.
    pub at: u64,
    /// `script` or `other`.
    pub class: String,
    pub completed: u64,
    pub failed: u64,
    pub interrupted: u64,
}

/// Which jobs `GET /api/jobs` and the MCP tool `list_jobs` list. Without either field the answer
/// is the newest 100 jobs of every kind, as it always was.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct Listed {
    /// Only jobs of this kind, such as `build`; several kinds are separated by commas, as
    /// `build,snapshot,create`.
    #[serde(default)]
    pub kind: Option<String>,
    /// `false` leaves the script jobs (`exec`) out, so the newest 100 of every other kind are
    /// listed; `true` lists the script jobs alone.
    #[serde(default)]
    pub scripts: Option<bool>,
}

impl Listed {
    pub(crate) fn filter(&self) -> Result<Filter> {
        let kinds: Vec<String> = self
            .kind
            .as_deref()
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|kind| !kind.is_empty())
            .map(str::to_owned)
            .collect();
        let named = |kind: &String| {
            kind.len() <= 64
                && kind
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        };
        if kinds.len() > MAX_KINDS || !kinds.iter().all(named) {
            return Err(Error(
                StatusCode::BAD_REQUEST,
                format!(
                    "kind is a job kind such as build, or at most {MAX_KINDS} of them separated by commas"
                ),
            ));
        }
        Ok(Filter {
            kinds,
            class: self.scripts.map(|scripts| match scripts {
                true => Class::Script,
                false => Class::Other,
            }),
        })
    }
}

/// Prune the finished jobs beyond what each class keeps.
pub(crate) fn prune(db: &Db) -> anyhow::Result<usize> {
    db.prune_jobs(KEPT)
}

/// Count a job that ended, whose record says how and when, among its hour's.
pub(crate) fn count(db: &Db, job: &Job) -> anyhow::Result<()> {
    db.count_job_end(
        Class::of(&job.kind),
        &job.status,
        job.finished_at.unwrap_or_else(now),
        COUNTED_DAYS * 24,
    )
}

/// What follows a job's end for the history: it is counted, and the jobs it pushed beyond what
/// their class keeps go. Neither fails the job.
pub(crate) fn ended(app: &App, job: &Job) {
    if let Err(e) = count(&app.db, job) {
        tracing::debug!("Job counts: {e:#}");
    }
    if let Err(e) = prune(&app.db) {
        tracing::debug!("Job history: {e:#}");
    }
}

/// Whether `id` has the shape of a job's id, 32 lowercase hexadecimal digits.
fn job_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Refuse the read of a job whose record is not kept, saying that it was pruned when the manager
/// remembers pruning it, and that it may have been when the id is a job's by its shape: the ids
/// pruned are remembered only since the manager started, and only the newest.
pub(crate) fn kept(app: &App, id: &str) -> Result<()> {
    if app.db.contains("job", id)? {
        return Ok(());
    }
    Err(if app.db.was_pruned(id) {
        pruned(id)
    } else if job_id(id) {
        Error(
            StatusCode::NOT_FOUND,
            format!(
                "No job {id} is kept. If it ran earlier, it has ended and its record was removed: {}. Otherwise the ID is wrong",
                keeps()
            ),
        )
    } else {
        crate::error::missing()
    })
}

/// The answer to a read of a job that is known to have run and whose record is gone.
pub(crate) fn pruned(id: &str) -> Error {
    Error(
        StatusCode::NOT_FOUND,
        format!(
            "The job {id} ended and its record is no longer kept: {}. Read a job soon after it ends, or run it again",
            keeps()
        ),
    )
}

fn keeps() -> String {
    format!(
        "Thoric keeps the newest {KEPT_SCRIPT_JOBS} finished script jobs and the newest {KEPT_JOBS} finished jobs of every other kind, and removes older ones with their logs and results"
    )
}

/// What `GET /api/jobs/history` asks for.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct Counted {
    /// How many days back the counts reach, 30 unless given and at most 90.
    #[serde(default)]
    pub days: Option<u64>,
}

/// `GET /api/jobs/history`: how many jobs ended in each hour of the last `days` days and the day
/// before them, by class and by how they ended, oldest first. An hour without a job is left out.
/// The counts outlive the job records, so they cover the whole range whatever was pruned.
pub async fn counts(
    State(app): State<Arc<App>>,
    Query(counted): Query<Counted>,
) -> Result<Json<Value>> {
    let days = counted.days.unwrap_or(DEFAULT_DAYS);
    if !(1..=MAX_DAYS).contains(&days) {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            format!("days must be between 1 and {MAX_DAYS}"),
        ));
    }
    // One more day, so the first local day of a page in any time zone is whole.
    let since = now().saturating_sub((days + 1) * 86400) / 3600 * 3600;
    let hours = app.db.job_counts(since)?;
    Ok(Json(json!({
        "days": days,
        "since": since,
        "kept_days": MAX_DAYS,
        "hours": hours,
    })))
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
