//! What an endpoint offers: the container tools of the scopes it was granted, bound to its
//! containers, and the tools of their desktop servers, each under the name the agent sees.
use super::{
    endpoints::{Endpoint, Grant},
    placement::Placement,
    registry::ServerRecord,
    servers::records,
};
use crate::App;
use blake2::{Blake2s256, Digest};
use rmcp::model::{MetaObject, Tool};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

/// A group of container tools an endpoint can be granted for a container. Creating, changing and
/// deleting containers, images and volumes are never part of one, but for `scale`, which an
/// endpoint of a scale set alone is granted: it creates and deletes that set's instances, from its
/// template and within its limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Scope {
    View,
    Input,
    Applications,
    FilesRead,
    FilesWrite,
    Commands,
    Media,
    Power,
    Scale,
}
/// The tool an endpoint of a scale set offers whatever it was granted, since its other tools take
/// the numbers it lists.
pub(crate) const LIST_INSTANCES: &str = "list_instances";
impl Scope {
    pub const ALL: [Scope; 9] = [
        Scope::View,
        Scope::Input,
        Scope::Applications,
        Scope::FilesRead,
        Scope::FilesWrite,
        Scope::Commands,
        Scope::Media,
        Scope::Power,
        Scope::Scale,
    ];
    /// What a container's own endpoint gets: everything but commands, which with sudo is root
    /// inside the container.
    pub const DEFAULT: [Scope; 7] = [
        Scope::View,
        Scope::Input,
        Scope::Applications,
        Scope::FilesRead,
        Scope::FilesWrite,
        Scope::Media,
        Scope::Power,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Scope::View => "view",
            Scope::Input => "input",
            Scope::Applications => "applications",
            Scope::FilesRead => "files_read",
            Scope::FilesWrite => "files_write",
            Scope::Commands => "commands",
            Scope::Media => "media",
            Scope::Power => "power",
            Scope::Scale => "scale",
        }
    }
    /// The Thoric MCP tools of the scope, each of which works on one container.
    pub fn tools(self) -> &'static [&'static str] {
        match self {
            // Only what reads: save_screenshot writes a file, so it is files_write's.
            Scope::View => &[
                "get_container",
                "container_logs",
                "container_diagnostics",
                "get_usage",
                "list_processes",
                "screenshot",
                "list_windows",
                "inspect_ui",
                "find_text",
                "wait_for_condition",
                "get_clipboard",
            ],
            // Asking a person for help changes nothing in the container, and the agents that
            // need it are those that drive its screen.
            Scope::Input => &[
                "desktop_input",
                "act_and_observe",
                "focus_window",
                "set_window_geometry",
                "gamepad_input",
                "request_human",
                "set_clipboard",
            ],
            Scope::Applications => &[
                "launch_application",
                "list_applications",
                "get_application",
                "application_logs",
                "stop_application",
                "application_diagnostics",
                "sample_metrics",
                "kill_process",
            ],
            Scope::FilesRead => &[
                "list_files",
                "search_files",
                "read_file",
                "download_file",
                "get_changes",
            ],
            Scope::FilesWrite => &[
                "write_file",
                "upload_file",
                "move_file",
                "delete_file",
                "save_screenshot",
            ],
            Scope::Commands => &[
                "exec_container",
                "open_terminal",
                "list_terminals",
                "terminal_input",
                "terminal_output",
                "close_terminal",
            ],
            Scope::Media => &[
                "start_recording",
                "stop_recording",
                "list_recordings",
                "export_artifacts",
                "capture_audio",
                "inject_audio",
            ],
            Scope::Power => &["container_action"],
            Scope::Scale => &["create_instance", "delete_instance"],
        }
    }
    /// Scopes whose tools return a job, which `get_job` then reads.
    pub(super) fn has_jobs(self) -> bool {
        matches!(self, Scope::Commands | Scope::Power | Scope::Scale)
    }
    /// Whether the tool of this name is one of the scope's.
    pub(super) fn offers(self, name: &str) -> bool {
        self.tools().contains(&name)
    }
}
/// Arguments an endpoint never passes on: the desktop user is as far as a scope reaches.
const ROOT_TOOLS: [&str; 3] = ["exec_container", "open_terminal", "kill_process"];

/// Sentences of the administrator's descriptions that do not hold on an endpoint, which takes
/// neither root nor container_id and offers neither create_transfer nor the dashboard's routes,
/// with what the endpoint says instead. Each sentence must be in its tool's description.
pub(super) const ENDPOINT_WORDING: [(&str, &str, &str); 6] = [
    (
        "exec_container",
        "Defaults to desktop UID/GID 1001:1001; set root=true explicitly to install packages.",
        "Runs as the desktop user, UID/GID 1001:1001, never as root; with sudo enabled for the container, sudo works in the script.",
    ),
    (
        "open_terminal",
        " root=true runs the shell as root.",
        " The shell runs as the desktop user, never as root.",
    ),
    (
        "kill_process",
        "Runs as the desktop user, which may signal only its own processes; root=true signals a process of any user as root.",
        "Runs as the desktop user, never as root, so it signals only the desktop user's own processes.",
    ),
    (
        "download_file",
        " Larger files are available through that route, and a folder downloads there as a .tar.gz (/api/containers/{container_id}/files/download?path=FOLDER); create_transfer moves either to the machine you run on.",
        " Larger files and folders are not available through this endpoint.",
    ),
    (
        "get_usage",
        " Leave out container_id for every container. A deleted container is still reported, with removed true, until its samples are 30 minutes old.",
        "",
    ),
    (
        "get_usage",
        " gpu is the whole host GPU, sampled with nvidia-smi while a GPU container runs: neither WSL nor the driver tells which container uses it, so it is never a container's share.",
        " The host GPU is not reported here.",
    ),
];

/// What a tool of an endpoint calls.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Target {
    /// A Thoric MCP tool for the instance of the scale set that `instance` names.
    Instance { tool: &'static str, set: String },
    /// A Thoric MCP tool of the scale set itself, which lists, creates or deletes its instances.
    Set { tool: &'static str, set: String },
    /// A tool of a desktop server, in the instance of the scale set that the argument named
    /// `argument` names.
    InstanceServer {
        set: String,
        server: String,
        tool: String,
        argument: &'static str,
    },
    /// A tool of a desktop server that the scale set runs in its instance `number` alone, for
    /// every instance: it takes no instance.
    HomeServer {
        set: String,
        server: String,
        tool: String,
        number: u32,
    },
    /// A Thoric MCP tool, for the containers listed by their alias. With one grant the tool
    /// takes no container argument.
    Thoric {
        tool: &'static str,
        containers: Vec<(String, String)>,
    },
    /// `get_job`, for jobs of these containers. On a scale set's endpoint, `set`, they are its
    /// instances now, and a job of one that is gone since is still the endpoint's to read.
    Job { containers: Vec<String>, set: bool },
    /// A tool of a desktop server.
    Server {
        container_id: String,
        server: String,
        tool: String,
    },
}

pub(crate) struct Entry {
    pub tool: Tool,
    pub target: Target,
}

/// A desktop server's tool with the name it has on an endpoint of its container alone.
pub(super) struct Named<'a> {
    /// `SERVER__TOOL`, or with the hash tail when another tool of the server took that name.
    pub name: String,
    /// Whether the name carries the hash tail because of such a collision.
    pub hashed: bool,
    pub tool: &'a Tool,
}

/// A server's tools under the names they have on an endpoint of their container alone, which
/// server lists report and a grant's deny list uses. Two tools whose names clean to one keep
/// their order, the later one with the hash tail of its own name; one whose hashed name is taken
/// too is left out. Server names have no underscore, so a name tells its server apart from the
/// tool, and tools of different servers never collide; a container tool's name has no `__`.
pub(super) fn server_tool_names(record: &ServerRecord) -> Vec<Named<'_>> {
    let mut out: Vec<Named<'_>> = Vec::new();
    for tool in &record.tools {
        let parts = [record.name.as_str(), tool.name.as_ref()];
        let mut name = exposed_name(&parts);
        let mut hashed = false;
        if out.iter().any(|named| named.name == name) {
            name = exposed_name_hashed(&parts);
            hashed = true;
            if out.iter().any(|named| named.name == name) {
                continue;
            }
        }
        out.push(Named { name, hashed, tool });
    }
    out
}

/// The name an agent sees: the parts joined with `__`, in the characters MCP clients accept and
/// at most 64 of them, shortened with a hash of the whole when longer.
pub(crate) fn exposed_name(parts: &[&str]) -> String {
    exposed(parts, false)
}

/// The exposed name with the hash tail whatever its length, for a name another tool already
/// took: `a.b` and `a_b` on one server both clean to `a_b`, and the tail tells them apart.
pub(crate) fn exposed_name_hashed(parts: &[&str]) -> String {
    exposed(parts, true)
}

fn exposed(parts: &[&str], hashed: bool) -> String {
    let joined = parts.join("__");
    let clean: String = joined
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if clean.len() <= 64 && !hashed {
        return clean;
    }
    let hash = Blake2s256::digest(joined.as_bytes());
    let suffix: String = hash[..3].iter().map(|b| format!("{b:02x}")).collect();
    format!("{}_{suffix}", &clean[..clean.len().min(57)])
}

/// A Thoric tool as an endpoint offers it: bound to its one container, or taking `container`
/// among the aliases of those granted it, never as root, and described in those terms.
pub(crate) fn bound_tool(tool: &Tool, aliases: Option<&[String]>) -> Tool {
    let mut schema = Value::Object((*tool.input_schema).clone());
    let mut drop = vec!["container_id"];
    if ROOT_TOOLS.contains(&tool.name.as_ref()) {
        drop.push("root");
    }
    if let Some(properties) = schema["properties"].as_object_mut() {
        for key in &drop {
            properties.remove(*key);
        }
    }
    if let Some(required) = schema["required"].as_array_mut() {
        required.retain(|key| !drop.iter().any(|d| key == d));
    }
    if let Some(aliases) = aliases {
        schema["properties"]["container"] = json!({"type":"string","enum":aliases,"description":"The container to work on, by its name on this endpoint."});
        match schema["required"].as_array_mut() {
            Some(required) => required.push(json!("container")),
            None => schema["required"] = json!(["container"]),
        }
    }
    let mut bound = tool.clone();
    bound.input_schema = Arc::new(schema.as_object().cloned().unwrap_or_default());
    if let Some(description) = &tool.description {
        let worded = ENDPOINT_WORDING
            .iter()
            .filter(|(name, ..)| *name == tool.name)
            .fold(description.to_string(), |text, (_, from, to)| {
                text.replace(from, to)
            });
        bound.description = Some(worded.into());
    }
    bound
}

/// What an endpoint of a scale set says of the instance a tool takes.
const INSTANCE: &str = "The instance of the scale set to work on, by its number; list_instances shows the instances and the volumes they share.";

/// A Thoric tool as an endpoint of a scale set offers it: taking `instance`, never `container_id`
/// or root.
pub(crate) fn instance_tool(tool: &Tool) -> Tool {
    let mut bound = bound_tool(tool, None);
    let mut schema = Value::Object((*bound.input_schema).clone());
    schema["properties"]["instance"] = instance_schema();
    match schema["required"].as_array_mut() {
        Some(required) => required.push(json!("instance")),
        None => schema["required"] = json!(["instance"]),
    }
    bound.input_schema = Arc::new(schema.as_object().cloned().unwrap_or_default());
    bound
}

fn instance_schema() -> Value {
    json!({"type": "integer", "minimum": 1, "maximum": crate::scale::MAX_INSTANCES, "description": INSTANCE})
}

/// A tool of the scale set itself as its endpoint offers it: for that set, so without `set`.
pub(crate) fn set_tool(tool: &Tool) -> Tool {
    let mut schema = Value::Object((*tool.input_schema).clone());
    if let Some(properties) = schema["properties"].as_object_mut() {
        properties.remove("set");
    }
    if let Some(required) = schema["required"].as_array_mut() {
        required.retain(|key| key != "set");
    }
    let mut bound = tool.clone();
    bound.input_schema = Arc::new(schema.as_object().cloned().unwrap_or_default());
    bound
}

/// The argument a desktop server's tool takes the instance by on an endpoint of a scale set:
/// `instance`, or `thoric_instance` for a tool that has an `instance` of its own.
pub(super) fn instance_argument(tool: &Tool) -> &'static str {
    if tool
        .input_schema
        .get("properties")
        .and_then(|properties| properties.get("instance"))
        .is_some()
    {
        "thoric_instance"
    } else {
        "instance"
    }
}

/// A desktop server's tool as an endpoint of a scale set offers it once for every instance: with
/// the argument that names the instance beside its own, or, for a server the set runs in its
/// instance `home` alone, without one and described as running there.
fn instance_server_tool(
    tool: &Tool,
    name: String,
    set: &str,
    server: &str,
    home: Option<u32>,
) -> Tool {
    let mut exposed = tool.clone();
    exposed.name = name.into();
    let description = tool.description.as_deref().unwrap_or_default();
    exposed.description = Some(match home {
        Some(home) => {
            format!("[{set} / {server}, instance {home} for every instance] {description}").into()
        }
        None => format!("[{set} / {server}] {description}").into(),
    });
    exposed.annotations = None;
    exposed.icons = None;
    if home.is_none() {
        let argument = instance_argument(tool);
        let mut schema = Value::Object((*tool.input_schema).clone());
        if !schema["properties"].is_object() {
            schema["properties"] = json!({});
        }
        schema["properties"][argument] = instance_schema();
        match schema["required"].as_array_mut() {
            Some(required) => required.push(json!(argument)),
            None => schema["required"] = json!([argument]),
        }
        exposed.input_schema = Arc::new(schema.as_object().cloned().unwrap_or_default());
    }
    let mut meta = serde_json::Map::new();
    meta.insert("thoric/set".into(), json!(set));
    meta.insert("thoric/server".into(), json!(server));
    meta.insert("thoric/tool".into(), json!(tool.name));
    exposed.meta = Some(MetaObject(meta));
    exposed
}

/// The tools of an endpoint of the scale set `set`: `list_instances`, the container tools of the
/// scopes granted, taking `instance`, `create_instance` and `delete_instance` with `scale`,
/// `get_job`, and each desktop server's tools once, taking `instance`, as the instances have
/// them; they come from one image and compose, so they have the same ones. A server the set runs
/// in one instance alone has that instance's tools, which take no instance.
fn set_catalog(app: &App, grant: &Grant, set: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    if !grant.excludes(LIST_INSTANCES)
        && let Some(tool) = crate::mcp::listed_tool(LIST_INSTANCES)
    {
        entries.push(Entry {
            tool: set_tool(&tool),
            target: Target::Set {
                tool: LIST_INSTANCES,
                set: set.into(),
            },
        });
    }
    for scope in Scope::ALL {
        if !grant.scopes.contains(&scope) {
            continue;
        }
        for name in scope.tools() {
            if grant.excludes(name) {
                continue;
            }
            let Some(tool) = crate::mcp::listed_tool(name) else {
                continue;
            };
            entries.push(if scope == Scope::Scale {
                Entry {
                    tool: set_tool(&tool),
                    target: Target::Set {
                        tool: name,
                        set: set.into(),
                    },
                }
            } else {
                Entry {
                    tool: instance_tool(&tool),
                    target: Target::Instance {
                        tool: name,
                        set: set.into(),
                    },
                }
            });
        }
    }
    let scale_set = app
        .db
        .get::<crate::scale::ScaleSet>(crate::scale::SETS, set)
        .ok();
    let instances = scale_set
        .iter()
        .flat_map(|scale_set| scale_set.instances.iter())
        .collect::<Vec<_>>();
    let members = instances
        .iter()
        .map(|(_, instance)| instance.container_id.clone())
        .collect::<Vec<_>>();
    // The instance that runs a server for all of them, for a server the set runs once.
    let home_of = |server: &str| {
        scale_set
            .as_ref()
            .and_then(|scale_set| scale_set.runs_in(server))
    };
    if grant.scopes.iter().any(|scope| scope.has_jobs())
        && !grant.excludes("get_job")
        && let Some(mut tool) = crate::mcp::listed_tool("get_job")
    {
        tool.description = Some("Read a job that a tool of this endpoint started, such as create_instance, exec_container or container_action: its status, bounded log and result. Wait for completed, failed or interrupted; a queued job is not finished.".into());
        entries.push(Entry {
            tool,
            target: Target::Job {
                containers: members.clone(),
                set: true,
            },
        });
    }
    for (number, instance) in instances {
        for record in records(app, &instance.container_id)
            .iter()
            .filter(|record| {
                record.error.is_none()
                && grant.servers.includes(&record.name)
                // Only the instance that runs it has the tools of a server it runs for all.
                && home_of(&record.name).is_none_or(|home| home == *number)
            })
        {
            let home = home_of(&record.name);
            for Named { name, tool, .. } in server_tool_names(record) {
                if grant.excludes(&name) || entries.iter().any(|entry| entry.tool.name == name) {
                    continue;
                }
                entries.push(Entry {
                    tool: instance_server_tool(tool, name, set, &record.name, home),
                    target: match home {
                        Some(number) => Target::HomeServer {
                            set: set.into(),
                            server: record.name.clone(),
                            tool: tool.name.to_string(),
                            number,
                        },
                        None => Target::InstanceServer {
                            set: set.into(),
                            server: record.name.clone(),
                            tool: tool.name.to_string(),
                            argument: instance_argument(tool),
                        },
                    },
                });
            }
        }
    }
    entries
}

/// A desktop server's tool as an endpoint offers it. Its annotations and icons come from inside
/// the desktop, so they are left out: a client could otherwise run a tool unasked because it
/// claims to be read-only.
fn server_tool(
    tool: &Tool,
    name: String,
    container: &str,
    container_id: &str,
    server: &str,
) -> Tool {
    let mut exposed = tool.clone();
    exposed.name = name.into();
    let description = tool.description.as_deref().unwrap_or_default();
    exposed.description = Some(format!("[{container} / {server}] {description}").into());
    exposed.annotations = None;
    exposed.icons = None;
    let mut meta = serde_json::Map::new();
    meta.insert("thoric/container_id".into(), json!(container_id));
    meta.insert("thoric/server".into(), json!(server));
    meta.insert("thoric/tool".into(), json!(tool.name));
    exposed.meta = Some(MetaObject(meta));
    exposed
}

/// Every tool of an endpoint, in a stable order: container tools by scope, `get_job`, then the
/// tools of the desktop servers by container and server. A tool a grant leaves out is not offered
/// for that container: on an endpoint of several containers it keeps the others, and it is gone
/// once every grant leaves it out.
pub(crate) fn catalog(app: &App, endpoint: &Endpoint) -> Vec<Entry> {
    if let Some((grant, set)) = endpoint.set() {
        return set_catalog(app, grant, set);
    }
    let single = endpoint.grants.len() == 1;
    let mut entries: Vec<Entry> = Vec::new();
    // A container has no instances to create or delete.
    for scope in Scope::ALL
        .into_iter()
        .filter(|scope| *scope != Scope::Scale)
    {
        let granted: Vec<&Grant> = endpoint
            .grants
            .iter()
            .filter(|grant| grant.scopes.contains(&scope))
            .collect();
        if granted.is_empty() {
            continue;
        }
        for name in scope.tools() {
            let Some(tool) = crate::mcp::listed_tool(name) else {
                continue;
            };
            let containers: Vec<(String, String)> = granted
                .iter()
                .filter(|grant| !grant.excludes(name))
                .map(|grant| (grant.alias.clone(), grant.container_id.clone()))
                .collect();
            if containers.is_empty() {
                continue;
            }
            let aliases: Vec<String> = containers.iter().map(|(alias, _)| alias.clone()).collect();
            entries.push(Entry {
                tool: bound_tool(&tool, (!single).then_some(aliases.as_slice())),
                target: Target::Thoric {
                    tool: name,
                    containers,
                },
            });
        }
    }
    let jobs: Vec<String> = endpoint
        .grants
        .iter()
        .filter(|grant| {
            grant.scopes.iter().any(|scope| scope.has_jobs()) && !grant.excludes("get_job")
        })
        .map(|grant| grant.container_id.clone())
        .collect();
    if !jobs.is_empty()
        && let Some(mut tool) = crate::mcp::listed_tool("get_job")
    {
        tool.description = Some("Read a job that a tool of this endpoint started, such as exec_container or container_action: its status, bounded log and result. Wait for completed, failed or interrupted; a queued job is not finished.".into());
        entries.push(Entry {
            tool,
            target: Target::Job {
                containers: jobs,
                set: false,
            },
        });
    }
    let sets = crate::scale::all(app).unwrap_or_default();
    for grant in &endpoint.grants {
        let container = app
            .container(&grant.container_id)
            .map(|c| c.spec.name)
            .unwrap_or_else(|_| grant.alias.clone());
        // Only the granted containers' servers are read, each with its whole tool list. A server
        // another instance of the container's set runs for it is not offered here.
        let known = records(app, &grant.container_id);
        for record in known.iter().filter(|record| {
            record.error.is_none()
                && grant.servers.includes(&record.name)
                && Placement::of(&sets, &grant.container_id, &record.name)
                    .home()
                    .is_none()
        }) {
            for Named { name, hashed, tool } in server_tool_names(record) {
                // The deny list names the tool as this container alone would expose it, so it
                // holds whatever the alias is and whichever containers the endpoint covers.
                if grant.excludes(&name) {
                    continue;
                }
                // With several containers the alias leads; the hash tail, which tells two tools
                // that clean to one name apart, stays where the name alone has it, so leaving one
                // of them out does not rename the other.
                let name = if single {
                    name
                } else {
                    let parts = [
                        grant.alias.as_str(),
                        record.name.as_str(),
                        tool.name.as_ref(),
                    ];
                    if hashed {
                        exposed_name_hashed(&parts)
                    } else {
                        exposed_name(&parts)
                    }
                };
                if entries.iter().any(|entry| entry.tool.name == name) {
                    continue;
                }
                entries.push(Entry {
                    tool: server_tool(tool, name, &container, &grant.container_id, &record.name),
                    target: Target::Server {
                        container_id: grant.container_id.clone(),
                        server: record.name.clone(),
                        tool: tool.name.to_string(),
                    },
                });
            }
        }
    }
    entries
}

/// The scopes and their tools, for the dashboard's endpoint form.
pub(crate) fn describe() -> Value {
    json!({
        "scopes": Scope::ALL
            .iter()
            .map(|scope| json!({"id": scope.id(), "tools": scope.tools(), "set_only": *scope == Scope::Scale}))
            .collect::<Vec<_>>(),
        "default": Scope::DEFAULT.iter().map(|scope| scope.id()).collect::<Vec<_>>(),
        "set_tools": [LIST_INSTANCES],
    })
}
