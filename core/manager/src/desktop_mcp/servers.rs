//! The dashboard's and the administrator's view of desktop servers: listing them, registering
//! one in a running desktop, removing it and reading its tools again, and calling one of its tools
//! for the administrator's channel.
use super::{
    placement::shown,
    registry::{Declaration, SERVERS, ServerRecord, Source, server_name},
    scopes::server_tool_names,
    watch,
};
use crate::{
    App,
    error::{Error, Result, missing},
    model::Container,
    scale::ScaleSet,
};
use anyhow::Context;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

/// A server as lists show it: never the values of its variables or headers. Each tool comes with
/// `exposed`, its name on an endpoint of this container alone, which an endpoint's deny list
/// uses. `runs_in` is the instance of the container's scale set that runs the server for it.
pub(crate) fn summary(
    app: &App,
    record: &ServerRecord,
    container: &str,
    running: bool,
    sets: &[ScaleSet],
) -> Value {
    let mut value = match &record.spec {
        Some(spec) => spec.summary(),
        None => {
            json!({"description":"","transport":"stdio","command":null,"args":[],"cwd":null,"env":[],"url":null,"headers":[]})
        }
    };
    value["container_id"] = json!(record.container_id);
    value["container_name"] = json!(container);
    value["name"] = json!(record.name);
    value["source"] = json!(record.source);
    let (state, runs_in) = shown(record, running, sets);
    value["state"] = json!(state);
    value["runs_in"] = json!(runs_in);
    value["error"] = json!(record.error);
    value["tools"] = json!(
        server_tool_names(record)
            .iter()
            .map(|named| json!({
                "name": named.tool.name,
                "description": named.tool.description.as_deref().unwrap_or_default(),
                "exposed": named.name,
            }))
            .collect::<Vec<_>>()
    );
    value["tools_at"] = json!(record.tools_at);
    value["log"] = json!(
        app.desktop_mcp
            .log_tail(&ServerRecord::key(&record.container_id, &record.name), 4096)
    );
    value
}

/// The servers the manager knows of a container, by name. Only that container's records are
/// read, by the `container:` that begins their ids, since each carries its whole tool list.
pub(super) fn records(app: &App, container_id: &str) -> Vec<ServerRecord> {
    let mut records: Vec<ServerRecord> = app
        .db
        .list_prefixed::<ServerRecord>(SERVERS, &ServerRecord::key(container_id, ""))
        .unwrap_or_default()
        .into_iter()
        .filter(|record| record.container_id == container_id)
        .collect();
    records.sort_by(|a, b| a.name.cmp(&b.name));
    records
}

/// Every desktop server the manager knows, as it last saw them, from one read of them all.
pub async fn all_servers(State(app): State<Arc<App>>) -> Result<Json<Value>> {
    let mut by_container: HashMap<String, Vec<ServerRecord>> = HashMap::new();
    for record in app.db.list::<ServerRecord>(SERVERS)? {
        by_container
            .entry(record.container_id.clone())
            .or_default()
            .push(record);
    }
    let mut items = Vec::new();
    let mut containers: Vec<Container> = app.db.list(crate::model::CONTAINERS)?;
    containers.sort_by(|a, b| a.spec.name.cmp(&b.spec.name));
    let sets = crate::scale::all(&app)?;
    for container in containers {
        let Some(mut records) = by_container.remove(&container.id) else {
            continue;
        };
        records.sort_by(|a, b| a.name.cmp(&b.name));
        let running = app.desktop_mcp.watched(&container.id);
        for record in records {
            items.push(summary(&app, &record, &container.spec.name, running, &sets));
        }
    }
    Ok(Json(json!({"items": items})))
}

/// A running container's Docker ID, or a conflict that says to start it.
async fn running(app: &App, container: &Container) -> Result<String> {
    let info = app.owned(container).await?;
    if info["State"]["Running"] != true {
        return Err(Error(
            StatusCode::CONFLICT,
            "Start the container before changing its MCP servers".into(),
        ));
    }
    Ok(info["Id"]
        .as_str()
        .context("Missing container ID")?
        .to_owned())
}

/// The servers of one container, read again from its desktop while it runs.
pub async fn container_servers(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    Ok(Json(servers_of(&app, &id, false).await?))
}

/// The servers of one container, read again from its desktop while it runs. With `schemas` each
/// tool also has its `input_schema`, which an agent needs to call it with call_mcp_server_tool
/// and the dashboard's lists leave out.
pub(crate) async fn servers_of(app: &Arc<App>, id: &str, schemas: bool) -> Result<Value> {
    let container = app.container(id)?;
    let info = app.owned(&container).await?;
    let running = info["State"]["Running"] == true;
    if running {
        let docker_id = info["Id"].as_str().context("Missing container ID")?;
        let found = app.read_servers(docker_id).await?;
        watch::apply(app, id, found).await;
    }
    let sets = crate::scale::all(app)?;
    let items = records(app, id)
        .iter()
        .map(|record| {
            let mut value = summary(app, record, &container.spec.name, running, &sets);
            if schemas {
                for (listed, named) in value["tools"]
                    .as_array_mut()
                    .into_iter()
                    .flatten()
                    .zip(server_tool_names(record))
                {
                    listed["input_schema"] = json!(named.tool.input_schema);
                }
            }
            value
        })
        .collect::<Vec<_>>();
    Ok(json!({"items": items}))
}

/// Call a desktop server's tool for the administrator's channel, on a connection of its own that
/// no endpoint shares, unless a scale set runs the server once for its instances. The server must
/// be known and list the tool, so a typo is told as such.
pub(crate) async fn call_server_tool(
    app: &Arc<App>,
    container_id: &str,
    server: &str,
    tool: &str,
    arguments: rmcp::model::JsonObject,
) -> Result<rmcp::model::CallToolResult> {
    let container = app.container(container_id)?;
    let record = find(app, container_id, server).map_err(|_| {
        Error(
            StatusCode::NOT_FOUND,
            format!(
                "{} has no MCP server called {server}; list_mcp_servers lists them",
                container.spec.name
            ),
        )
    })?;
    if let Some(refusal) = super::placement::placement(app, container_id, server).refusal(server) {
        return Err(refusal.into());
    }
    if let Some(error) = &record.error {
        return Err(Error(
            StatusCode::CONFLICT,
            format!("The server {server} cannot be reached: {error}"),
        ));
    }
    if !record.tools.iter().any(|listed| listed.name == tool) {
        return Err(Error(
            StatusCode::NOT_FOUND,
            format!(
                "The server {server} has no tool called {tool}; list_mcp_servers with container_id lists its tools by name with their input_schema"
            ),
        ));
    }
    Ok(super::client::call(app, ROOT_CONNECTIONS, &container, &record, tool, arguments).await?)
}

/// The pool's name for the connections `/mcp` holds, which no endpoint's name can be.
const ROOT_CONNECTIONS: &str = "/mcp";

fn find(app: &App, id: &str, name: &str) -> Result<ServerRecord> {
    app.db
        .get(SERVERS, &ServerRecord::key(id, name))
        .map_err(|_| missing())
}

/// Register a server in the desktop user's folder, replacing one of the same name there, and
/// read its tools.
pub async fn register_server(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(declaration): Json<Declaration>,
) -> Result<Json<Value>> {
    declaration.validate()?;
    let container = app.container(&id)?;
    // Values are resolved now too, so a name that is not a compose value, or a secret one, which
    // the desktop user's servers do not get, fails here.
    declaration
        .spec
        .resolve(&Source::Container.values(&app, &id))
        .map_err(|e| Error(StatusCode::BAD_REQUEST, format!("{e:#}")))?;
    let docker_id = running(&app, &container).await?;
    app.write_server(&docker_id, &declaration).await?;
    let found = app.read_servers(&docker_id).await?;
    watch::apply(&app, &id, found).await;
    watch::fetch(&app, &id, &declaration.name).await;
    let record = find(&app, &id, &declaration.name)?;
    let sets = crate::scale::all(&app)?;
    Ok(Json(summary(
        &app,
        &record,
        &container.spec.name,
        true,
        &sets,
    )))
}

/// Remove a server the desktop user registered. One the image declares stays: the image's files
/// are the desktop's, and an endpoint can leave it out instead.
pub async fn unregister_server(
    State(app): State<Arc<App>>,
    Path((id, name)): Path<(String, String)>,
) -> Result<Json<Value>> {
    server_name(&name)?;
    let container = app.container(&id)?;
    let record = find(&app, &id, &name)?;
    if record.source == Source::Image {
        return Err(Error(
            StatusCode::CONFLICT,
            format!(
                "The image declares {name}, so it cannot be removed; leave it out of an endpoint instead"
            ),
        ));
    }
    let docker_id = running(&app, &container).await?;
    if !app.remove_server(&docker_id, &name).await? {
        return Err(missing());
    }
    let found = app.read_servers(&docker_id).await?;
    watch::apply(&app, &id, found).await;
    Ok(Json(json!({"removed": true, "name": name})))
}

/// Start a server again and read its tools.
pub async fn refresh_server(
    State(app): State<Arc<App>>,
    Path((id, name)): Path<(String, String)>,
) -> Result<Json<Value>> {
    let container = app.container(&id)?;
    find(&app, &id, &name)?;
    running(&app, &container).await?;
    watch::fetch(&app, &id, &name).await;
    let record = find(&app, &id, &name)?;
    let sets = crate::scale::all(&app)?;
    Ok(Json(summary(
        &app,
        &record,
        &container.spec.name,
        true,
        &sets,
    )))
}
