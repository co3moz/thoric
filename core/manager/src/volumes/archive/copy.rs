//! A volume copied into a new one, for a task that starts from a prepared volume or a copy kept
//! before a risky change. Docker has no volume copy, so the export helper reads the source as
//! root on a read-only mount and the import helper unpacks into the new volume as the desktop
//! user; the manager pipes one into the other, without gzip. No helper mounts both volumes.
use super::{EXTRACT_LIMIT, Helper, PROGRESS_INTERVAL, READ_SIZE, Stopped, Unpacking};
use crate::{
    App,
    archives::transfer_permit,
    error::{Error, Result},
    jobs,
    model::volume_name,
    process::{self, Overflow},
    volumes::{Access, MOUNT, Volume},
};
use anyhow::{Context, anyhow, bail};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// How tar unpacks a copy: as the desktop user, keeping the source's modes. A setuid bit grants
/// nothing beyond that user, who owns every file.
const COPIED: &[&str] = &["--preserve-permissions"];

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CopyVolume {
    /// The new volume's name.
    pub name: String,
}

/// Copy a volume's files into a new volume. Returns a job; the copy belongs to the desktop user,
/// as after an import.
pub async fn copy(
    State(app): State<Arc<App>>,
    Path(source): Path<String>,
    Json(request): Json<CopyVolume>,
) -> Result<Json<Value>> {
    volume_name(&request.name)?;
    // Refused before anything starts; the job checks again under the new name's lock.
    app.open_volume(&source).await?;
    app.volume_named(&request.name).await?;
    if app
        .optional_inspect("volume", &request.name)
        .await?
        .is_some()
    {
        return Err(Error(
            StatusCode::CONFLICT,
            "A volume with this name already exists; copy into a new name".into(),
        ));
    }
    let permit = transfer_permit(&app)?;
    let (job_id, _) = jobs::start(
        app,
        "volume-copy",
        format!("volume:{}", request.name),
        true,
        move |app, job_id| async move {
            let _permit = permit;
            app.copy_volume(&job_id, &source, &request.name).await
        },
    )?;
    Ok(Json(json!({"job_id": job_id})))
}

impl App {
    async fn copy_volume(&self, job_id: &str, source: &str, name: &str) -> anyhow::Result<Value> {
        // `docker run` would create a source that went missing since the request was checked.
        let source = self
            .open_volume(source)
            .await
            .map_err(|error| anyhow!(error.1))?;
        let target = self
            .volume_named(name)
            .await
            .map_err(|error| anyhow!(error.1))?;
        self.new_volume(name)
            .await
            .map_err(|error| anyhow!(error.1))?;
        self.append_log(
            job_id,
            &format!(
                "Created the volume {name}. Copying the files of {}.",
                source.name
            ),
        )?;
        match self.pipe(job_id, &source, &target).await {
            Ok(copied) => {
                self.append_log(job_id, &format!("Copied {copied} bytes of files."))?;
                Ok(json!({"source": source.name, "name": name, "bytes": copied}))
            }
            Err(error) => {
                // This copy created the volume a moment ago and holds its lock; an incomplete
                // copy is removed rather than left looking like a good one.
                if self.docker(&["volume", "rm", name]).await.is_ok() {
                    let _ = self.append_log(job_id, "Removed the incomplete volume.");
                }
                Err(error)
            }
        }
    }

    /// Stream the source's files as a plain tar into the tar that unpacks the new volume.
    /// Returns the bytes of the tar stream.
    pub(crate) async fn pipe(
        &self,
        job_id: &str,
        source: &Volume,
        target: &Volume,
    ) -> anyhow::Result<u64> {
        let progress = |current: u64| json!({"transfer": "copy", "current": current});
        self.set_progress(job_id, Some(progress(0)))?;
        let Unpacking {
            helper: mut writer,
            mut stdin,
            errors: written,
        } = self
            .start_unpack(target, "/", COPIED)
            .await
            .map_err(|error| anyhow!(error.1))?;
        let mut reader = match Helper::spawn_reader(
            self,
            source,
            Access::Archive,
            &[
                "tar",
                "--create",
                "--numeric-owner",
                "--file",
                "-",
                "--directory",
                MOUNT,
                ".",
            ],
        )
        .await
        {
            Ok(reader) => reader,
            Err(error) => {
                writer.abandon(self).await;
                return Err(error);
            }
        };
        let mut stdout = reader.child.stdout.take().context("Missing tar output")?;
        let read = tokio::spawn(process::read_limited(
            reader.child.stderr.take().context("Missing tar errors")?,
            8192,
            Overflow::Tail,
        ));
        let mut buffer = vec![0; READ_SIZE];
        let (mut copied, mut reported) = (0u64, Instant::now());
        let streamed = async {
            loop {
                let count = stdout
                    .read(&mut buffer)
                    .await
                    .context("Could not read the volume's files")?;
                if count == 0 {
                    break;
                }
                if stdin.write_all(&buffer[..count]).await.is_err() {
                    return Err(anyhow!(Stopped));
                }
                copied += count as u64;
                if reported.elapsed() >= PROGRESS_INTERVAL {
                    self.set_progress(job_id, Some(progress(copied)))?;
                    reported = Instant::now();
                }
            }
            if stdin.shutdown().await.is_err() {
                return Err(anyhow!(Stopped));
            }
            anyhow::Ok(())
        }
        .await;
        drop(stdin);
        if let Err(error) = streamed {
            reader.abandon(self).await;
            if error.is::<Stopped>() {
                // The unpacking tar stopped reading; its own error output says why.
                let status = writer.child.wait().await;
                let errors = String::from_utf8_lossy(&written.await??).trim().to_owned();
                if status.is_err() || errors.is_empty() {
                    writer.abandon(self).await;
                    bail!("tar stopped reading the copy");
                }
                bail!("tar: {errors}");
            }
            writer.abandon(self).await;
            return Err(error);
        }
        let status = reader.child.wait().await?;
        let errors = String::from_utf8_lossy(&read.await??).trim().to_owned();
        match status.code() {
            Some(0) => {}
            // GNU tar ends with 1 when a file changed while it was read, as one in use can; the
            // copy is complete and holds the file as it was read.
            Some(1) => self.append_log(job_id, &format!("tar: {errors}"))?,
            _ => {
                writer.abandon(self).await;
                bail!("tar: {errors}");
            }
        }
        self.set_progress(job_id, None)?;
        let status = match tokio::time::timeout(EXTRACT_LIMIT, writer.child.wait()).await {
            Ok(status) => status?,
            Err(_) => {
                writer.abandon(self).await;
                bail!("Unpacking the copy exceeded the one-hour limit");
            }
        };
        let errors = String::from_utf8_lossy(&written.await??).trim().to_owned();
        if !status.success() {
            bail!("tar: {errors}");
        }
        Ok(copied)
    }
}

#[cfg(test)]
#[path = "copy_tests.rs"]
mod tests;
