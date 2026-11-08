//! Following what the desktops declare: one helper per running desktop prints the desktop user's
//! declarations whenever they change, and says when the image's folder changed, which the manager
//! then reads itself as root. The manager reads the tools of a new or changed server once. The
//! same helper hears the desktop's reports.
use super::{
    client,
    endpoints::announce,
    placement::{Placement, placement},
    registry::{Found, SERVERS, ServerRecord, Source, found, merged, watch_args},
    report,
    servers::records,
};
use crate::{App, model::now};
use rmcp::{RoleClient, service::Peer};
use serde_json::Value;
use std::{
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::io::BufReader;

/// How often the manager looks for desktops that started or stopped.
const SUPERVISE_EVERY: Duration = Duration::from_secs(10);
/// The longest listing a desktop's helper prints: 64 declarations of at most 64 KiB, each at most
/// three times as long once escaped as JSON, and what the image's folder holds. The desktop can
/// write a longer line, which is dropped.
const LISTING_LIMIT: usize = 32 << 20;
/// The image's folder is read as root at most this often, whatever the desktop's helper says.
const IMAGE_READ_EVERY: Duration = Duration::from_secs(5);

/// The image's declarations as the manager last read them as root, and what the desktop's helper
/// said the folder held then.
#[derive(Default)]
struct ImageServers {
    servers: Option<Vec<Found>>,
    seen: Value,
    read_at: Option<Instant>,
    /// Why the last read failed, so a failure is logged when it begins or its reason changes.
    failed: Option<String>,
}
impl ImageServers {
    /// The image's servers for a listing of the desktop's helper, read again as root when the
    /// helper says the folder changed, and on every listing until a read succeeds: the last ones
    /// read when a read fails, and none before the first read.
    async fn for_listing(
        &mut self,
        app: &App,
        record_id: &str,
        docker_id: &str,
        listing: &Value,
    ) -> Option<Vec<Found>> {
        if self.servers.is_none() || listing["image"] != self.seen {
            if let Some(at) = self.read_at {
                tokio::time::sleep(IMAGE_READ_EVERY.saturating_sub(at.elapsed())).await;
            }
            self.read_at = Some(Instant::now());
            match app.read_image_servers(docker_id).await {
                Ok(servers) => {
                    self.servers = Some(servers);
                    self.seen = listing["image"].clone();
                    if self.failed.take().is_some() {
                        tracing::info!("The image's MCP servers of {record_id} were read again");
                    }
                }
                Err(error) => {
                    let error = format!("{error:#}");
                    if self.failed.as_ref() != Some(&error) {
                        tracing::warn!(
                            "Could not read the image's MCP servers of {record_id} as root; their records stay as they are: {error}"
                        );
                    }
                    self.failed = Some(error);
                }
            }
        }
        self.servers.clone()
    }
}

/// Watch every running desktop and close idle server connections. The MCP endpoints, which
/// answer whether or not the root `/mcp` is on, and the MCP page need the servers' tool lists.
pub(crate) fn spawn(app: Arc<App>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SUPERVISE_EVERY);
        loop {
            tick.tick().await;
            if let Err(error) = supervise(&app).await {
                tracing::debug!("Desktop MCP servers: {error:#}");
            }
            app.desktop_mcp.pool.reap();
        }
    });
}

/// Start a watcher for every running managed container that has none.
async fn supervise(app: &Arc<App>) -> anyhow::Result<()> {
    let output = app
        .docker(&[
            "ps",
            "--filter",
            &format!("label={}=true", crate::model::LABEL),
            "--format",
            "{{.ID}} {{.Label \"thoric.environment\"}}",
        ])
        .await?;
    for line in String::from_utf8_lossy(&output).lines() {
        let Some((docker_id, record_id)) = line.split_once(' ') else {
            continue;
        };
        if record_id.is_empty() || app.container(record_id).is_err() {
            continue;
        }
        if app
            .desktop_mcp
            .watchers
            .lock()
            .unwrap()
            .insert(record_id.to_owned())
        {
            tokio::spawn(watcher(
                app.clone(),
                record_id.to_owned(),
                docker_id.to_owned(),
            ));
        }
    }
    Ok(())
}

/// Read a desktop's declarations as they change, and tell it what the manager knows of its
/// servers, until the desktop stops.
async fn watcher(app: Arc<App>, record_id: String, docker_id: String) {
    let child = app
        .command(&watch_args(&docker_id))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        // stdin carries the reports and stays open: the helper ends when it closes, with this
        // task.
        let reporter = child
            .stdin
            .take()
            .map(|stdin| tokio::spawn(report::send_reports(app.clone(), record_id.clone(), stdin)));
        if let Some(stdout) = child.stdout.take() {
            let mut stdout = BufReader::new(stdout);
            let mut line = Vec::new();
            let mut image = ImageServers::default();
            while let Ok(Some(cut)) = client::read_line(&mut stdout, &mut line, LISTING_LIMIT).await
            {
                if cut {
                    tracing::debug!(
                        "Desktop MCP servers of {record_id}: a listing longer than {} MiB",
                        LISTING_LIMIT >> 20
                    );
                    continue;
                }
                // The helper runs as the desktop user, so its servers are that user's whatever it
                // says.
                let listing = match serde_json::from_slice::<Value>(&line) {
                    Ok(listing) => listing,
                    Err(error) => {
                        tracing::debug!("Desktop MCP servers of {record_id}: {error}");
                        continue;
                    }
                };
                let user = match found(&listing, Source::Container) {
                    Ok(user) => user,
                    Err(error) => {
                        tracing::debug!("Desktop MCP servers of {record_id}: {error:#}");
                        continue;
                    }
                };
                match image
                    .for_listing(&app, &record_id, &docker_id, &listing)
                    .await
                {
                    Some(servers) => apply(&app, &record_id, merged(servers, user)).await,
                    // Before the image's folder was ever read, the desktop user's servers still
                    // count, and the image's records stay as they are.
                    None => apply_with(&app, &record_id, user, false).await,
                }
            }
        }
        if let Some(reporter) = reporter {
            reporter.abort();
        }
        let _ = child.kill().await;
    }
    app.desktop_mcp.watchers.lock().unwrap().remove(&record_id);
    // The tools stay listed while the container is stopped; its connections ended with it.
    app.desktop_mcp
        .pool
        .retain(|(_, container, _)| *container != record_id);
    announce(&app, &record_id);
}

/// Bring the records of a container in line with what its desktop declares. A new or changed
/// declaration has its tools read in the background.
pub(super) async fn apply(app: &Arc<App>, container_id: &str, found: Vec<Found>) {
    apply_with(app, container_id, found, true).await;
}

/// [`apply`], where `image_known` says whether `found` holds the image's servers too. Without them
/// the records of the image's servers stay, unless the desktop user declares the same name.
pub(super) async fn apply_with(
    app: &Arc<App>,
    container_id: &str,
    found: Vec<Found>,
    image_known: bool,
) {
    if app.container(container_id).is_err() {
        return;
    }
    let existing = records(app, container_id);
    let sets = crate::scale::all(app).unwrap_or_default();
    let mut changed = false;
    let mut pending = Vec::new();
    for server in &found {
        let old = existing.iter().find(|record| record.name == server.name);
        let record = match &server.spec {
            Err(error) => ServerRecord {
                container_id: container_id.into(),
                name: server.name.clone(),
                source: server.source,
                spec: None,
                tools: Vec::new(),
                tools_at: None,
                error: Some(error.clone()),
            },
            Ok(spec) => {
                if let Some(old) = old
                    && old.spec.as_ref() == Some(spec)
                    && old.source == server.source
                    && old.error.is_none()
                {
                    if old.tools_at.is_none() {
                        pending.push(server.name.clone());
                    }
                    continue;
                }
                pending.push(server.name.clone());
                ServerRecord {
                    container_id: container_id.into(),
                    name: server.name.clone(),
                    source: server.source,
                    spec: Some(spec.clone()),
                    tools: Vec::new(),
                    tools_at: None,
                    error: None,
                }
            }
        };
        if let Some(old) = old
            && old.spec == record.spec
            && old.source == record.source
            && old.error == record.error
        {
            continue;
        }
        // A changed declaration ends the connections that run the old one.
        app.desktop_mcp
            .pool
            .retain(|(_, container, name)| !(container == container_id && *name == server.name));
        let _ = app.db.put(
            SERVERS,
            &ServerRecord::key(container_id, &server.name),
            &record,
        );
        changed = true;
    }
    for old in existing.iter().filter(|old| {
        !found.iter().any(|server| server.name == old.name)
            && (image_known || old.source != Source::Image)
    }) {
        let _ = app
            .db
            .remove(SERVERS, &ServerRecord::key(container_id, &old.name));
        app.desktop_mcp
            .pool
            .retain(|(_, container, name)| !(container == container_id && *name == old.name));
        changed = true;
    }
    if changed {
        announce(app, container_id);
    }
    // A server that another instance of the set runs for this one is never started here, not
    // even to read its tools: its record stays without them.
    pending.retain(|name| Placement::of(&sets, container_id, name).home().is_none());
    for name in pending {
        let app = app.clone();
        let container_id = container_id.to_owned();
        tokio::spawn(async move { fetch(&app, &container_id, &name).await });
    }
}

/// Start a server once and keep the tools it lists, or why it could not list them. A server
/// already being read is left to that, and one another instance of its set runs is not read here.
pub(super) async fn fetch(app: &Arc<App>, container_id: &str, name: &str) {
    if placement(app, container_id, name).home().is_some() {
        return;
    }
    let key = ServerRecord::key(container_id, name);
    if !app.desktop_mcp.fetching.lock().unwrap().insert(key.clone()) {
        return;
    }
    let result = async {
        let container = app
            .container(container_id)
            .map_err(|e| anyhow::anyhow!(e.1))?;
        let record: ServerRecord = app.db.get(SERVERS, &key)?;
        let tools = client::read_tools(app, &container, &record).await;
        anyhow::Ok((record, tools))
    }
    .await;
    app.desktop_mcp.fetching.lock().unwrap().remove(&key);
    let Ok((read, tools)) = result else {
        return;
    };
    // The declaration may have changed while it was read; that read is the one that counts.
    let Ok(mut record) = app.db.get::<ServerRecord>(SERVERS, &key) else {
        return;
    };
    if record.spec != read.spec {
        return;
    }
    keep_tools(&mut record, tools);
    let _ = app.db.put(SERVERS, &key, &record);
    announce(app, container_id);
}

/// What a server's record keeps of a read of its tools: the list, or why there is none.
fn keep_tools(record: &mut ServerRecord, tools: anyhow::Result<Vec<rmcp::model::Tool>>) {
    match tools {
        Ok(tools) => {
            record.tools = tools;
            record.tools_at = Some(now());
            record.error = None;
        }
        Err(error) => {
            record.tools.clear();
            record.tools_at = None;
            record.error = Some(format!("{error:#}"));
        }
    }
}

/// How long a server's tools are left alone after it said they changed, so a burst of
/// notifications is one read.
const REREAD_AFTER: Duration = Duration::from_secs(1);

/// A server said its tools changed: read them through the connection that heard it. One read
/// runs per server; a notification during it asks for one more read, never for one each.
pub(super) fn reread_tools(
    app: Arc<App>,
    container_id: &str,
    server: &str,
    peer: Peer<RoleClient>,
) {
    let key = ServerRecord::key(container_id, server);
    {
        let mut rereads = app.desktop_mcp.rereads.lock().unwrap();
        if let Some(again) = rereads.get_mut(&key) {
            *again = true;
            return;
        }
        rereads.insert(key.clone(), false);
    }
    let container_id = container_id.to_owned();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(REREAD_AFTER).await;
            let tools = match tokio::time::timeout(
                Duration::from_secs(60),
                client::list_tools(&peer),
            )
            .await
            {
                Ok(tools) => tools,
                Err(_) => Err(anyhow::anyhow!(
                    "The server did not list its tools in a minute"
                )),
            };
            if let Ok(mut record) = app.db.get::<ServerRecord>(SERVERS, &key) {
                keep_tools(&mut record, tools);
                let _ = app.db.put(SERVERS, &key, &record);
                announce(&app, &container_id);
            }
            let mut rereads = app.desktop_mcp.rereads.lock().unwrap();
            match rereads.get_mut(&key) {
                Some(again) if *again => *again = false,
                _ => {
                    rereads.remove(&key);
                    return;
                }
            }
        }
    });
}

impl super::DesktopMcp {
    /// Whether the declarations of a container are being watched, which they are while it runs.
    pub(super) fn watched(&self, container_id: &str) -> bool {
        self.watchers.lock().unwrap().contains(container_id)
    }
}
