//! What an endpoint's tool call reaches, for the agent activity log: the container a bound tool
//! works on, and a desktop server's tool by its own name with the annotations its server gave it.
use super::{
    registry::{SERVERS, ServerRecord},
    scopes::{Entry, Target},
};
use crate::{App, activity::Call};
use serde_json::Value;

/// Tell `call` what the endpoint's tool `entry`, called with `args`, reaches.
pub(super) fn describe(app: &App, entry: &Entry, args: &Value, call: &mut Call) {
    match &entry.target {
        Target::Thoric { containers, .. } => {
            // As the endpoint resolves it: its one container, or the one `container` names.
            let id = if containers.len() == 1 && args.get("container").is_none() {
                containers.first().map(|(_, id)| id.clone())
            } else {
                let alias = args["container"].as_str();
                containers
                    .iter()
                    .find(|(name, _)| Some(name.as_str()) == alias)
                    .map(|(_, id)| id.clone())
            };
            call.on_container(app, id);
        }
        // A job of any of the endpoint's containers.
        Target::Job { .. } => {}
        Target::Instance { set, .. } => {
            call.on_container(app, instance(app, set, args, "instance"));
        }
        // The set's own tools: the instance one deletes, none for the others.
        Target::Set { set, .. } => {
            call.on_container(app, instance(app, set, args, "instance"));
        }
        Target::InstanceServer {
            set,
            server,
            tool,
            argument,
        } => {
            server_call(app, call, instance(app, set, args, argument), server, tool);
        }
        // Whichever instance the agent works in, the instance that runs the server is used.
        Target::HomeServer {
            set,
            server,
            tool,
            number,
        } => {
            let id = numbered(app, set, *number);
            server_call(app, call, id, server, tool);
        }
        Target::Server {
            container_id,
            server,
            tool,
        } => {
            let annotations = app
                .db
                .get::<ServerRecord>(SERVERS, &ServerRecord::key(container_id, server))
                .ok()
                .and_then(|record| record.tools.into_iter().find(|t| &*t.name == tool.as_str()))
                .and_then(|tool| tool.annotations);
            call.on_server(server, tool, annotations.as_ref());
            call.on_container(app, Some(container_id.clone()));
        }
    }
}

/// The container of the instance of `set` that the argument `argument` names, as the endpoint
/// resolves it.
fn instance(app: &App, set: &str, args: &Value, argument: &str) -> Option<String> {
    numbered(app, set, u32::try_from(args[argument].as_u64()?).ok()?)
}

/// The container of the instance `number` of `set`.
fn numbered(app: &App, set: &str, number: u32) -> Option<String> {
    app.db
        .get::<crate::scale::ScaleSet>(crate::scale::SETS, set)
        .ok()?
        .instances
        .get(&number)
        .map(|instance| instance.container_id.clone())
}

/// Tell `call` it reaches the tool `tool` of the desktop server `server` in the container `id`,
/// with the annotations that container's record of the server gives it.
fn server_call(app: &App, call: &mut Call, id: Option<String>, server: &str, tool: &str) {
    let annotations = id
        .as_ref()
        .and_then(|id| {
            app.db
                .get::<ServerRecord>(SERVERS, &ServerRecord::key(id, server))
                .ok()
        })
        .and_then(|record| record.tools.into_iter().find(|t| &*t.name == tool))
        .and_then(|tool| tool.annotations);
    call.on_server(server, tool, annotations.as_ref());
    call.on_container(app, id);
}
