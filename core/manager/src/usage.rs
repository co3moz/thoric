//! Resource usage of the running containers: CPU, memory, disk and network once a second for the
//! last 30 minutes, with the host's GPU beside them, kept in memory and never written to disk.
//!
//! Docker reports each container's counters on a stats stream that the collector keeps open for
//! as long as the container runs. Stopped and deleted containers keep their samples until they
//! age out of the window; a deleted one is listed as `removed`, under the name it last had. Each
//! container holds at most 1800 samples of 40 bytes, about 70 KiB, reserved when its first sample
//! arrives; a GPU holds 1800 of 32.
//! Every sample is stamped by one clock that only moves forward, so the newest stamp is a cursor:
//! a client that passes it back as `since` receives exactly what came after.
mod collector;
pub(crate) mod engine;
mod gpu;
mod history;
pub(crate) mod stats;

use crate::{
    App,
    error::{Result, missing},
    model::{CONTAINERS, Container},
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use history::{GpuSample, GpuState, Label, Sample, Series, Store, WINDOW_MS};
use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, MutexGuard};

pub(crate) use collector::spawn;

/// The samples of every container and GPU. Its lock is held only to add a sample or to copy
/// some out, never across an await.
#[derive(Default)]
pub struct Usage(Mutex<Store>);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl Usage {
    fn store(&self) -> MutexGuard<'_, Store> {
        // A panic while a sample was added leaves at most that sample half written.
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    pub(crate) fn record(&self, id: &str, measure: &stats::Measure) {
        self.store().record(id, now_ms(), measure);
    }
    pub(crate) fn record_gpu(&self, reading: &gpu::GpuReading) {
        self.store().record_gpu(now_ms(), reading);
    }
    pub(crate) fn set_error(&self, error: Option<String>) {
        self.store().error = error;
    }
    pub(crate) fn set_gpu(&self, state: GpuState, reason: Option<String>) {
        let mut store = self.store();
        store.gpu_state = state;
        store.gpu_reason = reason;
    }
    /// A container's average CPU use over the last `window`, in cores, or `None` without a
    /// sample in it.
    pub(crate) fn cpu_average(&self, id: &str, window: std::time::Duration) -> Option<f64> {
        let since = now_ms().saturating_sub(window.as_millis() as u64);
        let store = self.store();
        let (count, sum) = store
            .containers
            .get(id)?
            .samples
            .since(since)
            .fold((0u32, 0.0), |(count, sum), sample| {
                (count + 1, sum + f64::from(sample.cpu))
            });
        (count > 0).then(|| sum / f64::from(count))
    }
    /// Forget what aged out, and label each series with the record it still has.
    pub(crate) fn prune(&self, records: &[Container]) {
        self.store().prune(now_ms(), |id| {
            records.iter().find(|record| record.id == id).map(Label::of)
        });
    }

    /// The samples after `since` of the given records, then of the deleted containers whose
    /// samples are still in the window, and of the GPUs, copied out of the lock. `only` limits
    /// the containers to one ID.
    fn snapshot(&self, records: &[Container], only: Option<&str>, since: u64) -> Snapshot {
        let store = self.store();
        let wanted = |id: &str| only.is_none_or(|only| only == id);
        let mut containers: Vec<ContainerSamples> = records
            .iter()
            .filter(|record| wanted(&record.id))
            .filter_map(|record| {
                let series = store.containers.get(&record.id)?;
                Some(ContainerSamples::new(
                    &record.id,
                    Label::of(record),
                    false,
                    series,
                    since,
                ))
            })
            .collect();
        let mut removed: Vec<ContainerSamples> = store
            .containers
            .iter()
            .filter(|(id, _)| wanted(id) && !records.iter().any(|record| &record.id == *id))
            .filter_map(|(id, series)| {
                let label = series.label.clone()?;
                Some(ContainerSamples::new(id, label, true, series, since))
            })
            .collect();
        removed.sort_by(|a, b| a.id.cmp(&b.id));
        containers.append(&mut removed);
        let gpus = store
            .gpus
            .iter()
            .map(|(index, series)| GpuSamples {
                index: *index,
                name: series.name.clone(),
                memory_total: series.memory_total,
                samples: series.samples.since(since).copied().collect(),
            })
            .collect();
        Snapshot {
            cursor: store.cursor(),
            error: store.error.clone(),
            gpu_state: store.gpu_state.clone(),
            gpu_reason: store.gpu_reason.clone(),
            containers,
            gpus,
        }
    }
}

impl Label {
    fn of(record: &Container) -> Self {
        Self {
            name: record.spec.name.clone(),
            cpu_limit: record.spec.cpus,
        }
    }
}

struct ContainerSamples {
    id: String,
    name: String,
    cpu_limit: Option<f64>,
    /// The record is deleted; the samples stay until they age out.
    removed: bool,
    memory_limit: u64,
    online_cpus: u32,
    samples: Vec<Sample>,
}
struct GpuSamples {
    index: u32,
    name: String,
    memory_total: Option<u64>,
    samples: Vec<GpuSample>,
}
struct Snapshot {
    cursor: u64,
    error: Option<String>,
    gpu_state: GpuState,
    gpu_reason: Option<String>,
    containers: Vec<ContainerSamples>,
    gpus: Vec<GpuSamples>,
}

impl ContainerSamples {
    fn new(id: &str, label: Label, removed: bool, series: &Series, since: u64) -> Self {
        Self {
            id: id.to_owned(),
            name: label.name,
            cpu_limit: label.cpu_limit,
            removed,
            memory_limit: series.memory_limit,
            online_cpus: series.online_cpus,
            samples: series.samples.since(since).copied().collect(),
        }
    }
    fn json(&self) -> Value {
        json!({"id": self.id, "name": self.name, "cpu_limit": self.cpu_limit, "removed": self.removed, "memory_limit": self.memory_limit, "online_cpus": self.online_cpus, "samples": self.samples})
    }
}
impl Snapshot {
    fn header(&self) -> Value {
        json!({"available": self.error.is_none(), "error": self.error, "interval_ms": 1000, "window_ms": WINDOW_MS, "cursor": self.cursor})
    }
    fn gpu(&self, devices: Vec<Value>) -> Value {
        json!({"state": self.gpu_state, "reason": self.gpu_reason, "scope": "host", "devices": devices})
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct Since {
    /// The `cursor` of an earlier response; only samples stamped after it are returned.
    #[serde(default)]
    pub since: u64,
}

/// The record `id` names, or `None` once it is deleted: a deleted container's samples are still
/// served while they are in the window.
fn record(app: &App, id: &str) -> Result<Option<Container>> {
    match app.container(id) {
        Ok(record) => Ok(Some(record)),
        Err(error) if error.0 == StatusCode::NOT_FOUND => Ok(None),
        Err(error) => Err(error),
    }
}

/// Every container's samples, deleted ones included, and the host's GPU.
pub async fn all(State(app): State<Arc<App>>, Query(query): Query<Since>) -> Result<Json<Value>> {
    let records: Vec<Container> = app.db.list(CONTAINERS)?;
    let snapshot = app.usage.snapshot(&records, None, query.since);
    let mut body = snapshot.header();
    body["containers"] = snapshot
        .containers
        .iter()
        .map(ContainerSamples::json)
        .collect();
    let devices = snapshot
        .gpus
        .iter()
        .map(|gpu| json!({"index": gpu.index, "name": gpu.name, "memory_total": gpu.memory_total, "samples": gpu.samples}))
        .collect();
    body["gpu"] = snapshot.gpu(devices);
    Ok(Json(body))
}

/// One container's samples; empty until it has run since the manager started, and still there
/// after it is deleted until they age out.
pub async fn container(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Query(query): Query<Since>,
) -> Result<Json<Value>> {
    let record = record(&app, &id)?;
    let snapshot = app
        .usage
        .snapshot(record.as_slice(), Some(&id), query.since);
    let mut body = snapshot.header();
    let series = match (snapshot.containers.first(), &record) {
        (Some(series), _) => series.json(),
        (None, Some(record)) => {
            json!({"id": record.id, "name": record.spec.name, "cpu_limit": record.spec.cpus, "removed": false, "memory_limit": null, "online_cpus": null, "samples": []})
        }
        (None, None) => return Err(missing()),
    };
    for (key, value) in series.as_object().into_iter().flatten() {
        body[key] = value.clone();
    }
    Ok(Json(body))
}

/// The options of the `get_usage` tool.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    /// How many of the last seconds to summarize, from 1 to 1800; 60 when left out.
    #[schemars(range(min = 1, max = 1800))]
    pub seconds: Option<u64>,
}

/// Latest, average and peak of each figure over the last seconds, for agents that need the
/// numbers rather than a chart.
pub(crate) fn summary(app: &App, id: Option<&str>, options: &Summary) -> Result<Value> {
    let seconds = options.seconds.unwrap_or(60);
    if !(1..=1800).contains(&seconds) {
        return Err(anyhow::anyhow!("seconds must be between 1 and 1800").into());
    }
    let records: Vec<Container> = match id {
        Some(id) => record(app, id)?.into_iter().collect(),
        None => app.db.list(CONTAINERS)?,
    };
    let since = now_ms().saturating_sub(seconds * 1000);
    let snapshot = app.usage.snapshot(&records, id, since);
    if id.is_some() && records.is_empty() && snapshot.containers.is_empty() {
        return Err(missing());
    }
    let containers: Vec<Value> = snapshot
        .containers
        .iter()
        .filter(|container| !container.samples.is_empty())
        .map(|container| {
            let figure = |value: fn(&Sample) -> f64, places: i32| {
                aggregate(container.samples.iter().map(value), places)
            };
            json!({
                "id": container.id,
                "name": container.name,
                "removed": container.removed,
                "samples": container.samples.len(),
                "cpu_limit": container.cpu_limit,
                "memory_limit_bytes": container.memory_limit,
                "cpu_cores": figure(|s| f64::from(s.cpu), 3),
                "memory_bytes": figure(|s| s.memory as f64, 0),
                "disk_read_bytes_per_second": figure(|s| f64::from(s.disk_read), 0),
                "disk_write_bytes_per_second": figure(|s| f64::from(s.disk_write), 0),
                "network_rx_bytes_per_second": figure(|s| f64::from(s.net_rx), 0),
                "network_tx_bytes_per_second": figure(|s| f64::from(s.net_tx), 0),
            })
        })
        .collect();
    let devices = snapshot
        .gpus
        .iter()
        .filter(|gpu| !gpu.samples.is_empty())
        .map(|gpu| {
            json!({
                "index": gpu.index,
                "name": gpu.name,
                "samples": gpu.samples.len(),
                "memory_total_bytes": gpu.memory_total,
                "utilization_percent": aggregate(gpu.samples.iter().filter_map(|s| s.utilization.map(f64::from)), 1),
                "memory_bytes": aggregate(gpu.samples.iter().filter_map(|s| s.memory.map(|m| m as f64)), 0),
            })
        })
        .collect();
    let mut body = snapshot.header();
    body["seconds"] = json!(seconds);
    body["containers"] = json!(containers);
    body["gpu"] = snapshot.gpu(devices);
    Ok(body)
}

/// The latest, average and peak of a series, rounded to `places` decimals; null when empty.
pub(crate) fn aggregate(values: impl Iterator<Item = f64>, places: i32) -> Value {
    let (mut count, mut sum, mut peak, mut latest) = (0u32, 0.0, f64::MIN, 0.0);
    for value in values {
        count += 1;
        sum += value;
        peak = peak.max(value);
        latest = value;
    }
    if count == 0 {
        return Value::Null;
    }
    let scale = 10f64.powi(places);
    let round = |value: f64| {
        if places > 0 {
            json!((value * scale).round() / scale)
        } else {
            json!(value.round() as i64)
        }
    };
    json!({"latest": round(latest), "average": round(sum / f64::from(count)), "peak": round(peak)})
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
