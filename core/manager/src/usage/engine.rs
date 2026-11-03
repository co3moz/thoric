//! The Engine API requests the collector makes: which managed containers run, and the stats
//! stream of each. Both go over the socket pulls use, one connection per request; Docker's own
//! events (`crate::events::docker`) are read through [`stream`] too.
use crate::model::{Container, LABEL};
use anyhow::{Context, Result, bail};
use http_body_util::BodyExt;
use hyper::body::Bytes;
use serde_json::Value;
use std::{collections::BTreeMap, time::Duration};
use tokio::io::{AsyncRead, AsyncWrite};

/// A list of every container on a busy host stays far below this.
const LIST_LIMIT: usize = 8 << 20;
/// One stats message or event is a few kilobytes; a longer line is not one.
const LINE_LIMIT: usize = 1 << 20;

/// Every container that carries the managed label, running or not, so one listing tells both
/// which to follow and the state of each.
pub(crate) fn list_path() -> String {
    let filters = format!(r#"{{"label":["{LABEL}=true"]}}"#);
    format!(
        "/containers/json?all=1&filters={}",
        crate::pulls::query_value(&filters)
    )
}

/// Whether `docker_id` is an ID Docker lists, which the paths below may hold.
fn listed_id(docker_id: &str) -> bool {
    docker_id.len() == 64 && docker_id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The stats stream of a container, by the ID Docker listed it under.
pub(crate) fn stats_path(docker_id: &str) -> Option<String> {
    listed_id(docker_id).then(|| format!("/containers/{docker_id}/stats?stream=true"))
}

/// A container's inspect, by the ID Docker listed it under.
pub(crate) fn inspect_path(docker_id: &str) -> Option<String> {
    listed_id(docker_id).then(|| format!("/containers/{docker_id}/json"))
}

/// A running container this manager owns.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Running {
    pub docker_id: String,
    pub gpu: bool,
}

/// The listed containers that belong to a record, with their Docker ID. A container counts only
/// when both its name and its owner label match the record, as [`crate::App::owned`] requires.
fn owned<'a>(
    records: &'a [Container],
    listed: &'a Value,
) -> impl Iterator<Item = (&'a Container, &'a str, &'a Value)> {
    listed
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(move |container| {
            let docker_id = container["Id"].as_str().filter(|id| listed_id(id))?;
            let owner = container["Labels"]["thoric.environment"].as_str();
            let record = records
                .iter()
                .find(|record| Some(record.id.as_str()) == owner)?;
            let named = container["Names"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .any(|name| name.trim_start_matches('/') == record.container);
            (named && container["Labels"][LABEL] == "true")
                .then_some((record, docker_id, container))
        })
}

/// The records whose container Docker lists as running, by record ID.
pub(crate) fn running(records: &[Container], listed: &Value) -> BTreeMap<String, Running> {
    owned(records, listed)
        .filter(|(.., container)| container["State"] == "running")
        .map(|(record, docker_id, _)| {
            let running = Running {
                docker_id: docker_id.to_owned(),
                gpu: record.spec.gpu,
            };
            (record.id.clone(), running)
        })
        .collect()
}

/// The state Docker lists each record's container in, such as `running` or `exited`, by record
/// ID. A record whose container Docker does not list is left out.
pub(crate) fn states(records: &[Container], listed: &Value) -> BTreeMap<String, String> {
    owned(records, listed)
        .filter_map(|(record, _, container)| {
            let state = container["State"].as_str()?;
            Some((record.id.clone(), state.to_owned()))
        })
        .collect()
}

/// Stops a task when dropped, such as the connection under a response nobody reads any more.
pub(crate) struct Abort(pub tokio::task::JoinHandle<()>);
impl Drop for Abort {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn request<S>(io: S, path: &str) -> Result<(hyper::Response<hyper::body::Incoming>, Abort)>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(io))
            .await
            .context("Docker API handshake failed")?;
    // Errors on the connection reach the body, so the task's own result is not needed.
    let connection = Abort(tokio::spawn(async move {
        let _ = connection.await;
    }));
    let request = hyper::Request::get(path)
        .header(hyper::header::HOST, "docker")
        .body(http_body_util::Empty::<Bytes>::new())?;
    let response = sender
        .send_request(request)
        .await
        .context("Docker API request failed")?;
    if !response.status().is_success() {
        let status = response.status();
        let body = http_body_util::Limited::new(response.into_body(), 65_536)
            .collect()
            .await
            .map(|body| body.to_bytes())
            .unwrap_or_default();
        let message = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|value| value["message"].as_str().map(str::to_owned))
            .unwrap_or_else(|| String::from_utf8_lossy(&body).trim().to_owned());
        return Err(crate::error::engine_failure(
            status,
            format!("Docker: {message} ({status})"),
        ));
    }
    Ok((response, connection))
}

/// One JSON response.
pub(crate) async fn get<S>(io: S, path: &str) -> Result<Value>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (response, _connection) = request(io, path).await?;
    let body = http_body_util::Limited::new(response.into_body(), LIST_LIMIT)
        .collect()
        .await
        .map_err(|error| anyhow::anyhow!("The Docker API response was interrupted: {error}"))?
        .to_bytes();
    serde_json::from_slice(&body).context("Docker sent an unreadable response")
}

/// Pass on each line of a streamed response until it ends, or until it sends nothing for `idle`.
/// `opened` is called once Docker accepted the request, before the first line.
pub(crate) async fn stream<S>(
    io: S,
    path: &str,
    idle: Duration,
    opened: impl FnOnce(),
    mut on_line: impl FnMut(&[u8]),
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut response, _connection) = request(io, path).await?;
    opened();
    let mut pending = Vec::new();
    while let Some(frame) = tokio::time::timeout(idle, response.body_mut().frame())
        .await
        .context("Docker stopped sending messages")?
    {
        let Ok(data) = frame
            .context("The Docker API response was interrupted")?
            .into_data()
        else {
            continue;
        };
        pending.extend_from_slice(&data);
        while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            let line = line.trim_ascii();
            if !line.is_empty() {
                on_line(line);
            }
        }
        if pending.len() > LINE_LIMIT {
            bail!("Docker sent an oversized message");
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
