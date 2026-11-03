//! When each container was last used, in memory only: what uses it says so, with [`Activity::touch`]
//! for a moment or with the [`InUse`] that [`Activity::open`] returns for as long as it lasts.
//! A container nothing has used since the manager started counts from the start.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};
use tokio::time::Instant;

/// The last use of every container used since the manager started. Its lock is held only to read
/// or note a use, never across an await.
#[derive(Clone)]
pub(crate) struct Activity(Arc<Mutex<Uses>>);

struct Uses {
    /// When the manager started, the last use of a container nothing has used since.
    since: Instant,
    containers: HashMap<String, Use>,
}

struct Use {
    last: Instant,
    /// How many uses are going on, each an [`InUse`].
    open: usize,
}

impl Default for Activity {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Uses {
            since: Instant::now(),
            containers: HashMap::new(),
        })))
    }
}

impl Activity {
    /// A manager that started `ago`, as tests that run real commands need it, since a paused clock
    /// would also run out their time limits.
    #[cfg(test)]
    pub(crate) fn started(ago: Duration) -> Self {
        let activity = Self::default();
        let since = Instant::now()
            .checked_sub(ago)
            .expect("the clock goes back");
        activity.uses().since = since;
        activity
    }

    fn uses(&self) -> MutexGuard<'_, Uses> {
        // Nothing is half written while it is held.
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn noted(&self, id: &str, opened: usize) {
        let now = Instant::now();
        let mut uses = self.uses();
        let entry = uses
            .containers
            .entry(id.to_owned())
            .or_insert(Use { last: now, open: 0 });
        entry.last = now;
        entry.open += opened;
    }

    /// The container was used just now.
    pub(crate) fn touch(&self, id: &str) {
        self.noted(id, 0);
    }

    /// The container is used until the returned guard drops, which counts as its last use.
    pub(crate) fn open(&self, id: &str) -> InUse {
        self.noted(id, 1);
        InUse {
            activity: self.clone(),
            id: id.to_owned(),
        }
    }

    /// How long the container has gone unused, `None` while something uses it.
    pub(crate) fn idle_for(&self, id: &str) -> Option<Duration> {
        let uses = self.uses();
        match uses.containers.get(id) {
            Some(entry) if entry.open > 0 => None,
            Some(entry) => Some(entry.last.elapsed()),
            None => Some(uses.since.elapsed()),
        }
    }

    /// Forget the containers `keep` refuses, such as deleted ones, unless something uses them.
    pub(crate) fn retain(&self, mut keep: impl FnMut(&str) -> bool) {
        self.uses()
            .containers
            .retain(|id, entry| entry.open > 0 || keep(id));
    }
}

/// A use of a container that lasts until this drops, such as a job on it or an open VNC viewer.
pub(crate) struct InUse {
    activity: Activity,
    id: String,
}

impl Drop for InUse {
    fn drop(&mut self) {
        let now = Instant::now();
        if let Some(entry) = self.activity.uses().containers.get_mut(&self.id) {
            entry.open = entry.open.saturating_sub(1);
            entry.last = now;
        }
    }
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod tests;
