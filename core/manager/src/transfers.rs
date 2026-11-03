//! One-time transfer links, so an MCP client moves files of real size over HTTP.
//!
//! An MCP client holds the bearer token, which `/mcp` accepts and no other route does, and the
//! file tools carry at most 512 KiB of base64 per call through the model's conversation. The
//! `create_transfer` tool issues a link for one HTTP request instead: a random token bound to
//! one operation and its arguments, which expires a few minutes later and lives only in memory.
//! The request that presents it runs the same handler a dashboard request would, with its locks,
//! limits, job and progress. The token takes the place of both the session cookie and the
//! `X-Thoric-Request` header: that header guards against requests a browser sends with ambient
//! cookies, and a secret in the URL is nothing a browser adds on its own.
//!
//! Accepting the MCP token on the whole HTTP API would be simpler, but it would put a long-lived
//! administrator credential into curl commands and shell history, where a leak reaches every
//! route for as long as the token is configured. A leaked link is good for one request of one
//! operation, for minutes. Links are issued only over MCP, so with MCP disabled the route
//! answers nothing. A link that is used is recorded in the agent activity log, under the caller
//! whose tool call issued it.
use crate::{
    App, activity, apps, archives,
    auth::{self, Session},
    error::{Error, Result},
    files,
    model::{self, ImageRecord},
    uploads, volumes,
};
use axum::{
    body::Body,
    extract::{Path, Query, State, rejection::QueryRejection},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// How long a link waits for its request. A transfer that has begun runs to its end.
pub(crate) const LIFETIME: Duration = Duration::from_secs(300);
/// Unused links kept at most; an agent asks for one right before it transfers.
const LIMIT: usize = 64;
/// Where links point. Request tracing shows the path without the token.
pub(crate) const ROUTE: &str = "/api/transfers/";
/// The names `create_transfer` offers, one for each [`Operation`].
pub(crate) const OPERATIONS: [&str; 9] = [
    "container_upload",
    "container_download",
    "volume_upload",
    "volume_download",
    "volume_import",
    "volume_extract",
    "volume_export",
    "image_import",
    "image_export",
];

/// The one request a link authorizes, with every argument it runs with. Only an upload's size is
/// left to the request, which knows its body.
///
/// Each variant runs one existing handler. A route added later becomes transferable with a
/// variant here, its name in [`OPERATIONS`] and its arms in [`run`] and [`Operation::check`].
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Operation {
    /// `POST /api/containers/{id}/files`: into the desktop home, a writable volume or a compose folder.
    ContainerUpload {
        container_id: String,
        destination: String,
        name: String,
        #[serde(default)]
        replace: bool,
    },
    /// `GET /api/containers/{id}/files/download`: a file, or a folder as a `.tar.gz`.
    ContainerDownload { container_id: String, path: String },
    /// `POST /api/volumes/{name}/files`
    VolumeUpload {
        volume: String,
        destination: String,
        name: String,
        #[serde(default)]
        replace: bool,
    },
    /// `GET /api/volumes/{name}/files/download`: a file, or a folder as a `.tar.gz`.
    VolumeDownload { volume: String, path: String },
    /// `POST /api/volumes/import`: a `.tar.gz` or `.tar` into a new volume.
    VolumeImport { name: String },
    /// `POST /api/volumes/{name}/extract`: a `.tar.gz` or `.tar` into a new folder of a volume.
    VolumeExtract { volume: String, path: String },
    /// `GET /api/volumes/{name}/archive`
    VolumeExport { volume: String },
    /// `POST /api/images/import`
    ImageImport {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        description: Option<String>,
    },
    /// `GET /api/images/{id}/archive`
    ImageExport { image_id: String },
}

impl Operation {
    /// Whether the request carries the data in its body.
    fn sends(&self) -> bool {
        matches!(
            self,
            Self::ContainerUpload { .. }
                | Self::VolumeUpload { .. }
                | Self::VolumeImport { .. }
                | Self::VolumeExtract { .. }
                | Self::ImageImport { .. }
        )
    }

    fn name(&self) -> &'static str {
        match self {
            Self::ContainerUpload { .. } => "container_upload",
            Self::ContainerDownload { .. } => "container_download",
            Self::VolumeUpload { .. } => "volume_upload",
            Self::VolumeDownload { .. } => "volume_download",
            Self::VolumeImport { .. } => "volume_import",
            Self::VolumeExtract { .. } => "volume_extract",
            Self::VolumeExport { .. } => "volume_export",
            Self::ImageImport { .. } => "image_import",
            Self::ImageExport { .. } => "image_export",
        }
    }

    /// The arguments the link was issued with, for the agent activity log.
    fn arguments(&self) -> serde_json::Map<String, Value> {
        let mut arguments = match serde_json::to_value(self) {
            Ok(Value::Object(arguments)) => arguments,
            _ => Default::default(),
        };
        arguments.remove("operation");
        arguments
    }

    /// How much the operation changes: a download reads, an upload adds and one that replaces a
    /// file destroys what was there.
    fn kind(&self) -> activity::Kind {
        match self {
            Self::ContainerUpload { replace: true, .. }
            | Self::VolumeUpload { replace: true, .. } => activity::Kind::Destructive,
            _ if self.sends() => activity::Kind::Change,
            _ => activity::Kind::Read,
        }
    }

    /// Refuse arguments the request would refuse, as far as that is known without Docker, so no
    /// link is issued that cannot work. The request checks everything again.
    fn check(&self, app: &App) -> Result<()> {
        match self {
            Self::ContainerUpload {
                container_id,
                destination,
                name,
                ..
            } => uploads::validate_target(&app.container(container_id)?, destination, name)?,
            Self::ContainerDownload { container_id, path } => {
                app.container(container_id)?;
                files::split_path(path)?;
            }
            Self::VolumeUpload {
                volume,
                destination,
                name,
                ..
            } => {
                model::volume_name(volume)?;
                uploads::file_name(name)?;
                apps::directory(destination)?;
            }
            Self::VolumeDownload { volume, path } => {
                model::volume_name(volume)?;
                files::split_path(path)?;
            }
            Self::VolumeImport { name: volume } | Self::VolumeExport { volume } => {
                model::volume_name(volume)?
            }
            Self::VolumeExtract { volume, path } => {
                model::volume_name(volume)?;
                apps::directory(path)?;
            }
            Self::ImageImport { .. } => {}
            Self::ImageExport { image_id } => {
                app.db.get::<ImageRecord>("image", image_id)?;
            }
        }
        Ok(())
    }
}

struct Link {
    operation: Operation,
    expires: Instant,
    /// Whose tool call issued it, so the agent activity log names who used it.
    issuer: Option<activity::Caller>,
}

/// The links issued and not used yet, in memory only: a restart ends them all.
#[derive(Default)]
pub(crate) struct Links(Mutex<HashMap<String, Link>>);

impl Links {
    /// A new link's token: 244 random bits, as a session's.
    pub(crate) fn issue(&self, operation: Operation, now: Instant) -> Result<String> {
        let mut links = self.0.lock().unwrap();
        links.retain(|_, link| link.expires > now);
        if links.len() >= LIMIT {
            return Err(Error(
                StatusCode::TOO_MANY_REQUESTS,
                format!(
                    "{LIMIT} transfer links are waiting to be used. Use them or let them expire."
                ),
            ));
        }
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        links.insert(
            token.clone(),
            Link {
                operation,
                expires: now + LIFETIME,
                issuer: activity::current_caller(),
            },
        );
        Ok(token)
    }

    /// The operation a link authorizes, as [`Links::take_issued`] spends it.
    #[cfg(test)]
    pub(crate) fn take(&self, token: &str, now: Instant) -> Option<Operation> {
        self.take_issued(token, now).map(|(operation, _)| operation)
    }

    /// The operation a link authorizes, with the caller whose tool call issued it. The link is
    /// removed whether or not it has expired, so the first request that presents the token spends
    /// it.
    fn take_issued(
        &self,
        token: &str,
        now: Instant,
    ) -> Option<(Operation, Option<activity::Caller>)> {
        let link = self.0.lock().unwrap().remove(token)?;
        (link.expires > now).then_some((link.operation, link.issuer))
    }
}

/// `create_transfer`: issue a link for one operation.
pub(crate) fn create(app: &App, operation: Operation) -> Result<Value> {
    operation.check(app)?;
    let (name, sends) = (operation.name(), operation.sends());
    let token = app.transfers.issue(operation, Instant::now())?;
    Ok(json!({
        "url": format!("{ROUTE}{token}"),
        "method": if sends { "PUT" } else { "GET" },
        "operation": name,
        "expires_in_seconds": LIFETIME.as_secs(),
        "expires_at": model::now() + LIFETIME.as_secs(),
    }))
}

/// What a request may add to its link: an upload's size, when it sends no Content-Length.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Size {
    size: Option<u64>,
}

/// `/api/transfers/{token}`: run the operation a link authorizes, once.
pub async fn transfer(
    State(app): State<Arc<App>>,
    Path(token): Path<String>,
    method: Method,
    headers: HeaderMap,
    query: std::result::Result<Query<Size>, QueryRejection>,
    body: Body,
) -> Response {
    // Only the root /mcp issues links (create_transfer is in no endpoint scope), so they answer
    // only while it is on.
    if !app.config.root_mcp_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if headers.contains_key(header::ORIGIN) && !auth::origin_matches(&headers) {
        return Error(
            StatusCode::FORBIDDEN,
            "Transfer request origin rejected".into(),
        )
        .into_response();
    }
    let Some((operation, issuer)) = app.transfers.take_issued(&token, Instant::now()) else {
        return Error(
            StatusCode::NOT_FOUND,
            "This transfer link is unknown, used or expired. Ask create_transfer for a new one."
                .into(),
        )
        .into_response();
    };
    // A link that is used is an agent's action, recorded as its tool calls are.
    let call = activity::Call::transfer(
        &app,
        issuer,
        operation.name(),
        operation.arguments(),
        operation.kind(),
    );
    // The container is in use until a download has been sent, which outlasts the record.
    let using = operation
        .arguments()
        .get("container_id")
        .and_then(Value::as_str)
        .and_then(|id| app.container(id).ok())
        .map(|container| app.lifecycle.open(&container.id));
    let response = match run(app.clone(), operation, method, &headers, query, body).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    let response = call.finish_transfer(&app, response).await;
    match using {
        Some(using) => crate::lifecycle::lasting(response, using),
        None => response,
    }
}

async fn run(
    app: Arc<App>,
    operation: Operation,
    method: Method,
    headers: &HeaderMap,
    query: std::result::Result<Query<Size>, QueryRejection>,
    body: Body,
) -> Result<Response> {
    let sends = operation.sends();
    if sends != matches!(method, Method::POST | Method::PUT) || (!sends && method != Method::GET) {
        let allow = if sends { "POST, PUT" } else { "GET" };
        return Ok((
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, allow)],
            axum::Json(json!({"error": format!("This link takes {allow}")})),
        )
            .into_response());
    }
    let Query(Size { size }) = query.map_err(|_| {
        Error(
            StatusCode::BAD_REQUEST,
            "A transfer link takes no query parameters other than size".into(),
        )
    })?;
    if !sends && size.is_some() {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            "A download takes no size".into(),
        ));
    }
    let declared = size.or_else(|| {
        headers
            .get(header::CONTENT_LENGTH)?
            .to_str()
            .ok()?
            .parse()
            .ok()
    });
    // A file upload needs its size; an archive import also streams in without one.
    let length = || {
        declared.ok_or_else(|| {
            Error(
                StatusCode::LENGTH_REQUIRED,
                "Send Content-Length, or the body's size as ?size=".into(),
            )
        })
    };
    // The link stands for the checks a browser-facing route makes, so the body is binary
    // whatever the client labels it, and no session can end the transfer.
    let mut binary = HeaderMap::new();
    binary.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    let link = Session::Link;
    let state = State(app);
    Ok(match operation {
        Operation::ContainerUpload {
            container_id,
            destination,
            name,
            replace,
        } => {
            let request = uploads::Upload {
                destination,
                name,
                size: length()?,
                replace,
            };
            uploads::upload(
                state,
                Path(container_id),
                Query(request),
                binary,
                link,
                body,
            )
            .await
            .into_response()
        }
        Operation::ContainerDownload { container_id, path } => files::download(
            state,
            Path(container_id),
            Query(files::DownloadQuery { path }),
            link,
        )
        .await
        .into_response(),
        Operation::VolumeUpload {
            volume,
            destination,
            name,
            replace,
        } => {
            let request = uploads::Upload {
                destination,
                name,
                size: length()?,
                replace,
            };
            volumes::upload(state, Path(volume), Query(request), binary, link, body)
                .await
                .into_response()
        }
        Operation::VolumeDownload { volume, path } => volumes::download(
            state,
            Path(volume),
            Query(volumes::DownloadQuery { path }),
            link,
        )
        .await
        .into_response(),
        Operation::VolumeImport { name } => {
            let request = volumes::Import {
                name,
                size: declared,
            };
            volumes::import(state, Query(request), binary, link, body)
                .await
                .into_response()
        }
        Operation::VolumeExtract { volume, path } => {
            let request = volumes::Extract {
                path,
                size: declared,
            };
            volumes::extract(state, Path(volume), Query(request), binary, link, body)
                .await
                .into_response()
        }
        Operation::VolumeExport { volume } => volumes::export(state, Path(volume), link)
            .await
            .into_response(),
        Operation::ImageImport { name, description } => {
            let request = archives::Import {
                size: declared,
                name,
                description,
            };
            archives::import(state, Query(request), binary, link, body)
                .await
                .into_response()
        }
        Operation::ImageExport { image_id } => archives::export(state, Path(image_id), link)
            .await
            .into_response(),
    })
}

#[cfg(test)]
#[path = "transfers_tests.rs"]
mod tests;
