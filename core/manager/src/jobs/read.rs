//! Reading a job as an agent follows it: waiting a bounded time for it to end, so an agent does
//! not poll in a loop, and only part of its log, the end or what is new since the last read, so a
//! read does not carry the whole 256 KiB log every time.
//!
//! A log keeps its last [`LOG_LIMIT`] bytes, while its positions count every byte ever added to
//! it: an answer that asks for part of the log says where that part starts and where the log
//! ends, and the next read asks from that end.
use crate::{
    App,
    error::Result,
    human::Caller,
    model::{Job, LOG_LIMIT},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Mutex, time::Duration};
use tokio::{sync::broadcast::error::RecvError, time::Instant};

/// The longest one read waits; the agent reads again to wait longer.
pub(crate) const MAX_WAIT: u64 = 300;
/// How much of the log an MCP read returns when the agent does not say: the end, where a
/// failure's reason is.
pub(crate) const MCP_TAIL: usize = 4096;
/// How often a waiting call reports progress to a client that asked for it.
const PROGRESS_EVERY: Duration = Duration::from_secs(10);
/// The jobs whose reads without a wait are counted; one more forgets them all rather than grow.
const POLLED_JOBS: usize = 256;

/// What a read of a job asks for beyond the job itself. Without any of them the answer is the
/// whole job, as it always was.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct Read {
    /// Wait up to this many seconds, at most 300, for a queued or running job to end before
    /// answering. The answer is the job as it is then, ended or not.
    #[serde(default)]
    pub wait_seconds: Option<u64>,
    /// Return at most the last this many bytes of the log, from `log_offset` when it is given.
    #[serde(default)]
    pub log_tail_bytes: Option<usize>,
    /// Return the log from this byte of the job's whole output, such as the `log_end` of an
    /// earlier answer, to read only what was added since.
    #[serde(default)]
    pub log_offset: Option<u64>,
}

impl Read {
    fn checked(&self) -> Result<()> {
        if self.wait_seconds.is_some_and(|wait| wait > MAX_WAIT) {
            return Err(anyhow::anyhow!("wait_seconds must be at most {MAX_WAIT}").into());
        }
        if self.log_tail_bytes.is_some_and(|tail| tail > LOG_LIMIT) {
            return Err(anyhow::anyhow!("log_tail_bytes must be at most {LOG_LIMIT}").into());
        }
        Ok(())
    }
}

/// The job `id` as `read` asks for it, `retryable` once it can be started again and `cancellable`
/// while a cancel reaches it. `caller` is the
/// MCP call that waits, which hears progress and may give the wait up.
pub(crate) async fn answer(
    app: &App,
    id: &str,
    read: &Read,
    caller: Option<&Caller>,
) -> Result<Value> {
    read.checked()?;
    super::history::kept(app, id)?;
    if let Some(wait) = read.wait_seconds.filter(|wait| *wait > 0) {
        until_ended(app, id, Duration::from_secs(wait), caller).await?;
    }
    let (mut job, end) = app.db.job_and_end(id)?;
    let part = (read.log_offset.is_some() || read.log_tail_bytes.is_some())
        .then(|| window(&mut job, end, read.log_offset, read.log_tail_bytes));
    let retryable = super::retry::retryable(app, &job);
    let mut value = json!(job);
    if let Some((start, end)) = part {
        value["log_start"] = json!(start);
        value["log_end"] = json!(end);
    }
    if retryable {
        value["retryable"] = json!(true);
    }
    if app.cancels.cancellable(id) {
        value["cancellable"] = json!(true);
    }
    Ok(value)
}

/// How often each job that has not ended was read over MCP without a wait, in memory only: an
/// agent that reads one again and again is told to wait instead.
#[derive(Default)]
pub(crate) struct Polls(Mutex<HashMap<String, u32>>);

impl Polls {
    /// Count one more read of the job `id` without a wait and say how many there were.
    fn count(&self, id: &str) -> u32 {
        let mut polls = self.0.lock().unwrap();
        if polls.len() >= POLLED_JOBS && !polls.contains_key(id) {
            polls.clear();
        }
        let count = polls.entry(id.to_owned()).or_default();
        *count = count.saturating_add(1);
        *count
    }
    fn forget(&self, id: &str) {
        self.0.lock().unwrap().remove(id);
    }
}

/// [`answer`] for the MCP tool `get_job`. From the second read in a row of a job that has not
/// ended, without `wait_seconds`, the answer carries a `hint` to wait instead of polling.
pub(crate) async fn mcp_answer(
    app: &App,
    id: &str,
    read: &Read,
    caller: Option<&Caller>,
) -> Result<Value> {
    let waited = read.wait_seconds.is_some_and(|wait| wait > 0);
    let mut job = match answer(app, id, read, caller).await {
        Ok(job) => job,
        Err(error) => {
            // A job that is gone, such as one the history pruned, is no longer polled.
            app.polls.forget(id);
            return Err(error);
        }
    };
    if waited || !job["finished_at"].is_null() {
        app.polls.forget(id);
        return Ok(job);
    }
    let reads = app.polls.count(id);
    if reads > 1 {
        job["hint"] = json!(format!(
            "This job has not ended and was read {reads} times in a row without wait_seconds. Pass wait_seconds, up to {MAX_WAIT}, and one call answers as soon as the job ends."
        ));
    }
    Ok(job)
}

/// The arguments of the MCP tool `get_job`: the job and [`Read`], with the end of the log,
/// [`MCP_TAIL`] bytes, unless the agent asks for another part.
pub(crate) fn mcp_arguments(args: Value) -> Result<(String, Read)> {
    #[derive(Deserialize)]
    struct Arguments {
        job_id: String,
        #[serde(flatten)]
        read: Read,
    }
    let Arguments { job_id, mut read } = serde_json::from_value(args).map_err(|e| {
        crate::error::Error(
            axum::http::StatusCode::BAD_REQUEST,
            format!("Invalid tool arguments: {e}"),
        )
    })?;
    read.log_tail_bytes.get_or_insert(MCP_TAIL);
    Ok((job_id, read))
}

/// Cut the log of `job`, which ends at `end` of the job's whole output, to what follows `offset`
/// and of that the last `tail` bytes, each from a character's start. Returns where the part kept
/// starts and where the log ends: a start past `offset` means bytes were left out, older than
/// what the log keeps or before the tail.
pub(crate) fn window(
    job: &mut Job,
    end: u64,
    offset: Option<u64>,
    tail: Option<usize>,
) -> (u64, u64) {
    let kept_from = end.saturating_sub(job.log.len() as u64);
    let from = offset.unwrap_or(kept_from).clamp(kept_from, end);
    let mut at = usize::try_from(from - kept_from).unwrap_or(job.log.len());
    if let Some(tail) = tail {
        at = at.max(job.log.len().saturating_sub(tail));
    }
    while !job.log.is_char_boundary(at) {
        at += 1;
    }
    job.log.drain(..at);
    (kept_from + at as u64, end)
}

/// Wait up to `wait` for the job to end. A wait also ends when the client gives the call up or
/// the manager stops, and the answer is then the job as it is.
async fn until_ended(app: &App, id: &str, wait: Duration, caller: Option<&Caller>) -> Result<()> {
    let started = Instant::now();
    let deadline = started + wait;
    // Subscribed before the first look, so an end between the two is not missed.
    let mut events = app.events.subscribe();
    let mut progress = tokio::time::interval_at(started + PROGRESS_EVERY, PROGRESS_EVERY);
    progress.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let given_up = async {
        match caller {
            Some(caller) => caller.given_up().await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(given_up);
    loop {
        let job: Job = app.db.get("job", id)?;
        if job.finished_at.is_some() || Instant::now() >= deadline {
            return Ok(());
        }
        // Every job's events name it; another job's are not a reason to look again.
        loop {
            tokio::select! {
                received = events.recv() => match received {
                    Ok(event) if !event.contains(id) => continue,
                    Ok(_) | Err(RecvError::Lagged(_)) => break,
                    Err(RecvError::Closed) => return Ok(()),
                },
                () = tokio::time::sleep_until(deadline) => break,
                () = app.events.closed() => return Ok(()),
                () = &mut given_up => return Ok(()),
                _ = progress.tick() => {
                    if let Some(caller) = caller {
                        caller
                            .report_waiting(
                                started.elapsed(),
                                wait,
                                &format!("Waiting for the {} job to end", job.kind),
                            )
                            .await;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "read_tests.rs"]
mod tests;
