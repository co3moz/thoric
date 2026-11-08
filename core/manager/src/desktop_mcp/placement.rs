//! Where a desktop server of a scale set's instance runs. Every instance runs its own, unless the
//! set's `servers` say `first`: such a server holds what only one copy may hold, such as a project
//! a shared volume keeps with a lock file, so instance 1 alone runs it, for every instance. There
//! it runs in one process that every endpoint, the administrator's `/mcp` and the read of its tools
//! share, so two of them never open the project at once. The other instances never start it, not
//! even to read its tools, and a call that names one of them is refused with the instance to use.
use super::{
    endpoints::announce,
    registry::{SERVERS, ServerRecord},
    watch,
};
use crate::{
    App,
    scale::{ScaleSet, ServerPolicy},
};
use axum::http::StatusCode;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// The pool's name for the one connection to a server that runs once for a whole set, which no
/// endpoint's name can be.
pub(super) const SHARED: &str = "*";

/// Where a server of a container runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Placement {
    /// In the container, for it alone: a container of no set, or a server every instance runs.
    Own,
    /// In the container, the instance of `set` that runs it for all of them.
    Home { set: String },
    /// In instance `home` of `set`, never in this container.
    Elsewhere { set: String, home: u32 },
}

impl Placement {
    /// Where the container `container_id` runs `server`, by the sets it may be an instance of.
    pub fn of(sets: &[ScaleSet], container_id: &str, server: &str) -> Self {
        let Some((set, number)) = crate::scale::instance_of(sets, container_id) else {
            return Self::Own;
        };
        match set.runs_in(server) {
            None => Self::Own,
            Some(home) if home == number => Self::Home {
                set: set.name.clone(),
            },
            Some(home) => Self::Elsewhere {
                set: set.name.clone(),
                home,
            },
        }
    }
    /// The name the pool keeps a connection of `endpoint` to this server under: the endpoint's
    /// own, or the one every endpoint shares for a server that runs once for its set.
    pub fn connection<'a>(&self, endpoint: &'a str) -> &'a str {
        match self {
            Self::Home { .. } => SHARED,
            _ => endpoint,
        }
    }
    /// The instance that runs the server instead of this container, when another does.
    pub fn home(&self) -> Option<u32> {
        match self {
            Self::Elsewhere { home, .. } => Some(*home),
            _ => None,
        }
    }
    /// Why the server is not started in this container, when it runs in another instance.
    pub fn refusal(&self, server: &str) -> Option<anyhow::Error> {
        let Self::Elsewhere { set, home } = self else {
            return None;
        };
        Some(crate::error::Fault::error(
            StatusCode::CONFLICT,
            format!(
                "The scale set {set} runs the desktop MCP server {server} in instance {home} alone, for every instance; use it there"
            ),
        ))
    }
}

/// A server's state as lists and the desktop show it, with the instance that runs it instead of
/// its container: `elsewhere` for a server another instance of its set runs, and the record's own
/// state otherwise, also for a declaration that does not read.
pub(super) fn shown(
    record: &ServerRecord,
    running: bool,
    sets: &[ScaleSet],
) -> (&'static str, Option<u32>) {
    let home = Placement::of(sets, &record.container_id, &record.name).home();
    match home {
        Some(_) if record.error.is_none() => ("elsewhere", home),
        _ => (record.state(running), home),
    }
}

/// Where the container `container_id` runs `server`.
pub(super) fn placement(app: &App, container_id: &str, server: &str) -> Placement {
    Placement::of(
        &crate::scale::all(app).unwrap_or_default(),
        container_id,
        server,
    )
}

/// The servers whose policy differs between `before` and the set's.
fn moved(before: &BTreeMap<String, ServerPolicy>, set: &ScaleSet) -> BTreeSet<String> {
    before
        .keys()
        .chain(set.servers.keys())
        .filter(|name| before.get(*name).copied().unwrap_or_default() != set.server_policy(name))
        .cloned()
        .collect()
}

/// The set's server policies changed from `before`: what ran under the old ones ends, the
/// instances that no longer run a server forget its tools, and those that run it again read them
/// once their desktop runs. The agents and desktops of every instance hear of it.
pub(crate) fn servers_changed(
    app: &Arc<App>,
    before: &BTreeMap<String, ServerPolicy>,
    set: &ScaleSet,
) {
    let moved = moved(before, set);
    if moved.is_empty() {
        return;
    }
    for (number, instance) in &set.instances {
        let id = instance.container_id.as_str();
        // A connection kept under an endpoint's name becomes the shared one, or the other way
        // round, and one in an instance that no longer runs the server ends with its process.
        app.desktop_mcp
            .pool
            .retain(|(_, container, server)| !(container == id && moved.contains(server)));
        for name in &moved {
            let key = ServerRecord::key(id, name);
            let Ok(mut record) = app.db.get::<ServerRecord>(SERVERS, &key) else {
                continue;
            };
            match set.runs_in(name) {
                Some(home) if home != *number => {
                    // Its tools are the home instance's; a failed read here no longer matters.
                    record.tools.clear();
                    record.tools_at = None;
                    if record.spec.is_some() {
                        record.error = None;
                    }
                    let _ = app.db.put(SERVERS, &key, &record);
                }
                _ => {
                    if record.spec.is_some()
                        && record.tools_at.is_none()
                        && app.desktop_mcp.watched(id)
                    {
                        let app = app.clone();
                        let (id, name) = (id.to_owned(), name.clone());
                        tokio::spawn(async move { watch::fetch(&app, &id, &name).await });
                    }
                }
            }
        }
        announce(app, id);
    }
}

#[cfg(test)]
#[path = "placement_tests.rs"]
mod tests;
