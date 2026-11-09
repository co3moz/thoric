//! Repeated reads as one row. An agent that polls, `get_job` above all, repeats one read
//! hundreds of times, and a row for each drowns what it did in between. A read that repeats the
//! call before it joins that call's row instead: the row counts it in `repeats`, keeps when the
//! last one began in `last_at` and the longest of their durations.
//!
//! A call joins the row before it only when both are the same read: a Thoric tool that only
//! reads and answered without an error and without starting a job, called by the same caller
//! (endpoint, credential and client) on the same container with the same arguments as the log
//! keeps them, and begun within [`WINDOW`] of the one before. Anything else the caller does on
//! that container in between, a change, an error, another tool or another argument, ends the
//! run, so the rows stay in the order things happened. Which row a caller's run is on is kept in
//! memory alone: after a restart the next read starts a row of its own.
use super::{Entry, store};
use crate::db::Db;
use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
};

/// The longest a repeated read may begin after the one before it and still join its row, in
/// milliseconds. A poll that waits, as `get_job` with `wait_seconds` does for up to five minutes,
/// begins that long after the one before.
pub(super) const WINDOW: u64 = 360_000;
/// How many runs are remembered before the ones that can no longer be joined are forgotten.
const RUNS: usize = 1024;

/// The calls of one caller on one container, or on none.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Stream {
    endpoint: Option<String>,
    credential: String,
    client: Option<String>,
    container_id: Option<String>,
}

/// The row a stream's last call is on, when that call was a read another may join.
#[derive(Debug)]
struct Run {
    id: i64,
    /// When the last call of the row began.
    at: u64,
    tool: String,
    /// The arguments as the row keeps them.
    arguments: String,
}

/// The row of each stream's running read, in memory only.
#[derive(Debug, Default)]
pub(crate) struct Recent(Mutex<HashMap<Stream, Run>>);

/// Whether a call is one another like it may join, or may itself join one.
fn joins(entry: &Entry) -> bool {
    entry.source == "tool"
        && entry.kind == "read"
        && entry.outcome == "ok"
        && entry.job_id.is_none()
}

impl Recent {
    /// Write a call: onto the row of the same read before it, or as a row of its own. Answers
    /// the row's ID.
    pub(super) fn record(&self, db: &Db, entry: &Entry, keep: usize) -> anyhow::Result<i64> {
        let stream = Stream {
            endpoint: entry.endpoint.clone(),
            credential: entry.credential.clone(),
            client: entry.client.clone(),
            container_id: entry.container_id.clone(),
        };
        let joins = joins(entry);
        let arguments = entry.arguments.to_string();
        // Held over the write, so two calls of one stream that end together are written in turn.
        let mut runs = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if joins
            && let Some(run) = runs.get_mut(&stream)
            && run.tool == entry.tool
            && run.arguments == arguments
            && entry.at.abs_diff(run.at) <= WINDOW
            && store::repeat(db, run.id, entry.at, entry.duration_ms)?
        {
            run.at = run.at.max(entry.at);
            return Ok(run.id);
        }
        let id = store::insert(db, entry, keep)?;
        if !joins {
            runs.remove(&stream);
            return Ok(id);
        }
        if runs.len() >= RUNS {
            runs.retain(|_, run| entry.at.abs_diff(run.at) <= WINDOW);
            if runs.len() >= RUNS {
                runs.clear();
            }
        }
        runs.insert(
            stream,
            Run {
                id,
                at: entry.at,
                tool: entry.tool.clone(),
                arguments,
            },
        );
        Ok(id)
    }
}

#[cfg(test)]
#[path = "merge_tests.rs"]
mod tests;
