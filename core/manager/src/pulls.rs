//! Downloads of the catalog images from their registry.
//!
//! An installation does not pull every catalog image up front: one is downloaded the first time
//! a container or an image build starts from it, or when the dashboard asks for it. The CLI hides byte
//! counts from non-terminal output, so pulls go through the Engine API socket and the job carries
//! one aggregated `progress` value for the dashboard and for agents. A release manager downloads
//! the catalog images published with it and tags them as the catalog reference
//! (`CatalogImage::source`). The image the reference named before goes once nothing needs it
//! (`App::remove_replaced`).
use crate::{App, catalog::CatalogImage};
use anyhow::{Context, Result, bail};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    time::{Duration, Instant},
};
use tokio::io::{AsyncRead, AsyncWrite};

/// A pull that sends nothing for this long has stalled; Docker reports large layers as they move.
const IDLE_LIMIT: Duration = Duration::from_secs(600);
/// How often running progress is written to the job record.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);
/// Pull messages are small; a longer line is not one.
const LINE_LIMIT: usize = 1 << 20;
/// A registry connection that stalls for a few seconds fails the daemon's pull, so a failed
/// download is tried again before the job gives up.
const ATTEMPTS: u64 = 3;

/// The Engine turned the pull down, such as an unknown image or denied access; trying again
/// cannot change that.
#[derive(Debug)]
pub(crate) struct Refused(String);
impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Refused {}

#[derive(Default)]
struct Layer {
    current: u64,
    /// The size Docker gave in the layer's first byte count, which starts at the layer's beginning.
    total: Option<u64>,
    downloaded: bool,
    complete: bool,
    existing: bool,
}

/// Aggregates Docker's per-layer pull messages into one count for the whole image.
#[derive(Default)]
pub(crate) struct PullProgress {
    /// Compressed layer sizes from the registry manifest, keyed by Docker's short layer ID.
    sizes: HashMap<String, u64>,
    layers: BTreeMap<String, Layer>,
}

impl PullProgress {
    /// Record one message. A message that is not about a layer comes back as a job log line.
    pub(crate) fn apply(&mut self, message: &Value) -> Result<Option<String>> {
        if let Some(error) = message["errorDetail"]["message"]
            .as_str()
            .or(message["error"].as_str())
        {
            bail!("Docker: {error}");
        }
        let status = message["status"].as_str().unwrap_or_default();
        let id = message["id"].as_str().unwrap_or_default();
        // "Pulling from" carries the tag as its ID, not a layer.
        if id.is_empty() || status.starts_with("Pulling from") {
            let line = if id.is_empty() {
                status.to_owned()
            } else {
                format!("{id}: {status}")
            };
            return Ok((!line.is_empty()).then_some(line));
        }
        // Every image layer is announced first. The containerd image store also reports blobs
        // it never unpacks, such as an attestation, and those never are.
        let layer = if matches!(status, "Pulling fs layer" | "Waiting" | "Already exists") {
            self.layers.entry(id.to_owned()).or_default()
        } else if let Some(layer) = self.layers.get_mut(id) {
            layer
        } else {
            return Ok(None);
        };
        match status {
            "Downloading" => {
                let detail = &message["progressDetail"];
                let left = detail["total"].as_u64().filter(|total| *total > 0);
                if layer.total.is_none() {
                    layer.total = left;
                }
                match (detail["current"].as_u64(), left) {
                    // Docker prints a full bar for an attempt that broke off as for one that
                    // finished, and the checksum that follows a finished one counts the layer.
                    (Some(current), Some(left)) if current >= left => {}
                    (Some(current), _) => {
                        // A broken download resumes where it stopped, counted from zero against
                        // what is left. One that starts over keeps what it had until it passes it.
                        let before = layer
                            .total
                            .zip(left)
                            .map_or(0, |(size, left)| size.saturating_sub(left));
                        layer.current = layer.current.max(before + current);
                    }
                    (None, _) => {}
                }
            }
            "Verifying Checksum" | "Download complete" | "Extracting" => layer.downloaded = true,
            "Pull complete" => {
                layer.downloaded = true;
                layer.complete = true;
            }
            // A layer an earlier attempt of this download fetched is not one the host had.
            "Already exists" if layer.downloaded || layer.current > 0 => {
                layer.downloaded = true;
                layer.complete = true;
            }
            "Already exists" => {
                layer.existing = true;
                layer.complete = true;
            }
            _ => {}
        }
        Ok(None)
    }

    /// Bytes to download and downloaded so far. Layers already on the host are not counted, and
    /// `total` is null while any remaining layer's size is unknown.
    pub(crate) fn summary(&self) -> Value {
        let (mut current, mut total) = (0, Some(0));
        for (id, layer) in &self.layers {
            if layer.existing {
                continue;
            }
            match self.sizes.get(id).copied().or(layer.total) {
                Some(size) => {
                    total = total.map(|total: u64| total + size);
                    current += if layer.downloaded {
                        size
                    } else {
                        layer.current.min(size)
                    };
                }
                None => {
                    total = None;
                    current += layer.current;
                }
            }
        }
        let done = self.layers.values().filter(|layer| layer.complete).count();
        json!({"current": current, "total": total, "layers": self.layers.len(), "layers_done": done})
    }
}

/// The architecture of the images a Docker host runs, as an image index names it: `amd64` for
/// an x86_64 host and `arm64` for an aarch64 one, such as a Raspberry Pi or a Mac's Docker
/// Desktop. `docker info` gives the kernel's name for it; without an answer, the manager's own
/// architecture stands in, since it runs on that host.
pub(crate) fn image_architecture(docker_info: Option<&Value>) -> String {
    let machine = docker_info
        .and_then(|info| info["Architecture"].as_str())
        .filter(|machine| !machine.is_empty())
        .unwrap_or(std::env::consts::ARCH);
    match machine {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "i386" | "i686" | "x86" => "386",
        other if other.starts_with("armv") => "arm",
        other => other,
    }
    .to_owned()
}

/// Layer sizes from `docker manifest inspect --verbose`. An index lists one entry per platform
/// and attestation; the sizes are those of the Linux image for `architecture`, the one the Docker
/// host runs ([`image_architecture`]).
pub(crate) fn manifest_sizes(output: &Value, architecture: &str) -> HashMap<String, u64> {
    let manifest = match output {
        Value::Array(entries) => {
            let mut matching = entries.iter().filter(|entry| {
                let platform = &entry["Descriptor"]["platform"];
                platform["os"] == "linux" && platform["architecture"] == architecture
            });
            match (matching.next(), matching.next()) {
                (Some(entry), None) => entry,
                _ => return HashMap::new(),
            }
        }
        entry => entry,
    };
    let layers = manifest["OCIManifest"]["layers"]
        .as_array()
        .or(manifest["SchemaV2Manifest"]["layers"].as_array());
    layers
        .into_iter()
        .flatten()
        .filter_map(|layer| {
            let digest = layer["digest"].as_str()?;
            let hex = digest.split_once(':').map_or(digest, |(_, hex)| hex);
            Some((hex.get(..12)?.to_owned(), layer["size"].as_u64()?))
        })
        .collect()
}

/// Percent-encode a query value; Go reads `+` in a query as a space.
pub(crate) fn query_value(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'/'
            | b':'
            | b'@' => char::from(byte).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// Stream `POST /images/create` over an Engine API connection and pass on each message.
pub(crate) async fn engine_pull<S>(
    io: S,
    reference: &str,
    mut on_message: impl FnMut(Value) -> Result<()>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(io))
            .await
            .context("Docker API handshake failed")?;
    // Errors on the connection reach the response body, so the task's own result is not needed.
    let connection = tokio::spawn(connection);
    let exchange = async {
        let request = hyper::Request::post(format!(
            "/images/create?fromImage={}",
            query_value(reference)
        ))
        .header(hyper::header::HOST, "docker")
        .body(http_body_util::Empty::<hyper::body::Bytes>::new())?;
        let mut response = sender
            .send_request(request)
            .await
            .context("Docker API request failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = http_body_util::Limited::new(response.into_body(), 65_536)
                .collect()
                .await
                .map(|body| body.to_bytes())
                .unwrap_or_default();
            let message = serde_json::from_slice::<Value>(&body)
                .ok()
                .and_then(|value| value["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| String::from_utf8_lossy(&body).trim().to_owned());
            let error = format!("Docker: {message} ({status})");
            if status.is_client_error() {
                return Err(Refused(error).into());
            }
            return Err(crate::error::engine_failure(status, error));
        }
        let mut pending = Vec::new();
        while let Some(frame) = tokio::time::timeout(IDLE_LIMIT, response.body_mut().frame())
            .await
            .context("The download stalled: Docker sent nothing for ten minutes")?
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
                message_line(&line, &mut on_message)?;
            }
            if pending.len() > LINE_LIMIT {
                bail!("Docker sent an oversized pull message");
            }
        }
        message_line(&pending, &mut on_message)
    };
    let result = exchange.await;
    // Closing the connection also cancels a pull that stopped early on the daemon's side.
    connection.abort();
    result
}

fn message_line(line: &[u8], on_message: &mut impl FnMut(Value) -> Result<()>) -> Result<()> {
    let line = line.trim_ascii();
    if line.is_empty() {
        return Ok(());
    }
    on_message(serde_json::from_slice(line).context("Docker sent an unreadable pull message")?)
}

#[cfg(unix)]
pub(crate) async fn connect(path: &Path) -> std::io::Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(path).await
}
#[cfg(not(unix))]
pub(crate) async fn connect(_: &Path) -> std::io::Result<tokio::io::DuplexStream> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "the Engine socket needs a Unix host",
    ))
}

impl App {
    /// Download a catalog image the first time a container or a build starts from it. Other
    /// references are left to the caller's own availability check.
    pub async fn ensure_catalog_image(&self, reference: &str, job_id: &str) -> Result<()> {
        let Some(image) = crate::catalog::catalog()
            .into_iter()
            .find(|image| image.reference == reference)
        else {
            return Ok(());
        };
        // A build or another download of this image replaces the same tag.
        let lock = self.resource_lock(&format!("image:{}", image.id));
        let _guard = match lock.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                self.append_log(
                    job_id,
                    &format!(
                        "Waiting for the current download or build of {}.",
                        image.name
                    ),
                )?;
                lock.lock().await
            }
        };
        if self.image_present(reference).await? {
            return Ok(());
        }
        self.pull_catalog_image(&image, job_id).await.map(drop)
    }

    /// Whether Docker has `reference`; a Docker that cannot answer is an error, not an absence.
    pub(crate) async fn image_present(&self, reference: &str) -> Result<bool> {
        match self.inspect("image", reference).await {
            Ok(_) => Ok(true),
            Err(error) => {
                // Only a listing that succeeds can tell an absent image from a Docker failure.
                let names = self
                    .docker(&[
                        "image",
                        "ls",
                        "--format",
                        "{{.Repository}}:{{.Tag}}",
                        reference,
                    ])
                    .await?;
                if String::from_utf8_lossy(&names)
                    .lines()
                    .any(|name| name == reference)
                {
                    return Err(error);
                }
                Ok(false)
            }
        }
    }

    /// Pull a catalog image, or a newer release of it, and report progress on the job.
    pub async fn pull_catalog_image(&self, image: &CatalogImage, job_id: &str) -> Result<Value> {
        self.pull_catalog_image_from(image, &image.source(), job_id)
            .await
    }

    async fn pull_catalog_image_from(
        &self,
        image: &CatalogImage,
        source: &str,
        job_id: &str,
    ) -> Result<Value> {
        self.append_log(
            job_id,
            &format!("Downloading {} from {source}.", image.name),
        )?;
        // The image the reference names until the download moves it, which goes afterwards when
        // nothing needs it.
        let replaced = self
            .inspect("image", &image.reference)
            .await
            .ok()
            .and_then(|info| info["Id"].as_str().map(str::to_owned));
        // A release tag that was here before the download is someone else's and stays.
        let source_was_here =
            source != image.reference && self.image_present(source).await.unwrap_or(true);
        // An empty count tells the dashboard the download is being prepared.
        self.set_progress(job_id, Some(PullProgress::default().summary()))?;
        // One count for every attempt, so the job's progress never goes back: a layer an earlier
        // attempt finished comes back as already present and stays counted.
        let mut progress = PullProgress {
            sizes: self.layer_sizes(source).await,
            ..Default::default()
        };
        let mut attempt = 1;
        let pulled = loop {
            match self.pull_image(job_id, source, &mut progress).await {
                Err(error) if attempt < ATTEMPTS && error.downcast_ref::<Refused>().is_none() => {
                    let wait = 5 * attempt;
                    self.append_log(
                        job_id,
                        &format!("{error:#}\nTrying the download again in {wait} seconds."),
                    )?;
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                    attempt += 1;
                }
                result => break result,
            }
        };
        self.set_progress(job_id, None)?;
        pulled?;
        if source != image.reference {
            self.docker(&["tag", source, &image.reference]).await?;
            self.append_log(job_id, &format!("Tagged it as {}.", image.reference))?;
            // The release tag goes, so only the reference names the new image.
            if !source_was_here && let Err(error) = self.docker(&["image", "rm", source]).await {
                tracing::warn!("Could not remove the tag {source}: {error:#}");
            }
        }
        let info = self
            .inspect("image", &image.reference)
            .await
            .context("The download finished, but Docker does not list the image")?;
        if let Some(replaced) = replaced.filter(|replaced| info["Id"] != replaced.as_str()) {
            let said = self.remove_replaced(&replaced, &image.reference).await;
            self.append_log(job_id, &said)?;
        }
        Ok(
            json!({"id": image.id, "kind": "catalog", "reference": image.reference, "image_id": info["Id"]}),
        )
    }

    /// Best effort: without the manifest, the total is known only once every layer has started.
    async fn layer_sizes(&self, reference: &str) -> HashMap<String, u64> {
        let args = ["manifest", "inspect", "--verbose", reference].map(str::to_owned);
        match crate::process::capture(
            self.command(&args),
            Duration::from_secs(30),
            4 * 1024 * 1024,
            crate::process::Overflow::Reject,
        )
        .await
        {
            Ok(output) if output.status.success() => serde_json::from_slice(&output.stdout)
                .map(|value| manifest_sizes(&value, &self.config.architecture))
                .unwrap_or_default(),
            _ => HashMap::new(),
        }
    }

    async fn pull_image(
        &self,
        job_id: &str,
        reference: &str,
        progress: &mut PullProgress,
    ) -> Result<()> {
        if let Some(socket) = &self.config.docker_socket {
            match connect(socket).await {
                Ok(stream) => {
                    let (mut written, mut done) = (Instant::now(), 0);
                    return engine_pull(stream, reference, |message| {
                        if let Some(line) = progress.apply(&message)? {
                            self.append_log(job_id, &line)?;
                        }
                        let summary = progress.summary();
                        let layers_done = summary["layers_done"].as_u64().unwrap_or_default();
                        // A finished layer is written at once, so a quiet extraction never shows stale state.
                        if written.elapsed() >= PROGRESS_INTERVAL || layers_done != done {
                            self.set_progress(job_id, Some(summary))?;
                            (written, done) = (Instant::now(), layers_done);
                        }
                        Ok(())
                    })
                    .await;
                }
                Err(error) => tracing::debug!(
                    "Pulling {reference} through the CLI; {} is unavailable: {error}",
                    socket.display()
                ),
            }
        }
        // The CLI still pulls without the socket, but its log names layer states without byte counts.
        self.docker_logged(job_id, &["pull".into(), reference.into()])
            .await
    }
}

#[cfg(test)]
#[path = "pulls_tests.rs"]
mod tests;
