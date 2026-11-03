//! Docker's change list of a Docker container: the Engine API's `GET /containers/{id}/changes`
//! where the manager reaches the socket, whose JSON names every path exactly, and `docker
//! container diff` otherwise, whose lines a name with a line break continues.
use crate::App;
use anyhow::{Context, Result, bail};
use http_body_util::BodyExt;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// The most bytes of a change list Docker may send; one of millions of paths is refused.
pub(crate) const LIST_LIMIT: usize = 32 * 1024 * 1024;
/// How long Docker may take to walk the container's writable layer.
const PATIENCE: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Added,
    Changed,
    Deleted,
}
impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Added, Kind::Changed, Kind::Deleted];
    pub fn id(self) -> &'static str {
        match self {
            Kind::Added => "added",
            Kind::Changed => "changed",
            Kind::Deleted => "deleted",
        }
    }
}

/// One path of the list and what happened to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    pub kind: Kind,
}

impl App {
    /// The change list of the Docker container `docker_id`, in Docker's order.
    pub(crate) async fn change_list(&self, docker_id: &str) -> Result<Vec<Change>> {
        if let Some(socket) = &self.config.docker_socket {
            match crate::pulls::connect(socket).await {
                Ok(stream) => return engine(stream, docker_id).await,
                // A CLI configured through its own context reaches a daemon the socket does not.
                Err(error) => tracing::debug!(
                    "Docker Engine socket {}: {error}; asking the CLI for the change list",
                    socket.display()
                ),
            }
        }
        // A list beyond the limit ends the command as soon as it is read that far.
        let output = crate::process::capture(
            self.command(&["container".into(), "diff".into(), docker_id.into()]),
            PATIENCE,
            LIST_LIMIT,
            crate::process::Overflow::Reject,
        )
        .await
        .map_err(|error| {
            if error.is::<crate::process::Exceeded>() {
                too_long()
            } else {
                error
            }
        })?;
        if !output.status.success() {
            return Err(crate::error::docker_failure(
                &output.stderr,
                output.status.code(),
            ));
        }
        Ok(cli_entries(&String::from_utf8_lossy(&output.stdout)))
    }
}

/// Ask the Engine API on `io` for the change list.
async fn engine<S>(io: S, docker_id: &str) -> Result<Vec<Change>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(io))
            .await
            .context("Docker API handshake failed")?;
    let connection = tokio::spawn(connection);
    let exchange = async {
        let request = hyper::Request::get(format!("/containers/{docker_id}/changes"))
            .header(hyper::header::HOST, "docker")
            .body(http_body_util::Empty::<hyper::body::Bytes>::new())?;
        let response = sender
            .send_request(request)
            .await
            .context("Docker API request failed")?;
        let status = response.status();
        let body = http_body_util::Limited::new(response.into_body(), LIST_LIMIT)
            .collect()
            .await
            .map_err(|error| {
                if error.is::<http_body_util::LengthLimitError>() {
                    too_long()
                } else {
                    anyhow::anyhow!(error).context("Docker's change list was cut short")
                }
            })?
            .to_bytes();
        if !status.is_success() {
            let message = serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| value["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| String::from_utf8_lossy(&body).trim().to_owned());
            return Err(crate::error::engine_failure(
                status,
                format!("Docker: {message} ({status})"),
            ));
        }
        engine_entries(&body)
    };
    let result = tokio::time::timeout(PATIENCE, exchange)
        .await
        .unwrap_or_else(|_| Err(crate::process::timed_out()));
    connection.abort();
    result
}

/// A list beyond [`LIST_LIMIT`].
fn too_long() -> anyhow::Error {
    crate::error::Fault::error(
        axum::http::StatusCode::BAD_GATEWAY,
        format!(
            "Docker's change list of this container is larger than {} MiB, more than the manager reads",
            LIST_LIMIT / 1024 / 1024
        ),
    )
}

/// The Engine API's answer: `null` or a list of `{"Path", "Kind"}`, where kind 0 is a change,
/// 1 an addition and 2 a deletion.
pub(crate) fn engine_entries(body: &[u8]) -> Result<Vec<Change>> {
    #[derive(Deserialize)]
    struct Entry {
        #[serde(rename = "Path")]
        path: String,
        #[serde(rename = "Kind")]
        kind: u8,
    }
    let entries: Option<Vec<Entry>> =
        serde_json::from_slice(body).context("Docker sent an unreadable change list")?;
    entries
        .unwrap_or_default()
        .into_iter()
        .map(|entry| {
            let kind = match entry.kind {
                0 => Kind::Changed,
                1 => Kind::Added,
                2 => Kind::Deleted,
                other => bail!("Docker named a change of an unknown kind {other}"),
            };
            Ok(Change {
                path: entry.path,
                kind,
            })
        })
        .collect()
}

/// What `docker container diff` printed: a line `A /path`, `C /path` or `D /path` each, where a
/// line that does not start so is the rest of a name with a line break in it. A name whose line
/// break is followed by such a start reads as two entries; the Engine API has no such case.
pub(crate) fn cli_entries(text: &str) -> Vec<Change> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    let mut entries: Vec<Change> = Vec::new();
    if text.is_empty() {
        return entries;
    }
    for line in text.split('\n') {
        let kind = match line.get(..3) {
            Some("A /") => Some(Kind::Added),
            Some("C /") => Some(Kind::Changed),
            Some("D /") => Some(Kind::Deleted),
            _ => None,
        };
        match (kind, entries.last_mut()) {
            (Some(kind), _) => entries.push(Change {
                path: line[2..].to_owned(),
                kind,
            }),
            (None, Some(last)) => {
                last.path.push('\n');
                last.path.push_str(line);
            }
            (None, None) => {}
        }
    }
    entries
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
