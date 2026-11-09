//! What a running desktop is told about its MCP servers: their state, tools and errors, and the
//! endpoints that reach it. The watcher sends a report on its helper's stdin whenever one changes,
//! and the helper keeps the latest in the desktop's runtime folder, where `thoric-mcp list
//! --json` and the tray read it. A report names no token and no other container.
use super::{
    endpoints::{ENDPOINTS, Endpoint},
    placement::shown,
    registry::ServerRecord,
    servers::records,
};
use crate::{App, model::now};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncWrite, AsyncWriteExt};

/// The longest tool description a report carries; the dashboard shows the whole one.
const DESCRIPTION_LIMIT: usize = 200;
/// How much of a server's stderr a report carries.
const LOG_LIMIT: usize = 2048;
/// How long a reporter waits after a change, so a burst of changes is one report.
const SETTLE: Duration = Duration::from_millis(250);

/// The report of one container, without the time it was made: its servers as the manager knows
/// them, and the endpoints that cover it with this container's grant alone.
pub(super) fn report(app: &App, container_id: &str) -> Value {
    let sets = crate::scale::all(app).unwrap_or_default();
    let servers = records(app, container_id)
        .iter()
        .map(|record| {
            // A server another instance of the set runs for this one is `elsewhere`, and
            // `runs_in` names that instance.
            let (state, runs_in) = shown(record, true, &sets);
            json!({
                "name": record.name,
                "source": record.source,
                "state": state,
                "runs_in": runs_in,
                "error": record.error,
                "tools": record.tools.iter().map(|tool| json!({
                    "name": tool.name,
                    "description": trimmed(tool.description.as_deref().unwrap_or_default()),
                })).collect::<Vec<_>>(),
                "tools_at": record.tools_at,
                "log": app.desktop_mcp.log_tail(&ServerRecord::key(container_id, &record.name), LOG_LIMIT),
            })
        })
        .collect::<Vec<_>>();
    let mut endpoints: Vec<Endpoint> = app.db.list(ENDPOINTS).unwrap_or_default();
    endpoints.sort_by(|a, b| a.name.cmp(&b.name));
    let endpoints = endpoints
        .iter()
        .filter_map(|endpoint| {
            let grant = endpoint
                .grants
                .iter()
                .find(|grant| grant.covers(&sets, container_id))?;
            // On an endpoint of a scale set, the desktop is the instance of this number.
            let instance = grant.set.as_ref().and_then(|set| {
                sets.iter()
                    .find(|known| known.name == *set)
                    .and_then(|known| known.number_of(container_id))
            });
            let mut entry = json!({
                "name": endpoint.name,
                "path": format!("/mcp/{}", endpoint.name),
                "automatic": endpoint.automatic,
                "has_token": endpoint.token_hash.is_some(),
                "alias": grant.alias,
                "scopes": grant.scopes,
                "profile": grant.profile,
                "servers": grant.servers,
                "excluded_tools": grant.excluded_tools,
            });
            if let Some(set) = &grant.set {
                entry["set"] = json!(set);
                entry["instance"] = json!(instance);
            }
            Some(entry)
        })
        .collect::<Vec<_>>();
    json!({"servers": servers, "endpoints": endpoints})
}

/// The first line of a description, at most [`DESCRIPTION_LIMIT`] bytes.
fn trimmed(description: &str) -> String {
    let line = description.trim().lines().next().unwrap_or_default();
    if line.len() <= DESCRIPTION_LIMIT {
        return line.to_owned();
    }
    let mut end = DESCRIPTION_LIMIT;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &line[..end])
}

/// Send a container's report to its watcher's helper now and whenever it changes, until the
/// helper stops reading.
pub(super) async fn send_reports(
    app: Arc<App>,
    container_id: String,
    mut helper: impl AsyncWrite + Unpin,
) {
    let mut changes = app.desktop_mcp.reports.subscribe();
    let mut last = None;
    loop {
        let current = report(&app, &container_id);
        if last.as_ref() != Some(&current) {
            let mut status = current.clone();
            status["status_at"] = json!(now());
            let mut line = serde_json::to_vec(&json!({ "status": status })).unwrap_or_default();
            line.push(b'\n');
            if helper.write_all(&line).await.is_err() || helper.flush().await.is_err() {
                return;
            }
            last = Some(current);
        }
        if changes.changed().await.is_err() {
            return;
        }
        tokio::time::sleep(SETTLE).await;
        // The next report covers what changed meanwhile too.
        changes.borrow_and_update();
    }
}

impl super::DesktopMcp {
    /// Wake the reporters of the running desktops: a server or an endpoint changed.
    pub(super) fn report_changes(&self) {
        self.reports.send_replace(());
    }
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
