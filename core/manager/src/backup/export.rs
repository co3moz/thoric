//! The backup job: what the installation has, then each part streamed into the archive the
//! browser downloads as it is made. A part goes out in chunks of [`CHUNK`] bytes, so the manager
//! holds one chunk at a time and writes nothing to its disk.
use super::{
    RESOURCE, file_name,
    format::{CHUNK, END, MANIFEST, RECORDS, Writer, chunk_name, part_end_name},
    installation,
    manifest::{
        Ending, Included, Installation, Manifest, PartEnd, PartKind, VolumeFacts, plan,
        template_volumes,
    },
};
use crate::{
    App,
    archives::{stall, transfer_permit},
    auth,
    compose::{PINNED, Pinned},
    desktop_mcp::{ENDPOINTS, Endpoint},
    error::Result,
    jobs,
    model::{CONTAINERS, Container, ImageRecord, LABEL, now},
    outbound::{CREDENTIALS, Credentials},
    process::{self, Overflow},
    scale::{SETS, ScaleSet, TEMPLATES, Template},
};
use anyhow::{Context, anyhow, bail};
use axum::{
    body::Bytes,
    extract::{Query, State},
    http::header,
    response::{IntoResponse, Response},
};
use blake2::{Blake2s256, Digest};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    pin::Pin,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::mpsc,
};
use tracing::Instrument;

/// How often the bytes sent so far are written to the job.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);
const READ_SIZE: usize = 256 * 1024;

/// `GET /api/backup`: stream a backup of the installation with the parts `included` names. The
/// transfer is a job, so its progress and any error stay visible after the download has begun.
pub async fn export(
    State(app): State<Arc<App>>,
    Query(included): Query<Included>,
    token: auth::Session,
) -> Result<Response> {
    let guard = installation(&app)?;
    let permit = transfer_permit(&app)?;
    let started = now();
    let (body, chunks) = mpsc::channel::<std::io::Result<Bytes>>(4);
    let span = tracing::Span::current();
    let (job_id, outcome) = jobs::start(
        app,
        "backup",
        RESOURCE.into(),
        false,
        move |app, job_id| async move {
            let _guard = guard;
            let _permit = permit;
            app.back_up(&job_id, included, started, &body, token)
                .instrument(span)
                .await
        },
    )?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/gzip".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", file_name(started)),
            ),
            (header::HeaderName::from_static("x-job-id"), job_id),
        ],
        stall::download(chunks, outcome),
    )
        .into_response())
}

/// The archive on its way to the browser.
struct Out<'a> {
    app: &'a App,
    job_id: &'a str,
    body: &'a mpsc::Sender<std::io::Result<Bytes>>,
    revoked: Pin<Box<dyn Future<Output = ()> + Send + 'a>>,
    writer: Option<Writer>,
    sent: u64,
    reported: Instant,
}

impl Out<'_> {
    /// Add one entry, compressed away from the async workers, and fold its data into `hasher`.
    async fn entry(
        &mut self,
        name: String,
        data: Vec<u8>,
        flush: bool,
        mut hasher: Option<Blake2s256>,
    ) -> anyhow::Result<Option<Blake2s256>> {
        let mut writer = self.writer.take().context("The backup has ended")?;
        let (writer, hasher, output) = tokio::task::spawn_blocking(move || {
            if let Some(hasher) = &mut hasher {
                hasher.update(&data);
            }
            let output = writer.entry(&name, &data, flush);
            (writer, hasher, output)
        })
        .await?;
        self.writer = Some(writer);
        self.send(output?).await?;
        Ok(hasher)
    }

    async fn send(&mut self, chunk: Vec<u8>) -> anyhow::Result<()> {
        if chunk.is_empty() {
            return Ok(());
        }
        self.sent += chunk.len() as u64;
        stall::send(self.body, Bytes::from(chunk), &mut self.revoked).await?;
        if self.reported.elapsed() >= PROGRESS_INTERVAL {
            self.app.set_progress(
                self.job_id,
                Some(json!({"transfer": "export", "current": self.sent})),
            )?;
            self.reported = Instant::now();
        }
        Ok(())
    }

    /// Send what `source` reads as the chunks of part `index`, and return their length and
    /// digest. The part's end entry follows only once its source has ended well.
    async fn part_data(
        &mut self,
        index: usize,
        source: &mut (impl AsyncRead + Unpin),
    ) -> anyhow::Result<(u64, Blake2s256)> {
        let mut hasher = Some(Blake2s256::new());
        let mut buffer = Vec::with_capacity(CHUNK);
        let mut read_buffer = vec![0; READ_SIZE];
        let (mut chunk, mut total) = (0u64, 0u64);
        loop {
            let read = tokio::select! {
                read = source.read(&mut read_buffer) => read.context("Could not read the part")?,
                _ = &mut self.revoked => bail!("Session expired. Sign in again."),
                _ = self.body.closed() => bail!(stall::CANCELLED),
            };
            let mut input = &read_buffer[..read];
            while !input.is_empty() {
                let take = (CHUNK - buffer.len()).min(input.len());
                buffer.extend_from_slice(&input[..take]);
                input = &input[take..];
                if buffer.len() == CHUNK {
                    total += CHUNK as u64;
                    let data = std::mem::replace(&mut buffer, Vec::with_capacity(CHUNK));
                    hasher = self
                        .entry(chunk_name(index, chunk), data, false, hasher)
                        .await?;
                    chunk += 1;
                }
            }
            if read == 0 {
                break;
            }
        }
        if !buffer.is_empty() {
            total += buffer.len() as u64;
            hasher = self
                .entry(chunk_name(index, chunk), buffer, false, hasher)
                .await?;
        }
        Ok((total, hasher.context("Missing digest")?))
    }

    async fn part_end(
        &mut self,
        index: usize,
        bytes: u64,
        hasher: Blake2s256,
    ) -> anyhow::Result<()> {
        let end = PartEnd {
            bytes,
            blake2s: hex(&hasher.finalize()),
        };
        self.entry(part_end_name(index), serde_json::to_vec(&end)?, false, None)
            .await?;
        Ok(())
    }

    /// Part `index`: `docker save` of the image `image_id`.
    async fn save_image(&mut self, index: usize, image_id: &str) -> anyhow::Result<u64> {
        let mut child = self
            .app
            .command(&["save".into(), image_id.into()])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Could not start Docker CLI")?;
        let mut stdout = child.stdout.take().context("Missing docker save output")?;
        let errors = tokio::spawn(process::read_limited(
            child.stderr.take().context("Missing docker save errors")?,
            8192,
            Overflow::Tail,
        ));
        let (bytes, hasher) = match self.part_data(index, &mut stdout).await {
            Ok(streamed) => streamed,
            Err(error) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(error);
            }
        };
        let status = child.wait().await?;
        let errors = errors.await??;
        if !status.success() {
            bail!("Docker: {}", String::from_utf8_lossy(&errors).trim());
        }
        self.part_end(index, bytes, hasher).await?;
        Ok(bytes)
    }

    /// Part `index`: the files of `container`, a commit of its container layer, or the commit a
    /// settings change left it when its container is missing.
    async fn save_files(&mut self, index: usize, container: &Container) -> anyhow::Result<u64> {
        let app = self.app;
        let commit = match app
            .optional_inspect("container", &container.container)
            .await?
        {
            Some(_) => {
                let lock = app.resource_lock(&format!("container:{}", container.id));
                let _guard = match lock.clone().try_lock_owned() {
                    Ok(guard) => guard,
                    Err(_) => {
                        app.append_log(
                            self.job_id,
                            &format!(
                                "Waiting for the current operation on \"{}\".",
                                container.spec.name
                            ),
                        )?;
                        lock.lock_owned().await
                    }
                };
                let env = app.container(&container.id).map_err(|e| anyhow!(e.1))?;
                let info = app.owned(&env).await?;
                app.append_log(
                    self.job_id,
                    &format!(
                        "Committing the files of \"{}\". Docker pauses a running desktop while it commits; named volumes, /tmp and /run are not part of it.",
                        env.spec.name
                    ),
                )?;
                let args = [
                    "commit".into(),
                    "--change".into(),
                    format!("LABEL {LABEL}=true"),
                    "--message".into(),
                    format!("Thoric backup of {}", env.spec.name),
                    info["Id"].as_str().context("Missing container ID")?.into(),
                ];
                let output = process::capture(
                    app.command(&args),
                    crate::docker::LOGGED_LIMIT,
                    65_536,
                    Overflow::Reject,
                )
                .await?;
                if !output.status.success() {
                    bail!("Docker: {}", String::from_utf8_lossy(&output.stderr).trim());
                }
                Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
            }
            None => None,
        };
        let image = match (&commit, &container.working_image) {
            (Some(commit), _) => commit.clone(),
            (None, Some(working)) => working.image_id.clone(),
            (None, None) => bail!(
                "The container \"{}\" and its files are gone",
                container.spec.name
            ),
        };
        let saved = self.save_image(index, &image).await;
        // The commit was made for this backup alone; nothing else names it.
        if let Some(commit) = commit
            && let Err(error) = app.docker(&["image", "rm", &commit]).await
        {
            app.append_log(self.job_id, &format!("Kept the commit {commit}: {error:#}"))?;
        }
        saved
    }

    /// Part `index`: the volume `name`'s files as a plain tar.
    async fn save_volume(&mut self, index: usize, name: &str) -> anyhow::Result<u64> {
        let app = self.app;
        let mut reader = app.read_volume_files(name).await?;
        let (bytes, hasher) = match self.part_data(index, &mut reader.stdout).await {
            Ok(streamed) => streamed,
            Err(error) => {
                reader.abandon(app).await;
                return Err(error);
            }
        };
        if let Some(warning) = reader.finish(app).await? {
            app.append_log(self.job_id, &format!("tar: {warning}"))?;
        }
        self.part_end(index, bytes, hasher).await?;
        Ok(bytes)
    }
}

impl App {
    pub(super) async fn back_up(
        &self,
        job_id: &str,
        included: Included,
        started: u64,
        body: &mpsc::Sender<std::io::Result<Bytes>>,
        token: auth::Session,
    ) -> anyhow::Result<Value> {
        self.plan_stages(job_id, &crate::stages::BACKUP_STAGES)?;
        self.begin_stage(job_id, "plan")?;
        let installation = self.read_installation().await?;
        let (manifest, records) = plan(installation, included, env!("CARGO_PKG_VERSION"), started);
        self.append_log(job_id, &describe(&manifest))?;
        let mut out = Out {
            app: self,
            job_id,
            body,
            revoked: Box::pin(self.session_revoked(token)),
            writer: Some(Writer::new(started)),
            sent: 0,
            reported: Instant::now(),
        };
        self.set_progress(job_id, Some(json!({"transfer": "export", "current": 0})))?;
        out.entry(
            MANIFEST.into(),
            serde_json::to_vec_pretty(&manifest)?,
            true,
            None,
        )
        .await?;
        out.entry(RECORDS.into(), serde_json::to_vec(&records)?, true, None)
            .await?;
        for (index, part) in manifest.parts.iter().enumerate() {
            match part.kind {
                PartKind::Image => {
                    self.begin_stage(job_id, "images")?;
                    let record = records
                        .images
                        .iter()
                        .find(|record| record.id == part.target)
                        .context("Missing image record")?;
                    self.append_log(job_id, &format!("Saving the image \"{}\".", record.name))?;
                    let bytes = out.save_image(index, &record.image_id).await?;
                    self.append_log(
                        job_id,
                        &format!("Saved {bytes} bytes of \"{}\".", record.name),
                    )?;
                }
                PartKind::Files => {
                    self.begin_stage(job_id, "files")?;
                    let container = records
                        .containers
                        .iter()
                        .find(|container| container.id == part.target)
                        .context("Missing container record")?;
                    let bytes = out.save_files(index, container).await?;
                    self.append_log(
                        job_id,
                        &format!(
                            "Saved {bytes} bytes of the files of \"{}\".",
                            container.spec.name
                        ),
                    )?;
                }
                PartKind::Volume => {
                    self.begin_stage(job_id, "volumes")?;
                    self.append_log(job_id, &format!("Saving the volume {}.", part.target))?;
                    let bytes = out.save_volume(index, &part.target).await?;
                    self.append_log(job_id, &format!("Saved {bytes} bytes of {}.", part.target))?;
                }
            }
        }
        let ending = Ending {
            parts: manifest.parts.len(),
        };
        out.entry(END.into(), serde_json::to_vec(&ending)?, false, None)
            .await?;
        let writer = out.writer.take().context("The backup has ended")?;
        let last = tokio::task::spawn_blocking(move || writer.finish()).await??;
        out.send(last).await?;
        let sent = out.sent;
        self.append_log(job_id, &format!("Sent {sent} bytes."))?;
        Ok(json!({
            "bytes": sent,
            "containers": manifest.containers.len(),
            "images": manifest.images.len(),
            "volumes": manifest.volumes.iter().filter(|v| v.included).count(),
            "endpoints": manifest.endpoints.len(),
            "scale_sets": manifest.scale_sets.len(),
            "parts": manifest.parts.len(),
            "secrets": manifest.included.secrets,
        }))
    }

    /// The records of the included kinds and what Docker still has of them.
    async fn read_installation(&self) -> anyhow::Result<Installation> {
        // Every record or none: a backup that silently lacked one would look whole until the day
        // it is restored, so a record this release cannot read fails it, as a part that cannot
        // be read does.
        let mut containers: Vec<Container> = self.db.list_complete(CONTAINERS)?;
        let images: Vec<ImageRecord> = self.db.list_complete("image")?;
        let composes: Vec<(String, Pinned)> = self.db.entries_complete(PINNED)?;
        let endpoints: Vec<Endpoint> = self.db.list_complete(ENDPOINTS)?;
        let proxy_passwords: Vec<(String, Credentials)> = self.db.entries_complete(CREDENTIALS)?;
        let scale_sets: Vec<ScaleSet> = self.db.list_complete(SETS)?;
        let scale_set_templates: Vec<(String, Template)> = self.db.entries_complete(TEMPLATES)?;
        let image_ids = lines(
            &self
                .docker(&["image", "ls", "-a", "--no-trunc", "--format", "{{.ID}}"])
                .await?,
        );
        let names = lines(&self.docker(&["ps", "-a", "--format", "{{.Names}}"]).await?);
        let mut with_files = HashSet::new();
        for container in &mut containers {
            if names.contains(&container.container) {
                let info = self.owned(container).await?;
                // A record from before identities were kept takes the one its container has, so
                // the restored container is the same machine.
                crate::identity::adopt(container, Some(&info));
                with_files.insert(container.id.clone());
            } else if container
                .working_image
                .as_ref()
                .is_some_and(|working| image_ids.contains(&working.image_id))
            {
                with_files.insert(container.id.clone());
            }
        }
        let available_images = images
            .iter()
            .filter(|record| image_ids.contains(&record.image_id))
            .map(|record| record.id.clone())
            .collect();
        let set_volumes = scale_sets
            .iter()
            .flat_map(|set| {
                let template = scale_set_templates
                    .iter()
                    .find(|(name, _)| *name == set.name);
                template_volumes(set, template.map(|(_, template)| template))
            })
            .collect();
        let volumes = self.backup_volumes(&containers, set_volumes).await?;
        Ok(Installation {
            containers,
            images,
            composes,
            endpoints,
            proxy_passwords,
            scale_sets,
            scale_set_templates,
            available_images,
            with_files,
            volumes,
        })
    }

    /// The volumes a backup may carry: those the containers mount and `set_volumes`, those the
    /// scale sets' templates mount, also one `VOLUME_FILTER` leaves out, and those Thoric created
    /// that the volume list shows; never the manager's.
    async fn backup_volumes(
        &self,
        containers: &[Container],
        set_volumes: BTreeSet<String>,
    ) -> anyhow::Result<BTreeMap<String, VolumeFacts>> {
        let all = lines(
            &self
                .docker(&["volume", "ls", "--format", "{{.Name}}"])
                .await?,
        );
        let listed: HashSet<String> = String::from_utf8_lossy(
            &self
                .docker_args(&crate::volumes::list_args(
                    self.config.volume_filter.as_deref(),
                ))
                .await?,
        )
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|volume| volume["Name"].as_str().map(str::to_owned))
        .collect();
        let managed = lines(
            &self
                .docker(&[
                    "volume",
                    "ls",
                    "--filter",
                    &format!("label={LABEL}=true"),
                    "--format",
                    "{{.Name}}",
                ])
                .await?,
        );
        let mounted: BTreeSet<String> = containers
            .iter()
            .flat_map(Container::mounts)
            .map(|mount| mount.volume)
            .chain(set_volumes)
            .collect();
        let manager = self.listing_manager().await.map_err(|e| anyhow!(e.1))?;
        let mut volumes = BTreeMap::new();
        for name in listed
            .intersection(&managed)
            .cloned()
            .chain(mounted)
            .collect::<BTreeSet<_>>()
        {
            if !all.contains(&name)
                || manager
                    .as_ref()
                    .is_some_and(|manager| crate::volumes::manager_volume(manager, &name))
            {
                continue;
            }
            let info = self.inspect("volume", &name).await?;
            let label = |key: &str| info["Labels"][key].as_str().map(str::to_owned);
            volumes.insert(
                name,
                VolumeFacts {
                    owner: label(crate::volumes::OWNER),
                    parameter: label(crate::volumes::PARAMETER),
                },
            );
        }
        Ok(volumes)
    }
}

/// What a backup holds, for its job log: names and counts, never a value.
fn describe(manifest: &Manifest) -> String {
    let files = manifest.containers.iter().filter(|c| c.files).count();
    let images = manifest.images.iter().filter(|i| i.image).count();
    let volumes = manifest.volumes.iter().filter(|v| v.included).count();
    format!(
        "The backup holds {} containers, the files of {files} of them, {} saved image records, {images} of them with their images, {volumes} volumes, {} MCP endpoints and {} scale sets. {}",
        manifest.containers.len(),
        manifest.images.len(),
        manifest.endpoints.len(),
        manifest.scale_sets.len(),
        if manifest.included.secrets {
            "It includes the values of secret parameters and proxy passwords: keep the file where only you can read it."
        } else {
            "The values of secret parameters and proxy passwords are left out."
        }
    )
}

fn lines(bytes: &[u8]) -> HashSet<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
