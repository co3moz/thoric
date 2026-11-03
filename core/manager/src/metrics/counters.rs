//! What `/metrics` keeps between scrapes: when the manager started, the counters of what ended
//! since then (jobs, MCP tool calls, transfer links), the wrong tokens of each address and the
//! volume count. A counter is a map behind a lock held only to add one or to copy them out, so
//! counting adds nothing a call or a job would notice.
use crate::{
    App,
    activity::{Outcome, Source},
    auth::Limiter,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// How old the volume count may be before a scrape has it counted again.
pub(crate) const VOLUMES_EVERY: Duration = Duration::from_secs(60);
/// What a tool call outside Thoric's catalog is counted as, so a caller's names never become labels.
pub(crate) const UNKNOWN_TOOL: &str = "unknown";

/// The metrics state of one manager.
pub(crate) struct Metrics {
    /// When the manager started, in Unix seconds.
    pub started: f64,
    /// Wrong tokens of the last minute, by the address they came from. Not the sign-in's: a scrape
    /// with the right token forgets its address's failures, which must not lift a sign-in lockout.
    pub attempts: Limiter,
    counts: Mutex<Counts>,
    volumes: Mutex<Volumes>,
}

/// The counters since the manager started.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Counts {
    /// Ended jobs, by kind and status.
    pub jobs: BTreeMap<(String, String), u64>,
    /// Calls of Thoric's own tools, by tool and outcome.
    pub tools: BTreeMap<(String, &'static str), u64>,
    /// Calls of desktop servers' tools, by outcome. Their names come from inside the desktops,
    /// so they are counted together.
    pub servers: BTreeMap<&'static str, u64>,
    /// Transfer links used, by operation and outcome.
    pub transfers: BTreeMap<(String, &'static str), u64>,
}

#[derive(Debug, Default)]
struct Volumes {
    count: Option<u64>,
    /// When a count was last started, whether or not Docker answered it.
    asked: Option<Instant>,
}

impl Default for Metrics {
    fn default() -> Self {
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        Self {
            started,
            attempts: Limiter::default(),
            counts: Mutex::default(),
            volumes: Mutex::default(),
        }
    }
}

impl Metrics {
    fn counts_mut(&self) -> MutexGuard<'_, Counts> {
        // A panic while a counter was added leaves at most that one behind.
        self.counts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A job of `kind` ended with `status`, `completed` or `failed`.
    pub(crate) fn job_ended(&self, kind: &str, status: &str) {
        *self
            .counts_mut()
            .jobs
            .entry((kind.to_owned(), status.to_owned()))
            .or_default() += 1;
    }

    /// An MCP tool call or a transfer ended as `outcome`. A Thoric tool is counted by its name
    /// only when the catalog has it, since a refused call may name anything.
    pub(crate) fn called(&self, source: Source, tool: &str, outcome: Outcome) {
        let outcome = outcome.id();
        let tool = match source {
            Source::Tool if crate::mcp::listed_tool(tool).is_none() => UNKNOWN_TOOL,
            _ => tool,
        };
        let mut counts = self.counts_mut();
        let count = match source {
            Source::Tool => counts.tools.entry((tool.to_owned(), outcome)).or_default(),
            Source::Server => counts.servers.entry(outcome).or_default(),
            Source::Transfer => counts
                .transfers
                .entry((tool.to_owned(), outcome))
                .or_default(),
        };
        *count += 1;
    }

    pub(crate) fn counts(&self) -> Counts {
        self.counts_mut().clone()
    }

    /// The volumes the Volumes page lists, as last counted. A count older than [`VOLUMES_EVERY`]
    /// is started again in the background, so no scrape waits for Docker and many scrapes make one
    /// listing a minute; `None` until the first count and while Docker does not answer.
    pub(crate) fn volumes(&self, app: &Arc<App>) -> Option<u64> {
        let mut volumes = self
            .volumes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if volumes
            .asked
            .is_none_or(|asked| asked.elapsed() >= VOLUMES_EVERY)
        {
            volumes.asked = Some(Instant::now());
            let app = app.clone();
            tokio::spawn(async move {
                let count = count_volumes(&app).await;
                if let Err(error) = &count {
                    tracing::debug!("Metrics: volume count: {error:#}");
                }
                app.metrics
                    .volumes
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .count = count.ok();
            });
        }
        volumes.count
    }
}

/// One listing of the volumes `VOLUME_FILTER` lets the manager list, given up before the next is
/// due, which ends the command.
async fn count_volumes(app: &App) -> anyhow::Result<u64> {
    let args = crate::volumes::list_args(app.config.volume_filter.as_deref());
    let listed = tokio::time::timeout(VOLUMES_EVERY / 2, app.docker_args(&args))
        .await
        .map_err(|_| anyhow::anyhow!("Docker did not list the volumes in time"))??;
    let lines = String::from_utf8_lossy(&listed)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    Ok(u64::try_from(lines).unwrap_or(u64::MAX))
}

#[cfg(test)]
#[path = "counters_tests.rs"]
mod tests;
