//! The agent activity log: one row for every MCP tool call, on `/mcp` and on every endpoint, and
//! for every transfer link that is used, so the dashboard can show which agent did what.
//!
//! A call is recorded when it ends: who made it (`/mcp` or an endpoint, the credential it showed
//! and the name its client gives itself), the tool, the container it worked on with that
//! container's name at the time, its arguments as [`mask`] keeps them, how it ended and how long
//! it took. A result is never kept beyond a short error text, so whatever a tool returns, such as
//! a new endpoint's token, never reaches the log. The rows live in the manager's SQLite, the newest
//! [`KEPT`] of them, and each call also writes one line without its arguments to the manager's own
//! log.
//!
//! Recording never fails or holds up a call: the row is written on a blocking thread once the
//! answer is ready, and a row that cannot be written is logged and dropped.
//!
//! Not recorded: MCP messages other than `tools/call`, the dashboard's own HTTP requests, the
//! desktop ports a container proxies, which carry a service's traffic rather than discrete
//! actions, and the desktop servers the MCP page's inspector starts in a `docker exec` of their
//! own. The inspector's calls to `/mcp` and to an endpoint run through the same handlers and are
//! recorded with the dashboard as their credential.
mod mask;
mod store;

pub(crate) use store::Filter;
pub(crate) use store::opened;

use crate::{App, error::Result};
use axum::{
    Json,
    body::{Body, to_bytes},
    extract::{Query, State},
    http::{header, request::Parts},
    response::Response,
};
use rmcp::{
    ErrorData, RoleServer,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ToolAnnotations,
    },
    service::RequestContext,
};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::{
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;

/// How many calls are kept, newest first.
pub(crate) const KEPT: usize = store::KEPT;
/// The longest tool, server or client name kept.
const NAME: usize = 128;
/// The longest error text kept.
const ERROR: usize = 300;
/// The longest error response of a transfer that is read for its message.
const ERROR_BODY: usize = 64 * 1024;

/// How a caller proved who it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Credential {
    /// The endpoint's own bearer token.
    EndpointToken,
    /// The administrator's MCP token, which opens `/mcp` and every endpoint.
    McpToken,
    /// Nothing: `MCP_AUTH_MODE=none` asks for no token.
    None,
    /// The dashboard's session, through the MCP page's inspector.
    Inspector,
    /// A one-time link `create_transfer` issued.
    Link,
}
impl Credential {
    pub fn id(self) -> &'static str {
        match self {
            Credential::EndpointToken => "endpoint_token",
            Credential::McpToken => "mcp_token",
            Credential::None => "none",
            Credential::Inspector => "inspector",
            Credential::Link => "link",
        }
    }
}

/// Who made a call: `/mcp` (no endpoint) or an endpoint by its name, which is its identity, with
/// the credential it showed and the name and version its MCP client gives itself.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Caller {
    pub endpoint: Option<String>,
    pub credential: Credential,
    pub client: Option<String>,
}
impl Caller {
    /// The caller of a request to `/mcp` (`endpoint` is `None`) or to an endpoint.
    ///
    /// A request that arrived over HTTP carries its parts, which rmcp hands every handler. The
    /// only calls without them are the inspector's, served inside the manager to the dashboard.
    pub(crate) fn of(
        app: &App,
        endpoint: Option<&str>,
        context: &RequestContext<RoleServer>,
    ) -> Self {
        let parts = context.extensions.get::<Parts>();
        let credential = match (parts, &app.config.mcp_token) {
            (None, _) => Credential::Inspector,
            (Some(_), None) => Credential::None,
            // `/mcp` opens with the administrator's token alone.
            (Some(_), Some(_)) if endpoint.is_none() => Credential::McpToken,
            (Some(parts), Some(administrator)) => {
                let bearer = parts
                    .headers
                    .get(header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer "));
                if bearer.is_some_and(|token| {
                    !administrator.is_empty()
                        && bool::from(token.as_bytes().ct_eq(administrator.as_bytes()))
                }) {
                    Credential::McpToken
                } else {
                    Credential::EndpointToken
                }
            }
        };
        Self {
            endpoint: endpoint.map(str::to_owned),
            credential,
            client: client(context.client_info()),
        }
    }
}

/// The name and version a client gives itself. A request of an older protocol version on `/mcp`,
/// which keeps no session, has none, and rmcp stands its own name in for it.
fn client(info: Option<Implementation>) -> Option<String> {
    let info = info?;
    let standin = Implementation::default();
    if info.name.is_empty() || (info.name == standin.name && info.version == standin.version) {
        return None;
    }
    let text = format!("{} {}", info.name, info.version);
    Some(clip(text.trim(), NAME))
}

/// How much a call can change, from its tool's annotations as MCP reads them: a tool that does not
/// say otherwise may change and destroy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Read,
    Change,
    Destructive,
}
impl Kind {
    pub fn of(annotations: Option<&ToolAnnotations>) -> Self {
        match annotations {
            Some(a) if a.read_only_hint == Some(true) => Kind::Read,
            Some(a) if a.destructive_hint == Some(false) => Kind::Change,
            _ => Kind::Destructive,
        }
    }
    /// A desktop server's tool. Its annotations come from inside the desktop, so a server may
    /// say that a tool destroys, but not that it only reads: such a claim counts as a change.
    pub fn of_server(annotations: Option<&ToolAnnotations>) -> Self {
        match Kind::of(annotations) {
            Kind::Read => Kind::Change,
            kind => kind,
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Kind::Read => "read",
            Kind::Change => "change",
            Kind::Destructive => "destructive",
        }
    }
}

/// What a call reached: a Thoric tool, a desktop server's tool or a transfer link's operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Tool,
    Server,
    Transfer,
}
impl Source {
    pub fn id(self) -> &'static str {
        match self {
            Source::Tool => "tool",
            Source::Server => "server",
            Source::Transfer => "transfer",
        }
    }
}

/// How a call ended: with a result, with a result the tool marked as an error, or refused before
/// the tool ran, such as an unknown tool or arguments its schema does not take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Ok,
    Error,
    Refused,
}
impl Outcome {
    pub fn id(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Error => "error",
            Outcome::Refused => "refused",
        }
    }
}

/// One recorded call, as the database keeps it and `GET /api/activity` lists it.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(crate) struct Entry {
    pub id: i64,
    /// When the call began, in Unix milliseconds.
    pub at: u64,
    /// The endpoint, or `None` for `/mcp`.
    pub endpoint: Option<String>,
    pub credential: String,
    pub client: Option<String>,
    pub source: String,
    /// A Thoric tool, a desktop server's tool by its own name, or a transfer's operation.
    pub tool: String,
    pub server: Option<String>,
    pub container_id: Option<String>,
    /// The container's name when the call began.
    pub container_name: Option<String>,
    pub kind: String,
    pub arguments: Value,
    /// The arguments that say most about the call, on one line.
    pub summary: String,
    pub outcome: String,
    pub error: Option<String>,
    pub duration_ms: u64,
    /// The job a call started, which `get_job` and the job panel follow.
    pub job_id: Option<String>,
}

tokio::task_local! {
    /// The caller of the tool call this task runs, for what it hands on, such as a transfer link.
    static CALLER: Caller;
}

/// Run `work` as `caller`'s, so what it issues, such as a transfer link, knows who asked.
pub(crate) async fn as_caller<F: Future>(caller: Caller, work: F) -> F::Output {
    CALLER.scope(caller, work).await
}

/// The caller of the tool call that runs now, if one does.
pub(crate) fn current_caller() -> Option<Caller> {
    CALLER.try_with(Caller::clone).ok()
}

/// A call on its way: what is known of it when it begins, recorded when it ends.
pub(crate) struct Call {
    started: Instant,
    at: u64,
    caller: Caller,
    source: Source,
    tool: String,
    server: Option<String>,
    container_id: Option<String>,
    container_name: Option<String>,
    kind: Kind,
    arguments: Map<String, Value>,
    /// Refused before its tool ran, although the answer is a tool error the agent can correct.
    refused: bool,
}

impl Call {
    /// A tool call as it arrives. Until a handler says more, it is a Thoric tool classified by
    /// the annotations `/mcp` gives it, on `/mcp` on the container its `container_id` names. An
    /// endpoint binds its containers itself, so what its caller names is not taken for one.
    pub(crate) fn start(app: &App, caller: Caller, request: &CallToolRequestParams) -> Self {
        let arguments = request.arguments.clone().unwrap_or_default();
        let container = arguments
            .get("container_id")
            .and_then(Value::as_str)
            .filter(|_| caller.endpoint.is_none())
            .map(str::to_owned);
        let mut call = Self {
            started: Instant::now(),
            at: now_ms(),
            caller,
            source: Source::Tool,
            tool: clip(&request.name, NAME),
            server: None,
            container_id: None,
            container_name: None,
            kind: crate::mcp::listed_tool(&request.name)
                .map_or(Kind::Change, |tool| Kind::of(tool.annotations.as_ref())),
            arguments,
            refused: false,
        };
        call.on_container(app, container);
        call
    }

    /// A transfer link's operation as its request arrives, with the caller whose tool call issued
    /// the link; the credential is the link.
    pub(crate) fn transfer(
        app: &App,
        issuer: Option<Caller>,
        operation: &str,
        arguments: Map<String, Value>,
        kind: Kind,
    ) -> Self {
        let (endpoint, client) =
            issuer.map_or((None, None), |issuer| (issuer.endpoint, issuer.client));
        let caller = Caller {
            endpoint,
            credential: Credential::Link,
            client,
        };
        let container = arguments
            .get("container_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut call = Self {
            started: Instant::now(),
            at: now_ms(),
            caller,
            source: Source::Transfer,
            tool: operation.to_owned(),
            server: None,
            container_id: None,
            container_name: None,
            kind,
            arguments,
            refused: false,
        };
        call.on_container(app, container);
        call
    }

    /// The container the call works on, with its name as it is now.
    pub(crate) fn on_container(&mut self, app: &App, id: Option<String>) {
        self.container_name = id
            .as_deref()
            .and_then(|id| app.container(id).ok())
            .map(|container| container.spec.name);
        self.container_id = id.map(|id| clip(&id, NAME));
    }

    /// The call reaches a desktop server's tool, by its own name and with its server's annotations.
    pub(crate) fn on_server(
        &mut self,
        server: &str,
        tool: &str,
        annotations: Option<&ToolAnnotations>,
    ) {
        self.source = Source::Server;
        self.server = Some(clip(server, NAME));
        self.tool = clip(tool, NAME);
        self.kind = Kind::of_server(annotations);
    }

    /// The call is answered without running its tool, such as for an argument its schema does not
    /// take. The answer is a tool error, which the agent reads and can correct, and the log still
    /// records a refusal.
    pub(crate) fn refused(&mut self) {
        self.refused = true;
    }

    /// Record how a tool call ended.
    pub(crate) fn finish(
        self,
        app: &Arc<App>,
        response: &std::result::Result<CallToolResponse, ErrorData>,
    ) {
        let (outcome, error, job) = match response {
            Err(error) => (Outcome::Refused, Some(error.message.to_string()), None),
            Ok(CallToolResponse::Complete(result)) if result.is_error == Some(true) => {
                let outcome = if self.refused {
                    Outcome::Refused
                } else {
                    Outcome::Error
                };
                (outcome, Some(error_text(result)), None)
            }
            Ok(CallToolResponse::Complete(result)) => (Outcome::Ok, None, job_of(result)),
            Ok(_) => (Outcome::Ok, None, None),
        };
        self.record(app, outcome, error, job);
    }

    /// Record how a transfer ended, from the answer its route gave, which is passed on unchanged.
    /// An error's message is read from its JSON body; a download streams on after this.
    pub(crate) async fn finish_transfer(self, app: &Arc<App>, response: Response) -> Response {
        let job = response
            .headers()
            .get("x-job-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        if response.status().is_success() {
            self.record(app, Outcome::Ok, None, job);
            return response;
        }
        let status = response.status();
        let json = response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|v| v.as_bytes().starts_with(b"application/json"));
        if !json {
            self.record(
                app,
                Outcome::Error,
                Some(format!("HTTP {}", status.as_u16())),
                job,
            );
            return response;
        }
        let (mut parts, body) = response.into_parts();
        // The body is passed on as read; one too long to read is passed on empty.
        parts.headers.remove(header::CONTENT_LENGTH);
        let bytes = to_bytes(body, ERROR_BODY).await.unwrap_or_default();
        let message = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|body| body["error"].as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
        self.record(app, Outcome::Error, Some(message), job);
        Response::from_parts(parts, Body::from(bytes))
    }

    /// The row this call makes, its arguments masked and its error cut and stripped of them.
    fn entry(self, outcome: Outcome, error: Option<String>, job_id: Option<String>) -> Entry {
        let masked = mask::mask(self.source, &self.tool, &self.arguments);
        Entry {
            id: 0,
            at: self.at,
            endpoint: self.caller.endpoint,
            credential: self.caller.credential.id().into(),
            client: self.caller.client,
            source: self.source.id().into(),
            tool: self.tool,
            server: self.server,
            container_id: self.container_id,
            container_name: self.container_name,
            kind: self.kind.id().into(),
            summary: masked.summary.clone(),
            error: error.map(|error| clip(&masked.redact(&error), ERROR)),
            arguments: masked.arguments,
            outcome: outcome.id().into(),
            duration_ms: u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            job_id: job_id.map(|id| clip(&id, NAME)),
        }
    }

    fn record(
        self,
        app: &Arc<App>,
        outcome: Outcome,
        error: Option<String>,
        job_id: Option<String>,
    ) {
        app.metrics.called(self.source, &self.tool, outcome);
        let entry = self.entry(outcome, error, job_id);
        // The arguments stay out of the manager's log, which more people may read than the dashboard.
        let caller = match &entry.endpoint {
            Some(name) => format!("/mcp/{name}"),
            None => "/mcp".into(),
        };
        let tool = match &entry.server {
            Some(server) => format!("{server}/{}", entry.tool),
            None => entry.tool.clone(),
        };
        let container = entry.container_name.as_deref();
        tracing::info!(
            caller = %caller,
            credential = %entry.credential,
            tool = %tool,
            container = %container.or(entry.container_id.as_deref()).unwrap_or("-"),
            kind = %entry.kind,
            outcome = %entry.outcome,
            ms = entry.duration_ms,
            job = %entry.job_id.as_deref().unwrap_or("-"),
            "Agent {}",
            if entry.source == "transfer" { "transfer" } else { "tool call" }
        );
        let app = app.clone();
        let write = move || {
            if let Err(error) = store::insert(&app.db, &entry) {
                tracing::warn!("Agent activity was not recorded: {error:#}");
            }
        };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => drop(runtime.spawn_blocking(write)),
            Err(_) => write(),
        }
    }
}

/// The text of a tool's error result, which Thoric's tools put in their first text block.
fn error_text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .find_map(|block| block.as_text().map(|text| text.text.clone()))
        .unwrap_or_else(|| "The tool reported an error".into())
}

/// The job a tool's result names, as Thoric's tools that start one do.
fn job_of(result: &CallToolResult) -> Option<String> {
    result
        .structured_content
        .as_ref()?
        .get("job_id")?
        .as_str()
        .map(str::to_owned)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// `text` cut at `limit` characters, without control characters a log line could be forged with.
pub(crate) fn clip(text: &str, limit: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}

/// `GET /api/activity`: the recorded calls, newest first, a page at a time.
pub async fn list(
    State(app): State<Arc<App>>,
    Query(filter): Query<store::Filter>,
) -> Result<Json<Value>> {
    filter.check()?;
    let limit = filter.limit();
    let items = store::query(&app.db, &filter)?;
    let next = (items.len() == limit)
        .then(|| items.last().map(|entry| entry.id))
        .flatten();
    Ok(Json(json!({"items": items, "next": next, "kept": KEPT})))
}

/// The recorded calls once at least `count` are written, newest first. Rows are written on a
/// blocking thread after a call answers, so a test waits for them here.
#[cfg(test)]
pub(crate) async fn recorded(app: &App, count: usize) -> Vec<Entry> {
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    let all = store::Filter {
        limit: Some(200),
        ..Default::default()
    };
    loop {
        let rows = store::query(&app.db, &all).unwrap();
        if rows.len() >= count {
            return rows;
        }
        assert!(
            Instant::now() < deadline,
            "{} of {count} calls were recorded: {rows:?}",
            rows.len()
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod tests;
