//! The Prometheus metrics at `GET /metrics`, in the text exposition format 0.0.4, for a scraper
//! that keeps their history beyond the half hour the usage charts hold.
//!
//! Prometheus holds no dashboard session, so the route lives outside `/api` and asks for a bearer
//! token of its own, `METRICS_TOKEN`; while none is set it answers 404, also with sign-in off. The
//! network and Host checks every route gets still come first. A wrong token counts against its
//! address in a limiter of the sign-in's kind, and an address that sent too many is refused for up
//! to a minute.
//!
//! Building the answer asks Docker nothing. It reads what the manager holds: the usage collector's
//! latest listing of the containers (every two seconds) and stats message of each running one
//! (every second), the host GPU's latest sample (every second while a GPU desktop runs), the
//! container and image records, the jobs that have not ended, the requests for a person and the
//! counters of [`counters`]. The one figure that needs Docker is the volume count, which a scrape
//! has counted again in the background once it is a minute old.
//!
//! Labels stay bounded: a container is named by its `id` and `name`, a tool by its name in
//! Thoric's catalog, and everything else by small fixed sets. The host's GPU is the host's, under
//! names of its own, and never a container's, since WSL reports no container's share.
mod counters;
mod exposition;

pub(crate) use counters::Metrics;

use crate::{
    App,
    auth::{self, Peer, Verdict},
    error::{Error, Result},
    model::{CONTAINERS, Container},
    usage::{
        current::{Current, GPU_STATES, Gpu, Running},
        stats::Counters,
    },
};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use exposition::{CONTENT_TYPE, Kind, Page};
use std::{collections::BTreeMap, net::IpAddr, sync::Arc, time::Instant};
use subtle::ConstantTimeEq;

/// What a scrape reads of a saved image's record, which spares decoding its icon and build.
#[derive(serde::Deserialize)]
struct ImageKind {
    kind: String,
}

/// The states a container is counted in: Docker's, `missing` for a record whose container Docker
/// does not list and `unavailable` while Docker cannot be listed. A state Docker adds later is
/// counted as `unknown`.
const STATES: [&str; 9] = [
    "created",
    "running",
    "paused",
    "restarting",
    "removing",
    "exited",
    "dead",
    "missing",
    "unavailable",
];

/// `GET /metrics`.
pub async fn serve(State(app): State<Arc<App>>, Peer(peer): Peer, headers: HeaderMap) -> Response {
    if let Some(refused) = refusal(&app, peer, &headers) {
        return refused;
    }
    match page(&app) {
        Ok(text) => ([(header::CONTENT_TYPE, CONTENT_TYPE)], text).into_response(),
        Err(error) => error.into_response(),
    }
}

/// The answer to a scrape that may not read the metrics: 404 while no token is set, 401 without
/// the right one and 429 for an address that sent too many wrong ones, each with the `{"error"}`
/// body of every route. Neither the token nor what was sent is ever logged.
fn refusal(app: &App, peer: Option<IpAddr>, headers: &HeaderMap) -> Option<Response> {
    let Some(expected) = app.config.metrics_token.as_deref() else {
        return Some(Error(StatusCode::NOT_FOUND, "Not found".into()).into_response());
    };
    let unauthorized = || {
        let refused = Error(
            StatusCode::UNAUTHORIZED,
            "A valid metrics bearer token is required".into(),
        );
        (
            [(header::WWW_AUTHENTICATE, "Bearer realm=\"Thoric metrics\"")],
            refused,
        )
            .into_response()
    };
    // The scheme is a word of any case.
    let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token)
    else {
        return Some(unauthorized());
    };
    let client = auth::client(app, peer, headers);
    let shown = client.map_or_else(|| "an unknown address".to_owned(), |ip| ip.to_string());
    if let Verdict::Refused { first } = app.metrics.attempts.begin(client, Instant::now()) {
        if first {
            tracing::warn!(
                "Metrics scrapes from {shown} are refused for up to a minute after repeated wrong tokens"
            );
        }
        let refused = Error(
            StatusCode::TOO_MANY_REQUESTS,
            "Try again in one minute".into(),
        );
        return Some(refused.into_response());
    }
    if !bool::from(token.as_bytes().ct_eq(expected.as_bytes())) {
        tracing::warn!("Wrong metrics token from {shown}");
        return Some(unauthorized());
    }
    app.metrics.attempts.succeeded(client);
    None
}

/// The page a scrape answers with.
fn page(app: &Arc<App>) -> Result<String> {
    let records: Vec<Container> = app.db.list(CONTAINERS)?;
    let images: Vec<ImageKind> = app.db.list("image")?;
    let current = app.usage.current();
    let mut page = Page::default();
    page.family(
        "thoric_info",
        Kind::Gauge,
        "The release of the manager that answers, always 1.",
    )
    .sample(&[("version", env!("CARGO_PKG_VERSION"))], 1.0);
    page.family(
        "thoric_start_time_seconds",
        Kind::Gauge,
        "When the manager started, in Unix seconds; the counters below count from then.",
    )
    .sample(&[], app.metrics.started);
    containers(&mut page, &records, &current);
    let mut kinds: BTreeMap<&str, u64> = BTreeMap::new();
    for image in &images {
        *kinds.entry(image.kind.as_str()).or_default() += 1;
    }
    let mut family = page.family(
        "thoric_images",
        Kind::Gauge,
        "Saved images by how they were made: build, snapshot, import or archive.",
    );
    for (kind, count) in kinds {
        family.sample(&[("kind", kind)], count as f64);
    }
    if let Some(volumes) = app.metrics.volumes(app) {
        page.family(
            "thoric_volumes",
            Kind::Gauge,
            "Named volumes the Volumes page lists, counted at most once a minute.",
        )
        .sample(&[], volumes as f64);
    }
    activity(&mut page, app);
    gpu(&mut page, &current);
    Ok(page.finish())
}

/// The containers by state and each running one's figures.
fn containers(page: &mut Page, records: &[Container], current: &Current) {
    let mut states: BTreeMap<&str, u64> = STATES.iter().map(|state| (*state, 0)).collect();
    for record in records {
        let state = match &current.states {
            None => "unavailable",
            Some(states) => match states.get(&record.id) {
                None => "missing",
                Some(state) => STATES
                    .iter()
                    .find(|known| **known == state.as_str())
                    .copied()
                    .unwrap_or("unknown"),
            },
        };
        *states.entry(state).or_default() += 1;
    }
    let mut family = page.family(
        "thoric_containers",
        Kind::Gauge,
        "Containers by the state Docker lists them in, missing for a record Docker has no container of, unavailable while Docker cannot be listed.",
    );
    for state in STATES.iter().copied().chain(["unknown"]) {
        match states.get(state) {
            Some(count) if *count > 0 || state != "unknown" => {
                family.sample(&[("state", state)], *count as f64);
            }
            _ => {}
        }
    }
    page.family(
        "thoric_usage_available",
        Kind::Gauge,
        "1 while the usage collector follows Docker, 0 while it cannot list the containers.",
    )
    .sample(&[], if current.available { 1.0 } else { 0.0 });
    // Only running containers that still have a record, under the name it has now, and none
    // while Docker cannot be listed, since which of them still run is not known.
    let listed = current.states.is_some();
    let running: Vec<(&Container, &Running)> = current
        .running
        .iter()
        .filter(|_| listed)
        .filter_map(|running| {
            let record = records.iter().find(|record| record.id == running.id)?;
            Some((record, running))
        })
        .collect();
    let mut family = page.family(
        "thoric_container_start_time_seconds",
        Kind::Gauge,
        "When Docker last started the running container, in Unix seconds.",
    );
    for (record, running) in &running {
        if let Some(started) = running.started {
            family.sample(&labels(record), started);
        }
    }
    let mut family = page.family(
        "thoric_container_cpu_limit_cores",
        Kind::Gauge,
        "The CPUs the running container's settings limit it to, where they do.",
    );
    for (record, _) in &running {
        if let Some(cpus) = record.spec.cpus {
            family.sample(&labels(record), cpus);
        }
    }
    type Figure = fn(&Counters) -> Option<f64>;
    let figures: [(&'static str, Kind, &str, Figure); 9] = [
        (
            "thoric_container_cpu_seconds_total",
            Kind::Counter,
            "CPU time the running container used since Docker started it.",
            |c| Some(c.cpu_time as f64 / 1e9),
        ),
        (
            "thoric_container_cpu_cores",
            Kind::Gauge,
            "Cores of the host the running container used over Docker's latest second.",
            |c| Some(f64::from(c.cpu)),
        ),
        (
            "thoric_container_memory_bytes",
            Kind::Gauge,
            "Memory the running container uses without the page cache, as docker stats counts it.",
            |c| Some(c.memory as f64),
        ),
        (
            "thoric_container_memory_limit_bytes",
            Kind::Gauge,
            "The running container's memory limit, or the host's memory without one.",
            |c| Some(c.memory_limit as f64),
        ),
        (
            "thoric_container_network_receive_bytes_total",
            Kind::Counter,
            "Bytes the running container received on its networks since Docker started it.",
            |c| Some(c.net_rx as f64),
        ),
        (
            "thoric_container_network_transmit_bytes_total",
            Kind::Counter,
            "Bytes the running container sent on its networks since Docker started it.",
            |c| Some(c.net_tx as f64),
        ),
        (
            "thoric_container_block_read_bytes_total",
            Kind::Counter,
            "Bytes the running container read from block devices since Docker started it.",
            |c| Some(c.disk_read as f64),
        ),
        (
            "thoric_container_block_write_bytes_total",
            Kind::Counter,
            "Bytes the running container wrote to block devices since Docker started it.",
            |c| Some(c.disk_write as f64),
        ),
        (
            "thoric_container_pids",
            Kind::Gauge,
            "Processes and threads in the running container, where Docker counts them.",
            |c| c.pids.map(|pids| pids as f64),
        ),
    ];
    for (name, kind, help, figure) in figures {
        let mut family = page.family(name, kind, help);
        for (record, running) in &running {
            if let Some(value) = running.counters.as_ref().and_then(figure) {
                family.sample(&labels(record), value);
            }
        }
    }
}

/// A container's labels: its ID and the name its record has now.
fn labels(record: &Container) -> [(&'static str, &str); 2] {
    [
        ("id", record.id.as_str()),
        ("name", record.spec.name.as_str()),
    ]
}

/// Jobs, agents' calls and their requests for a person.
fn activity(page: &mut Page, app: &App) {
    let (queued, running) = app.cancels.counts();
    page.family(
        "thoric_jobs",
        Kind::Gauge,
        "Jobs that have not ended, queued for their resource or running.",
    )
    .sample(&[("state", "queued")], queued as f64)
    .sample(&[("state", "running")], running as f64);
    let counts = app.metrics.counts();
    let mut family = page.family(
        "thoric_jobs_finished_total",
        Kind::Counter,
        "Jobs that ended since the manager started, by kind and status, completed or failed.",
    );
    for ((kind, status), count) in &counts.jobs {
        family.sample(
            &[("kind", kind.as_str()), ("status", status.as_str())],
            *count as f64,
        );
    }
    let mut family = page.family(
        "thoric_mcp_tool_calls_total",
        Kind::Counter,
        "Calls of Thoric's MCP tools on /mcp and the MCP endpoints since the manager started, by tool and outcome: ok, error, or refused before the tool ran. A name outside the catalog counts as unknown.",
    );
    for ((tool, outcome), count) in &counts.tools {
        family.sample(
            &[("tool", tool.as_str()), ("outcome", *outcome)],
            *count as f64,
        );
    }
    let mut family = page.family(
        "thoric_mcp_server_tool_calls_total",
        Kind::Counter,
        "Calls of desktop MCP servers' tools through the MCP endpoints since the manager started, by outcome.",
    );
    for outcome in ["ok", "error", "refused"] {
        let count = counts.servers.get(outcome).copied().unwrap_or_default();
        family.sample(&[("outcome", outcome)], count as f64);
    }
    let mut family = page.family(
        "thoric_transfers_total",
        Kind::Counter,
        "Transfer links used since the manager started, by operation and outcome.",
    );
    for ((operation, outcome), count) in &counts.transfers {
        family.sample(
            &[("operation", operation.as_str()), ("outcome", *outcome)],
            *count as f64,
        );
    }
    page.family(
        "thoric_human_requests_pending",
        Kind::Gauge,
        "Agents' requests for a person that wait for an answer.",
    )
    .sample(&[], app.human.pending().len() as f64);
}

/// The host's GPU, which is never a container's: WSL reports the whole adapter, Windows
/// applications included, and no container's share of it.
fn gpu(page: &mut Page, current: &Current) {
    let mut family = page.family(
        "thoric_host_gpu_state",
        Kind::Gauge,
        "Whether the host's GPU is sampled: disabled without GPU desktops, idle while none runs, sampling, or unavailable after a failed attempt.",
    );
    for state in GPU_STATES {
        let on = if state == current.gpu_state { 1.0 } else { 0.0 };
        family.sample(&[("state", state)], on);
    }
    type Figure = fn(&Gpu) -> Option<f64>;
    let figures: [(&'static str, &str, Figure); 3] = [
        (
            "thoric_host_gpu_utilization_ratio",
            "The host GPU's utilization over its latest second, of the whole adapter and never a container's share.",
            |gpu| gpu.utilization.map(|percent| f64::from(percent) / 100.0),
        ),
        (
            "thoric_host_gpu_memory_used_bytes",
            "The host GPU's memory in use, by every program of the host.",
            |gpu| gpu.memory_used.map(|bytes| bytes as f64),
        ),
        (
            "thoric_host_gpu_memory_total_bytes",
            "The host GPU's memory.",
            |gpu| gpu.memory_total.map(|bytes| bytes as f64),
        ),
    ];
    for (name, help, figure) in figures {
        let mut family = page.family(name, Kind::Gauge, help);
        for gpu in &current.gpus {
            if let Some(value) = figure(gpu) {
                let index = gpu.index.to_string();
                family.sample(
                    &[("gpu", index.as_str()), ("name", gpu.name.as_str())],
                    value,
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;
