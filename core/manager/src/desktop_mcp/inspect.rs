//! The MCP page's inspector: a WebSocket at `/api/mcp/inspect` that joins the dashboard to one
//! MCP server for as long as the page keeps it open. The browser is the MCP client; the manager
//! carries its JSON-RPC messages, one per text frame, without reading them.
//!
//! The first frame names the server: a desktop server the manager knows, one the dashboard tries
//! before saving it, an MCP endpoint as an agent with its token sees it, or the administrator's
//! `/mcp`. A desktop server runs in a `docker exec` of the inspector's own as the desktop user,
//! started as an endpoint starts it, so an inspector neither shares nor disturbs an agent's
//! connection. When the socket closes, its stdin closes, and the `docker exec` is killed if it has
//! not ended a moment later. An endpoint and `/mcp` are served inside the manager by the handlers
//! their routes use, without a bearer token: the dashboard's session already opens everything
//! they offer.
//!
//! Every later frame from the browser is one JSON-RPC message. Every frame to the browser is a
//! JSON object whose `type` says what it carries: `ready` with the `server` that was started,
//! `message` with a JSON-RPC message as the server wrote it, `stdout` and `stderr` with a line of
//! a desktop server's output that is not a message, `error` when the server cannot be opened or a
//! frame was refused, and `closed` with the `reason` the server stopped, after which the socket
//! closes.
use super::{
    client::{self, Bounded, Started},
    endpoints::{ENDPOINTS, Endpoint},
    gateway::EndpointMcp,
    registry::{Declaration, SERVERS, ServerRecord, ServerSpec, Source},
};
use crate::{
    App,
    auth::origin_matches,
    error::{Error, Result},
    mcp::{self, ThoricMcp},
    model::Container,
};
use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::Response,
};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use rmcp::{ServerHandler, ServiceExt};
use serde::{Deserialize, de::IgnoredAny};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    process::Child,
    sync::mpsc,
    task::JoinHandle,
};
use tokio_util::sync::{CancellationToken, DropGuard};

/// Inspectors open at once; a desktop server's holds a `docker exec`.
pub(super) const INSPECTORS: usize = 8;
/// How long the browser has to name the server.
const OPEN_LIMIT: Duration = Duration::from_secs(30);
/// The longest line of output that is not a message passed on; the rest is cut.
const LINE_SHOWN: usize = 8 * 1024;
/// How often the socket is pinged, so one whose browser went away is noticed.
const PING: Duration = Duration::from_secs(30);
/// How long a server has to stop once its stdin closed.
const EXIT_LIMIT: Duration = Duration::from_secs(2);
/// The pipe between the browser and a server served inside the manager.
const PIPE: usize = 64 * 1024;

/// The server the first frame names, by its `kind`.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Target {
    /// A desktop server the manager knows.
    Server { container: String, name: String },
    /// A desktop server the dashboard tries in a running container before saving it.
    Trial {
        container: String,
        server: Declaration,
    },
    /// An MCP endpoint, as an agent with its token sees it.
    Endpoint { name: String },
    /// The administrator's `/mcp`.
    Administrator,
}

type Reader = Box<dyn AsyncRead + Send + Unpin>;
type Writer = Box<dyn AsyncWrite + Send + Unpin>;

/// A started server as the relay sees it.
pub(super) struct Peer {
    /// What the server writes, one JSON-RPC message a line.
    output: Reader,
    /// What the server reads.
    input: Writer,
    /// A desktop server's stderr.
    errors: Option<Reader>,
    end: End,
}

/// What runs the server, and so what stops when the inspector closes.
enum End {
    /// A desktop server's `docker exec`.
    Process(Box<Child>),
    /// An endpoint's or `/mcp`'s handler, served until it is cancelled.
    Service {
        task: JoinHandle<std::result::Result<(), String>>,
        _stop: DropGuard,
    },
}
impl End {
    /// Stop the server if it still runs, and say why it stopped. Its stdin is closed by now, and
    /// a server that reads to the end stops on its own.
    async fn finish(self) -> String {
        match self {
            End::Process(mut child) => match tokio::time::timeout(EXIT_LIMIT, child.wait()).await {
                Ok(Ok(status)) => match status.code() {
                    Some(0) => "The server exited".into(),
                    Some(code) => format!("The server exited with code {code}"),
                    None => "The server was stopped by a signal".into(),
                },
                _ => {
                    let _ = child.kill().await;
                    "The server closed its output".into()
                }
            },
            End::Service { task, _stop } => match tokio::time::timeout(EXIT_LIMIT, task).await {
                Ok(Ok(Err(reason))) => reason,
                _ => "The server ended the session".into(),
            },
        }
    }
}

/// `/api/mcp/inspect`: the inspector's WebSocket, from the dashboard's own origin.
pub async fn inspect(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    if !origin_matches(&headers) {
        return Err(Error(
            StatusCode::FORBIDDEN,
            "WebSocket origin rejected".into(),
        ));
    }
    let token = crate::auth::cookie(&headers);
    Ok(ws
        .max_message_size(mcp::BODY_LIMIT)
        .on_upgrade(move |socket| async move {
            let inspector = session(app.clone(), socket);
            tokio::select! { () = app.session_revoked(token) => {}, () = inspector => {} }
        }))
}

/// One inspector: name the server, start it and carry its messages until either side ends.
async fn session(app: Arc<App>, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let Ok(_permit) = app.desktop_mcp.inspectors.clone().try_acquire_owned() else {
        return refuse(
            sink,
            &format!("{INSPECTORS} inspectors are open already; close one first"),
        )
        .await;
    };
    let target = first_frame(&mut stream).await;
    // A desktop server's session uses its container while it is open; an endpoint's or `/mcp`'s
    // calls count as their tool calls do.
    let _using = match &target {
        Ok(Target::Server { container, .. } | Target::Trial { container, .. }) => app
            .container(container)
            .ok()
            .map(|record| app.lifecycle.open(&record.id)),
        _ => None,
    };
    let opened = match target {
        Ok(target) => open(&app, target).await.map_err(|error| error.1),
        Err(reason) => Err(reason),
    };
    match opened {
        Ok((server, peer)) => {
            let ready = json!({"type": "ready", "server": server}).to_string();
            // A server not passed on is stopped as `peer` drops.
            if sink.send(Message::Text(ready.into())).await.is_ok() {
                relay(sink, stream, peer).await;
            }
        }
        Err(reason) => refuse(sink, &reason).await,
    }
}

/// Say why the inspector cannot open, and close the socket.
async fn refuse<S: Sink<Message> + Unpin>(mut sink: S, reason: &str) {
    let _ = sink.send(Message::Text(error_frame(reason).into())).await;
    let _ = sink.close().await;
}

/// The server the browser's first frame names, within [`OPEN_LIMIT`].
async fn first_frame<R, E>(stream: &mut R) -> std::result::Result<Target, String>
where
    R: Stream<Item = std::result::Result<Message, E>> + Unpin,
{
    let first = tokio::time::timeout(OPEN_LIMIT, stream.next())
        .await
        .map_err(|_| "The first frame names the server to inspect".to_owned())?;
    match first {
        Some(Ok(Message::Text(text))) => target(&text),
        _ => Err("The first frame names the server to inspect".into()),
    }
}

/// Read the first frame's target.
pub(super) fn target(text: &str) -> std::result::Result<Target, String> {
    serde_json::from_str(text).map_err(|e| format!("The first frame names the server: {e}"))
}

/// Start the server a target names, and describe it for the `ready` frame.
pub(super) async fn open(app: &Arc<App>, target: Target) -> Result<(Value, Peer)> {
    match target {
        Target::Server { container, name } => {
            let container = app.container(&container)?;
            let record: ServerRecord = app
                .db
                .get(SERVERS, &ServerRecord::key(&container.id, &name))
                .map_err(|_| {
                    Error(
                        StatusCode::NOT_FOUND,
                        format!("This container has no MCP server called {name}"),
                    )
                })?;
            let spec = record.spec.as_ref().ok_or_else(|| {
                Error(
                    StatusCode::CONFLICT,
                    format!(
                        "The server {name} has no valid declaration: {}",
                        record.error.as_deref().unwrap_or_default()
                    ),
                )
            })?;
            let server = json!({
                "kind": "server",
                "container_id": container.id,
                "container_name": container.spec.name,
                "name": name,
                "source": record.source,
                "transport": spec.transport(),
            });
            Ok((server, desktop(app, &container, spec, record.source).await?))
        }
        Target::Trial { container, server } => {
            server
                .validate()
                .map_err(|e| Error(StatusCode::BAD_REQUEST, format!("{e:#}")))?;
            let container = app.container(&container)?;
            let described = json!({
                "kind": "trial",
                "container_id": container.id,
                "container_name": container.spec.name,
                "name": server.name,
                "transport": server.spec.transport(),
            });
            // Tried before it is added to the desktop user's folder, where it then goes.
            Ok((
                described,
                desktop(app, &container, &server.spec, Source::Container).await?,
            ))
        }
        // An endpoint answers whether or not the root /mcp is on, here as on its route.
        Target::Endpoint { name } => {
            app.db.get::<Endpoint>(ENDPOINTS, &name).map_err(|_| {
                Error(
                    StatusCode::NOT_FOUND,
                    format!("No MCP endpoint is called {name}"),
                )
            })?;
            let server = json!({"kind": "endpoint", "name": name, "path": format!("/mcp/{name}")});
            Ok((server, serve(EndpointMcp::new(app.clone(), name))))
        }
        Target::Administrator => {
            root_enabled(app)?;
            let server = json!({"kind": "administrator", "path": "/mcp"});
            Ok((server, serve(ThoricMcp::new(app.clone()))))
        }
    }
}

/// `/mcp` answers only while the root MCP is on, here as on its route.
fn root_enabled(app: &App) -> Result<()> {
    if app.config.root_mcp_enabled {
        Ok(())
    } else {
        Err(Error(
            StatusCode::NOT_FOUND,
            "The root MCP is turned off (ROOT_MCP_ENABLED=false)".into(),
        ))
    }
}

/// Start a desktop server in a `docker exec` of the inspector's own.
async fn desktop(
    app: &App,
    container: &Container,
    spec: &ServerSpec,
    source: Source,
) -> Result<Peer> {
    let Started {
        child,
        stdin,
        stdout,
        stderr,
    } = client::start(app, container, spec, source).await?;
    Ok(Peer {
        output: Box::new(stdout),
        input: Box::new(stdin),
        errors: Some(Box::new(stderr)),
        end: End::Process(Box::new(child)),
    })
}

/// Serve a handler inside the manager, joined to the browser by a pipe.
fn serve<H: ServerHandler>(handler: H) -> Peer {
    let (ours, theirs) = tokio::io::duplex(PIPE);
    let stop = CancellationToken::new();
    let cancelled = stop.clone();
    let task = tokio::spawn(async move {
        let serving = async {
            let running = handler
                .serve(tokio::io::split(theirs))
                .await
                .map_err(|e| format!("The MCP handshake failed: {e}"))?;
            running.waiting().await.map(drop).map_err(|e| e.to_string())
        };
        tokio::select! { () = cancelled.cancelled() => Ok(()), result = serving => result }
    });
    let (output, input) = tokio::io::split(ours);
    Peer {
        output: Box::new(output),
        input: Box::new(input),
        errors: None,
        end: End::Service {
            task,
            _stop: stop.drop_guard(),
        },
    }
}

/// Carry frames between the browser and a started server until either side ends. When the server
/// stops first, the browser hears why before the socket closes.
pub(super) async fn relay<S, R, E>(mut sink: S, mut stream: R, peer: Peer)
where
    S: Sink<Message> + Unpin,
    R: Stream<Item = std::result::Result<Message, E>> + Unpin,
{
    let Peer {
        output,
        input,
        errors,
        end,
    } = peer;
    let (frames, mut queue) = mpsc::channel::<String>(64);
    let send = async {
        let mut ping = tokio::time::interval(PING);
        ping.tick().await;
        loop {
            let message = tokio::select! {
                frame = queue.recv() => match frame {
                    Some(frame) => Message::Text(frame.into()),
                    None => break,
                },
                _ = ping.tick() => Message::Ping(Default::default()),
            };
            if sink.send(message).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    };
    let work = async move {
        let stderr = errors.map(|errors| {
            let frames = frames.clone();
            tokio::spawn(async move {
                let _ = carry(errors, &frames, |line| Some(line_frame("stderr", line))).await;
            })
        });
        let server_stopped = tokio::select! {
            () = carry_output(output, &frames) => true,
            () = carry_input(&mut stream, input, &frames) => false,
        };
        let reason = end.finish().await;
        match stderr {
            // What the server wrote last on stderr comes before the reason it stopped. A stderr
            // still open after that holds a sender, which would keep the socket open.
            Some(mut stderr) if server_stopped => {
                if tokio::time::timeout(EXIT_LIMIT, &mut stderr).await.is_err() {
                    stderr.abort();
                }
            }
            Some(stderr) => stderr.abort(),
            None => {}
        }
        if server_stopped {
            let _ = frames
                .send(json!({"type": "closed", "reason": reason}).to_string())
                .await;
        }
    };
    tokio::join!(send, work);
}

/// Pass the server's output on until it ends, or until a line is longer than a message may be.
async fn carry_output(output: Reader, frames: &mpsc::Sender<String>) {
    if let Err(error) = carry(output, frames, output_frame).await {
        let _ = frames.send(error_frame(&error.to_string())).await;
    }
}

/// Read `reader` a line at a time and send each line's frame, until it ends, a line is longer
/// than [`client::Bounded`] lets through, or the browser is gone.
async fn carry(
    reader: Reader,
    frames: &mpsc::Sender<String>,
    frame: impl Fn(&[u8]) -> Option<String>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(Bounded::new(reader));
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line).await? == 0 {
            return Ok(());
        }
        if let Some(frame) = frame(&line)
            && frames.send(frame).await.is_err()
        {
            return Ok(());
        }
    }
}

/// Pass the browser's frames to the server, each message on a line of its own, until the browser
/// closes the socket. Once the server reads no more, its messages are refused instead.
async fn carry_input<R, E>(stream: &mut R, mut input: Writer, frames: &mpsc::Sender<String>)
where
    R: Stream<Item = std::result::Result<Message, E>> + Unpin,
{
    let mut reading = true;
    while let Some(Ok(message)) = stream.next().await {
        let refused = match message {
            Message::Text(text) => match checked(&text) {
                Ok(()) if reading => {
                    reading = write_line(&mut input, text.as_bytes()).await.is_ok();
                    (!reading).then_some("The server no longer reads its input")
                }
                Ok(()) => Some("The server no longer reads its input"),
                Err(reason) => Some(reason),
            },
            Message::Binary(_) => Some("Messages are sent as text frames"),
            Message::Close(_) => return,
            Message::Ping(_) | Message::Pong(_) => None,
        };
        if let Some(reason) = refused {
            let _ = frames.send(error_frame(reason)).await;
        }
    }
}

async fn write_line(input: &mut Writer, message: &[u8]) -> std::io::Result<()> {
    input.write_all(message).await?;
    input.write_all(b"\n").await?;
    input.flush().await
}

/// Whether a frame from the browser is one JSON-RPC message, or a batch, on one line.
pub(super) fn checked(text: &str) -> std::result::Result<(), &'static str> {
    if text.contains(['\n', '\r']) {
        return Err("A message is sent on one line, as JSON.stringify writes it");
    }
    if !text.trim_start().starts_with(['{', '['])
        || serde_json::from_str::<IgnoredAny>(text).is_err()
    {
        return Err("A frame holds one JSON-RPC message");
    }
    Ok(())
}

/// The frame for a line of the server's output: a message when it is JSON, a line of stdout
/// when it is not, and none when it is blank.
pub(super) fn output_frame(line: &[u8]) -> Option<String> {
    let line = line.trim_ascii();
    if line.is_empty() {
        return None;
    }
    match std::str::from_utf8(line) {
        // JSON that parses has nothing after its value, so it stands in the frame as it is.
        Ok(text) if serde_json::from_str::<IgnoredAny>(text).is_ok() => {
            Some(format!(r#"{{"type":"message","message":{text}}}"#))
        }
        _ => Some(line_frame("stdout", line)),
    }
}

/// A line of output that is not a message, cut at [`LINE_SHOWN`] bytes.
pub(super) fn line_frame(kind: &str, line: &[u8]) -> String {
    let text = String::from_utf8_lossy(line.trim_ascii_end());
    let mut end = text.len().min(LINE_SHOWN);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    json!({"type": kind, "line": &text[..end], "truncated": end < text.len()}).to_string()
}

pub(super) fn error_frame(message: &str) -> String {
    json!({"type": "error", "message": message}).to_string()
}

#[cfg(test)]
#[path = "inspect_tests.rs"]
mod tests;
