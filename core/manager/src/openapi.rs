//! The OpenAPI 3.1 document of the HTTP API, served at `GET /api/openapi.json`.
//!
//! Nothing in it is written by hand as JSON. [`routes::all`] names every route the router serves
//! with the access it needs, the types its handler reads and what it answers, and the JSON
//! Schemas of those types are the ones `schemars` derives from them, so a field added to a
//! request type reaches the document by itself. `openapi_tests.rs` holds the table to the router
//! in both directions: a route added in `main.rs` without an entry here fails `cargo test`, and so
//! does an entry for a route that is gone.
mod routes;

use axum::{http::header, response::IntoResponse};
use schemars::{JsonSchema, Schema, SchemaGenerator, generate::SchemaSettings};
use serde_json::{Map, Value, json};
use std::sync::LazyLock;

/// The document as it is sent, made once: it depends on nothing but the code.
static DOCUMENT: LazyLock<String> = LazyLock::new(|| document().to_string());

/// `GET /api/openapi.json`.
pub async fn serve() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/json")],
        DOCUMENT.as_str(),
    )
}

const SESSION: &str = "Session";
const INSTALLATION: &str = "Installation";
const CONTAINERS: &str = "Containers";
const DESKTOP: &str = "Desktop";
const FILES: &str = "Files";
const IMAGES: &str = "Images";
const VOLUMES: &str = "Volumes";
const MCP: &str = "MCP";
const JOBS: &str = "Jobs";
const EVENTS: &str = "Events";
const TRANSFERS: &str = "Transfers and ports";

/// The groups the operations are listed in, in this order.
const TAGS: &[(&str, &str)] = &[
    (SESSION, "Sign-in and the session the other routes need."),
    (
        INSTALLATION,
        "The installation: its settings, Docker, updates, backups, resource usage and dependencies.",
    ),
    (
        CONTAINERS,
        "Containers: creating, starting and stopping them, their settings, snapshots, scripts and logs.",
    ),
    (
        DESKTOP,
        "A running desktop: its screen, mouse and keyboard, windows, applications, terminals, recordings, audio and gamepad.",
    ),
    (FILES, "A container's files."),
    (
        IMAGES,
        "Images: the catalog, builds, composes, icons and archives.",
    ),
    (VOLUMES, "Named volumes, their files and their archives."),
    (
        MCP,
        "MCP: the root `/mcp`, the MCP endpoints, desktop MCP servers and the inspector.",
    ),
    (JOBS, "The jobs long operations run as, and their retry."),
    (
        EVENTS,
        "The dashboard's event stream, agents' requests for a person and the agent activity log.",
    ),
    (
        TRANSFERS,
        "Transfer links and desktop ports, which their own tokens open instead of the session.",
    ),
];

/// What a route needs before it answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Access {
    /// Nothing: the route answers before sign-in.
    Public,
    /// `X-Thoric-Request: 1`, which sign-in asks for as every change does.
    SignIn,
    /// The dashboard's session cookie, and `X-Thoric-Request: 1` on a change (`auth::protect`).
    Session,
    /// The MCP token, never the session.
    McpToken,
    /// An MCP endpoint's own token, or the MCP token.
    EndpointToken,
    /// The one-time token in the path, from the MCP tool `create_transfer`.
    TransferLink,
    /// The metrics token, `METRICS_TOKEN`, which opens `/metrics` alone.
    MetricsToken,
}

/// Makes the JSON Schema of the type a handler reads or answers with.
type Schematic = fn(&mut SchemaGenerator) -> Schema;

/// A reference to `T`'s schema among the components.
fn schema<T: JsonSchema>(generator: &mut SchemaGenerator) -> Schema {
    generator.subschema_for::<T>()
}

/// `T`'s schema itself, for a query string whose fields become parameters.
fn fields<T: JsonSchema>(generator: &mut SchemaGenerator) -> Schema {
    T::json_schema(generator)
}

/// What a route reads besides its path.
enum Body {
    None,
    Json {
        schema: Schematic,
        required: bool,
    },
    /// Bytes of one of these media types, streamed as they arrive or passed on.
    Bytes(&'static [&'static str]),
}

/// What a route answers with when it succeeds.
enum Reply {
    /// JSON the handler puts together itself, which `docs/API.md` describes.
    Json,
    /// JSON of a type with a schema.
    Typed(Schematic),
    /// `{"job_id"}` of the job it queued.
    Job,
    NoContent,
    /// A short `text/plain` answer.
    Text,
    /// A file or an archive of this media type, streamed as it is read.
    Bytes(&'static str),
    /// Server-sent events until the session ends.
    Events,
    /// A WebSocket.
    WebSocket,
    /// Whatever the service behind the route answers.
    Relayed,
}

/// One route of the router: one method, or several for a route any method reaches.
struct Route {
    methods: &'static [&'static str],
    /// Served by every method (`axum::routing::any`); `methods` are those the document names.
    any: bool,
    /// As `main.rs` registers it, `{*rest}` and all.
    path: &'static str,
    tag: &'static str,
    summary: &'static str,
    access: Access,
    query: Option<Schematic>,
    body: Body,
    reply: Reply,
    /// What a `503` of the route's own answers with, instead of the error that Docker cannot be
    /// reached.
    unavailable: Option<&'static str>,
}

fn route(
    methods: &'static [&'static str],
    path: &'static str,
    tag: &'static str,
    summary: &'static str,
) -> Route {
    Route {
        methods,
        any: false,
        path,
        tag,
        summary,
        access: Access::Session,
        query: None,
        body: Body::None,
        reply: Reply::Json,
        unavailable: None,
    }
}
fn get(path: &'static str, tag: &'static str, summary: &'static str) -> Route {
    route(&["GET"], path, tag, summary)
}
fn post(path: &'static str, tag: &'static str, summary: &'static str) -> Route {
    route(&["POST"], path, tag, summary)
}
fn put(path: &'static str, tag: &'static str, summary: &'static str) -> Route {
    route(&["PUT"], path, tag, summary)
}
fn delete(path: &'static str, tag: &'static str, summary: &'static str) -> Route {
    route(&["DELETE"], path, tag, summary)
}
/// A route every method reaches, as the methods the document names; its body and answer belong
/// to the service behind it.
fn any(
    methods: &'static [&'static str],
    path: &'static str,
    tag: &'static str,
    summary: &'static str,
) -> Route {
    Route {
        any: true,
        body: Body::Bytes(&["*/*"]),
        reply: Reply::Relayed,
        ..route(methods, path, tag, summary)
    }
}

impl Route {
    fn access(mut self, access: Access) -> Self {
        self.access = access;
        self
    }
    fn query<T: JsonSchema>(mut self) -> Self {
        self.query = Some(fields::<T>);
        self
    }
    fn json<T: JsonSchema>(mut self) -> Self {
        self.body = Body::Json {
            schema: schema::<T>,
            required: true,
        };
        self
    }
    /// A JSON body the handler also does without.
    fn optional_json<T: JsonSchema>(mut self) -> Self {
        self.body = Body::Json {
            schema: schema::<T>,
            required: false,
        };
        self
    }
    fn upload(mut self, media: &'static [&'static str]) -> Self {
        self.body = Body::Bytes(media);
        self
    }
    fn returns<T: JsonSchema>(mut self) -> Self {
        self.reply = Reply::Typed(schema::<T>);
        self
    }
    fn job(mut self) -> Self {
        self.reply = Reply::Job;
        self
    }
    fn no_content(mut self) -> Self {
        self.reply = Reply::NoContent;
        self
    }
    fn text(mut self) -> Self {
        self.reply = Reply::Text;
        self
    }
    fn download(mut self, media: &'static str) -> Self {
        self.reply = Reply::Bytes(media);
        self
    }
    fn events(mut self) -> Self {
        self.reply = Reply::Events;
        self
    }
    fn websocket(mut self) -> Self {
        self.reply = Reply::WebSocket;
        self
    }
    /// A `503` that is the route's own JSON answer, such as a readiness check that failed.
    fn unavailable(mut self, description: &'static str) -> Self {
        self.unavailable = Some(description);
        self
    }
}

/// What every error status carries (`error::Error`).
#[derive(JsonSchema)]
#[allow(dead_code, reason = "described in the document, never built")]
struct Error {
    /// What went wrong.
    error: String,
}

/// What a route that queues a job answers (`jobs::launch`).
#[derive(JsonSchema)]
#[allow(dead_code, reason = "described in the document, never built")]
struct JobStarted {
    /// Follow the job at `GET /api/jobs/{job_id}`.
    job_id: String,
}

/// The header that carries a request's ID both ways, as `request_log::REQUEST_ID` names it.
const REQUEST_ID: &str = "X-Request-Id";

/// The error statuses any route can answer with, as the table of errors in `docs/API.md` names
/// them (`error::Error`, `error::Fault`): status, the name of its response and what it means.
const ERRORS: &[(&str, &str, &str)] = &[
    (
        "400",
        "Refused",
        "The request was refused: a value that a check does not accept, or something it names that Docker does not have. Change the request before sending it again.",
    ),
    (
        "401",
        "SignedOut",
        "There is no session, or it ended, or the token is missing or wrong.",
    ),
    (
        "403",
        "Forbidden",
        "Not allowed: a foreign `Origin`, the manager's own volumes, or a file the desktop user cannot open.",
    ),
    (
        "404",
        "NotFound",
        "The record, or what the route names, does not exist.",
    ),
    (
        "409",
        "Conflict",
        "It conflicts with the state of something, such as another operation still running on the same container, image or volume. Send it again once that has changed.",
    ),
    (
        "500",
        "ManagerFailed",
        "The manager itself failed: its database, its disk, or a task that ended unexpectedly.",
    ),
    (
        "502",
        "DockerFailed",
        "Docker failed at what it was asked, or a helper that the manager runs in a desktop or a helper container crashed. The message carries their answer.",
    ),
    (
        "503",
        "DockerUnreachable",
        "Docker cannot be reached: its daemon is stopped or its socket is missing. The same request works once Docker is back.",
    ),
    (
        "504",
        "DockerTimedOut",
        "A Docker command did not finish within its time limit.",
    ),
];

/// The fields of a create request that `api::creation_request` fills with the installation's
/// defaults when the request leaves them out. The defaults their type names are those of records
/// saved before a field existed, which a request never gets.
const INSTALLATION_DEFAULTS: &[&str] = &[
    "gpu",
    "internet",
    "sudo",
    "audio",
    "microphone",
    "gamepad",
    "vpn",
    "proxy",
    // Its idle_stop_minutes.
    "lifecycle",
];

/// Say of each field of [`INSTALLATION_DEFAULTS`] where its value comes from instead.
fn installation_defaults(schemas: &mut Map<String, Value>) {
    let Some(properties) = schemas
        .get_mut("CreateContainer")
        .and_then(|schema| schema["properties"].as_object_mut())
    else {
        return;
    };
    for field in INSTALLATION_DEFAULTS {
        if let Some(Value::Object(property)) = properties.get_mut(*field) {
            property.remove("default");
            let mut description = property
                .remove("description")
                .and_then(|text| text.as_str().map(|text| format!("{text} ")))
                .unwrap_or_default();
            description.push_str(
                "Left out, the installation's default, which `GET /api/settings` reports.",
            );
            property.insert("description".into(), json!(description));
        }
    }
}

/// The whole document.
fn document() -> Value {
    let mut generator = SchemaSettings::draft2020_12()
        .with(|settings| {
            settings.definitions_path = "/components/schemas".into();
            settings.meta_schema = None;
        })
        .into_generator();
    let error = schema::<Error>(&mut generator).to_value();
    let job = schema::<JobStarted>(&mut generator).to_value();
    let mut paths = Map::new();
    for route in routes::all() {
        let item = paths
            .entry(template(route.path))
            .or_insert_with(|| json!({}));
        for method in route.methods {
            item[method.to_ascii_lowercase()] =
                operation(&route, method, &mut generator, &error, &job);
        }
    }
    let mut schemas = generator.take_definitions(true);
    installation_defaults(&mut schemas);
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Thoric",
            "version": env!("CARGO_PKG_VERSION"),
            "summary": "The HTTP API of a Thoric manager.",
            "description": "The routes the dashboard uses, for scripts and other interfaces. \
                Sign in at `POST /api/login` and send the `thoric_session` cookie it sets; a change \
                (POST, PUT or DELETE) also needs `X-Thoric-Request: 1`, and an Origin header, when \
                one is sent, must match the host. With `AUTH_MODE=none` no session is needed, and \
                with `MCP_AUTH_MODE=none` no MCP token. `docs/API.md` describes what the JSON \
                answers hold.",
            "license": {"name": env!("CARGO_PKG_LICENSE"), "identifier": env!("CARGO_PKG_LICENSE")},
        },
        "tags": TAGS
            .iter()
            .map(|(name, description)| json!({"name": name, "description": description}))
            .collect::<Vec<_>>(),
        "paths": paths,
        "components": {
            "schemas": schemas,
            "responses": ERRORS
                .iter()
                .map(|(_, name, description)| {
                    let response = json!({
                        "description": description,
                        "headers": {REQUEST_ID: {"$ref": "#/components/headers/RequestId"}},
                        "content": {"application/json": {"schema": error}},
                    });
                    (name.to_string(), response)
                })
                .collect::<Map<_, _>>(),
            "headers": {
                "RequestId": {
                    "description": "The request's ID, which the manager's log line for it names as `request_id`: the one the request sent, or a new UUID.",
                    "schema": {"type": "string"},
                },
            },
            "parameters": {
                "RequestId": {
                    "name": REQUEST_ID,
                    "in": "header",
                    "required": false,
                    "description": "An ID of the caller's own for the request, used instead of a new UUID when it is up to 128 letters, digits, `-`, `_`, `.` and `:`.",
                    "schema": {"type": "string"},
                },
                "Request": {
                    "name": "X-Thoric-Request",
                    "in": "header",
                    "required": true,
                    "description": "Marks a change as the dashboard's own or a script's, which a page of another site cannot send.",
                    "schema": {"const": "1"},
                },
            },
            "securitySchemes": {
                "session": {
                    "type": "apiKey",
                    "in": "cookie",
                    "name": "thoric_session",
                    "description": "The session `POST /api/login` starts; it lasts 12 hours.",
                },
                "mcp": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "The installation's MCP token, `MCP_TOKEN`.",
                },
                "endpoint": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "The MCP endpoint's own token, or the installation's MCP token.",
                },
                "metrics": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "The installation's metrics token, `METRICS_TOKEN`, which opens `/metrics` alone.",
                },
            },
        },
    })
}

/// A path as OpenAPI writes it: a catch-all `{*rest}` is a parameter like any other.
fn template(path: &str) -> String {
    path.replace("{*", "{")
}

/// The parameters in a path, in order, and whether each takes the rest of it.
fn path_parameters(path: &str) -> impl Iterator<Item = (&str, bool)> {
    path.split('/').filter_map(|segment| {
        let name = segment.strip_prefix('{')?.strip_suffix('}')?;
        Some(match name.strip_prefix('*') {
            Some(rest) => (rest, true),
            None => (name, false),
        })
    })
}

/// A unique name for each operation, from its method and path: `postContainersByIdFilesRead`.
fn operation_id(method: &str, path: &str) -> String {
    let mut id = method.to_ascii_lowercase();
    let segments = path.trim_start_matches('/').split('/');
    for (index, segment) in segments.enumerate() {
        if index == 0 && segment == "api" {
            continue;
        }
        let word = match segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            Some(name) => {
                id.push_str("By");
                name.trim_start_matches('*')
            }
            None if segment.is_empty() => "index",
            None => segment,
        };
        for part in word.split(|c: char| !c.is_ascii_alphanumeric()) {
            let mut chars = part.chars();
            if let Some(first) = chars.next() {
                id.push(first.to_ascii_uppercase());
                id.push_str(chars.as_str());
            }
        }
    }
    id
}

/// What a path parameter names, by its name and the path it is in.
fn path_parameter(path: &str, name: &str, rest: bool) -> Value {
    let (description, schema) = match name {
        _ if rest => (
            "The rest of the path, slashes included, passed on as it is.",
            json!({"type": "string"}),
        ),
        "id" if path.starts_with("/api/containers/") => (
            "The container's Thoric ID, not a Docker container ID.",
            json!({"type": "string"}),
        ),
        "id" if path.starts_with("/api/images/") => (
            "The image's `id` in the image list: a saved image's record or a catalog image.",
            json!({"type": "string"}),
        ),
        "id" if path.starts_with("/api/jobs/") => ("The job's ID.", json!({"type": "string"})),
        "id" => ("The request's ID.", json!({"type": "string"})),
        "name" if path.starts_with("/api/volumes/") => {
            ("The named volume.", json!({"type": "string"}))
        }
        "name" if path.contains("/mcp/servers/") => {
            ("The desktop MCP server.", json!({"type": "string"}))
        }
        "name" => ("The MCP endpoint.", json!({"type": "string"})),
        "application" => (
            "The application's ID, from launching it.",
            json!({"type": "string"}),
        ),
        "terminal" => (
            "The terminal's ID, from opening it.",
            json!({"type": "string"}),
        ),
        "recording" => (
            "The recording's ID, from starting it.",
            json!({"type": "string"}),
        ),
        "window" => ("The X11 window ID.", json!({"type": "string"})),
        "direction" => (
            "`output` hears the desktop, `input` speaks into its microphone.",
            json!({"type": "string", "enum": ["output", "input"]}),
        ),
        "port" => (
            "A port inside the desktop that the container's `ports` setting lists.",
            json!({"type": "integer", "minimum": 1, "maximum": 65535}),
        ),
        "token" => (
            "The transfer link's one-time token.",
            json!({"type": "string"}),
        ),
        _ => ("", json!({"type": "string"})),
    };
    let mut parameter = json!({"name": name, "in": "path", "required": true, "schema": schema});
    if !description.is_empty() {
        parameter["description"] = json!(description);
    }
    parameter
}

/// Each field of a query string's type as a parameter of its own.
fn query_parameters(schema: Schema) -> Vec<Value> {
    let schema = schema.to_value();
    let required = schema["required"].as_array().cloned().unwrap_or_default();
    let Some(properties) = schema["properties"].as_object() else {
        return Vec::new();
    };
    properties
        .iter()
        .map(|(name, property)| {
            let mut property = property.clone();
            let description = property
                .as_object_mut()
                .and_then(|property| property.remove("description"));
            let mut parameter = json!({
                "name": name,
                "in": "query",
                "required": required.contains(&json!(name)),
                "schema": property,
            });
            if let Some(description) = description {
                parameter["description"] = description;
            }
            parameter
        })
        .collect()
}

fn operation(
    route: &Route,
    method: &str,
    generator: &mut SchemaGenerator,
    error: &Value,
    job: &Value,
) -> Value {
    let change = !matches!(method, "GET" | "HEAD" | "OPTIONS");
    let mut parameters: Vec<Value> = path_parameters(route.path)
        .map(|(name, rest)| path_parameter(route.path, name, rest))
        .collect();
    if let Some(query) = route.query {
        parameters.extend(query_parameters(query(generator)));
    }
    if (route.access == Access::Session && change) || route.access == Access::SignIn {
        parameters.push(json!({"$ref": "#/components/parameters/Request"}));
    }
    parameters.push(json!({"$ref": "#/components/parameters/RequestId"}));
    let security = match route.access {
        Access::Public | Access::SignIn | Access::TransferLink => json!([]),
        Access::Session => json!([{"session": []}]),
        Access::McpToken => json!([{"mcp": []}]),
        Access::EndpointToken => json!([{"endpoint": []}]),
        Access::MetricsToken => json!([{"metrics": []}]),
    };
    let mut operation = json!({
        "operationId": operation_id(method, route.path),
        "summary": route.summary,
        "tags": [route.tag],
        "security": security,
        "parameters": parameters,
    });
    let mut stream = None;
    match &route.body {
        Body::None => {}
        Body::Json { schema, required } => {
            operation["requestBody"] = json!({
                "required": required,
                "content": {"application/json": {"schema": schema(generator).to_value()}},
            });
        }
        Body::Bytes(media) => {
            // A relayed route passes on what it gets; only a route that reads the bytes
            // itself streams an upload.
            if !route.any {
                stream = Some("upload");
            }
            operation["requestBody"] = json!({
                "required": !route.any,
                "content": media
                    .iter()
                    .map(|media| (media.to_string(), json!({})))
                    .collect::<Map<_, _>>(),
            });
        }
    }
    let answer = match &route.reply {
        Reply::Json => json!({"200": {
            "description": "The answer `docs/API.md` describes.",
            "content": {"application/json": {"schema": {}}},
        }}),
        Reply::Typed(schema) => json!({"200": {
            "description": "OK",
            "content": {"application/json": {"schema": schema(generator).to_value()}},
        }}),
        Reply::Job => json!({"200": {
            "description": "The job was queued.",
            "content": {"application/json": {"schema": job}},
        }}),
        Reply::NoContent => json!({"204": {"description": "Done."}}),
        Reply::Text => json!({"200": {
            "description": "OK",
            "content": {"text/plain": {"schema": {"type": "string"}}},
        }}),
        Reply::Bytes(media) => {
            stream = Some("download");
            json!({"200": {
                "description": "The content, streamed as it is read.",
                "content": {*media: {}},
            }})
        }
        Reply::Events => {
            stream = Some("event-stream");
            json!({"200": {
                "description": "Server-sent events, one JSON object each, until the session ends.",
                "content": {"text/event-stream": {}},
            }})
        }
        Reply::WebSocket => {
            stream = Some("websocket");
            json!({"101": {
                "description": "Switching Protocols: the connection is a WebSocket from here on.",
            }})
        }
        Reply::Relayed => {
            stream = Some("relay");
            json!({"2XX": {"description": "What the service behind the route answers."}})
        }
    };
    let mut responses = answer;
    let signed_out = matches!(route.access, Access::Public | Access::TransferLink);
    for (status, name, _) in ERRORS {
        if !(signed_out && *status == "401") {
            responses[*status] = json!({"$ref": format!("#/components/responses/{name}")});
        }
    }
    if let Some(description) = route.unavailable {
        responses["503"] = json!({
            "description": description,
            "content": {"application/json": {"schema": {}}},
        });
    }
    responses["default"] = json!({
        "description": "Another error status that the route names, such as 413 or 429, with its message.",
        "content": {"application/json": {"schema": error}},
    });
    // Every answer carries the request's ID (`request_log`); the shared ones say so themselves.
    for response in responses.as_object_mut().unwrap().values_mut() {
        if response.get("$ref").is_none() {
            response["headers"] = json!({REQUEST_ID: {"$ref": "#/components/headers/RequestId"}});
        }
    }
    operation["responses"] = responses;
    if let Some(stream) = stream {
        operation["x-thoric-stream"] = json!(stream);
    }
    operation
}

#[cfg(test)]
#[path = "openapi_tests.rs"]
mod tests;
