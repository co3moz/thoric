//! What Docker last said of each container, for `/metrics`: the state the collector's listing found
//! each record's container in and, of each running container, when it started and the counters of
//! its latest stats message. The collector writes it as it lists the containers and reads their
//! streams; nothing here asks Docker.
use super::{Usage, engine, history::GpuState, stats::Counters};
use std::{collections::BTreeMap, sync::MutexGuard};

/// The host GPU's states as `/api/usage` names them, in this order.
pub(crate) const GPU_STATES: [&str; 4] = ["disabled", "idle", "sampling", "unavailable"];

/// What the collector's latest listing found.
#[derive(Debug, Default)]
pub(crate) struct Listing {
    /// Docker's state of each record's container, by record ID, or `None` before the first
    /// listing and while Docker cannot be listed.
    states: Option<BTreeMap<String, String>>,
    /// The running containers, by record ID.
    running: BTreeMap<String, Live>,
}

#[derive(Debug)]
struct Live {
    docker_id: String,
    started: Option<f64>,
    counters: Option<Counters>,
}

/// A running container as `/metrics` reports it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Running {
    pub id: String,
    /// When Docker started it, in Unix seconds, once its inspect answered.
    pub started: Option<f64>,
    /// Its latest stats message, once one arrived.
    pub counters: Option<Counters>,
}

/// One of the host's GPUs as its latest sample measured it, the whole adapter's use and never a
/// container's share.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Gpu {
    pub index: u32,
    pub name: String,
    /// In percent.
    pub utilization: Option<f32>,
    pub memory_used: Option<u64>,
    pub memory_total: Option<u64>,
}

/// What `/metrics` reads of the collector, copied out of its locks.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Current {
    pub states: Option<BTreeMap<String, String>>,
    pub running: Vec<Running>,
    /// Whether the collector follows Docker: false while it cannot list the containers.
    pub available: bool,
    /// One of [`GPU_STATES`].
    pub gpu_state: &'static str,
    /// The host's GPUs, only while they are sampled.
    pub gpus: Vec<Gpu>,
}

impl Usage {
    fn listing(&self) -> MutexGuard<'_, Listing> {
        // A panic while the listing changed leaves at most one container half written.
        self.listing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// What a listing found: each record's state, and the containers that run. One that runs
    /// under another Docker ID than before, such as after a reset, starts again with nothing known.
    pub(crate) fn listed(
        &self,
        states: BTreeMap<String, String>,
        running: &BTreeMap<String, engine::Running>,
    ) {
        let mut listing = self.listing();
        listing.states = Some(states);
        listing.running.retain(|id, live| {
            running
                .get(id)
                .is_some_and(|now| now.docker_id == live.docker_id)
        });
        for (id, now) in running {
            listing.running.entry(id.clone()).or_insert_with(|| Live {
                docker_id: now.docker_id.clone(),
                started: None,
                counters: None,
            });
        }
    }

    /// The containers could not be listed: their states are unknown until they can be. What the
    /// streams learnt stays for the containers the next listing finds under the same Docker ID,
    /// whose streams go on and do not ask for their start again.
    pub(crate) fn unlisted(&self) {
        self.listing().states = None;
    }

    /// When the container `id`, running as `docker_id`, started, in Unix seconds.
    pub(crate) fn started(&self, id: &str, docker_id: &str, at: f64) {
        let mut listing = self.listing();
        if let Some(live) = listing.running.get_mut(id)
            && live.docker_id == docker_id
        {
            live.started = Some(at);
        }
    }

    /// The counters of the latest stats message of the container `id`, running as `docker_id`.
    pub(crate) fn counted(&self, id: &str, docker_id: &str, counters: Counters) {
        let mut listing = self.listing();
        if let Some(live) = listing.running.get_mut(id)
            && live.docker_id == docker_id
        {
            live.counters = Some(counters);
        }
    }

    /// The listing and the host's GPUs as they are now.
    pub(crate) fn current(&self) -> Current {
        let (states, running) = {
            let listing = self.listing();
            let running = listing
                .running
                .iter()
                .map(|(id, live)| Running {
                    id: id.clone(),
                    started: live.started,
                    counters: live.counters,
                })
                .collect();
            (listing.states.clone(), running)
        };
        let store = self.store();
        let gpu_state = match store.gpu_state {
            GpuState::Disabled => GPU_STATES[0],
            GpuState::Idle => GPU_STATES[1],
            GpuState::Sampling => GPU_STATES[2],
            GpuState::Unavailable => GPU_STATES[3],
        };
        // A GPU that is no longer sampled keeps samples for the charts, which are no reading now.
        let gpus = if store.gpu_state == GpuState::Sampling {
            store
                .gpus
                .iter()
                .filter_map(|(index, series)| {
                    let latest = series.samples.since(0).last()?;
                    Some(Gpu {
                        index: *index,
                        name: series.name.clone(),
                        utilization: latest.utilization,
                        memory_used: latest.memory,
                        memory_total: series.memory_total,
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        Current {
            states,
            running,
            available: store.error.is_none(),
            gpu_state,
            gpus,
        }
    }
}

#[cfg(test)]
#[path = "current_tests.rs"]
mod tests;
