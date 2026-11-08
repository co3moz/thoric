//! Requests the manager leaves for the WSL package: the dashboard's update and its settings
//! changes.
//!
//! Both end in a command that recreates this container, which nothing inside it can do, and the
//! manager never reaches into the distribution itself. Instead the package mounts
//! `/opt/thoric/update` at [`crate::releases::UPDATE_DIR`]. For each kind of request, a
//! [`Channel`], the manager leaves a request naming its job on the request's first line; a path
//! unit in the distribution starts a service, which renames the request to the channel's
//! `started`, runs the command as root with its output in `log` and writes how it ended to `exit`.
//! The job follows that log. The command replaces this container on the way, so the next manager
//! finds the job still running and follows it to its end; see [`resumable`].
use crate::{App, jobs, model::Job, process::TextDecoder};
use anyhow::{Context, bail};
use serde_json::Value;
use std::{
    io::{ErrorKind, SeekFrom},
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

/// One kind of request: the names of its files in the folder and of what takes it.
pub(crate) struct Channel {
    /// The kind of the job that follows a request.
    pub job: &'static str,
    pub request: &'static str,
    pub started: &'static str,
    pub log: &'static str,
    pub exit: &'static str,
    /// The path unit that takes the request, named when nobody does.
    pub path_unit: &'static str,
    /// What the service runs, as a terminal would.
    pub command: &'static str,
}

/// How long a job waits for each thing it expects.
#[derive(Clone, Copy)]
pub(crate) struct Timing {
    /// The distribution taking the request, which its path unit does at once.
    pub take: Duration,
    /// Between two reads of the log.
    pub poll: Duration,
    /// The whole command: the service's own limit and a little more to read what it wrote last.
    pub limit: Duration,
}

/// How much of the log one read takes; the rest comes with the next.
const READ_LIMIT: u64 = 1 << 20;
/// The longest line kept until its end comes: a line this long, such as a progress line a
/// command redraws with carriage returns, is added as it is and the rest of it as the next.
const LINE_LIMIT: usize = 64 << 10;

pub(crate) fn active(job: &Job) -> bool {
    job.status == "queued" || job.status == "running"
}

/// Leave a request with `lines` after the job's ID, wait for the distribution to take it and
/// follow the command to its end. Returns how the service ended, as it wrote it.
pub(crate) async fn request(
    app: &App,
    channel: &Channel,
    id: &str,
    lines: &[String],
    timing: Timing,
) -> anyhow::Result<String> {
    let dir = &app.config.update_dir;
    // Renamed into place, so the path unit never finds half a request.
    let partial = dir.join(format!("{}.new", channel.request));
    // What the last request left, also half of one; the request alone names this job.
    for name in [channel.request, channel.started, channel.log, channel.exit] {
        remove(&dir.join(name)).await?;
    }
    remove(&partial).await?;
    let mut body = format!("{id}\n");
    for line in lines {
        body.push_str(line);
        body.push('\n');
    }
    write_private(&partial, &body)
        .await
        .with_context(|| format!("Could not write {}", partial.display()))?;
    tokio::fs::rename(&partial, dir.join(channel.request)).await?;
    let taken = async {
        loop {
            if started_by(dir, channel).await?.as_deref() == Some(id) {
                return anyhow::Ok(());
            }
            tokio::time::sleep(timing.poll / 4).await;
        }
    };
    if let Ok(result) = tokio::time::timeout(timing.take, taken).await {
        result?;
    } else {
        // A request taken later would start a command that nobody follows.
        remove(&dir.join(channel.request)).await?;
        if started_by(dir, channel).await?.as_deref() != Some(id) {
            bail!(
                "The WSL distribution did not take the request. Check 'systemctl status {}' there, or run '{}' in a Thoric terminal",
                channel.path_unit,
                channel.command
            );
        }
    }
    follow(app, channel, id, timing).await
}

/// The job the request the distribution took last names.
async fn started_by(dir: &Path, channel: &Channel) -> anyhow::Result<Option<String>> {
    Ok(read_text(&dir.join(channel.started))
        .await?
        .and_then(|text| text.lines().next().map(|line| line.trim().to_owned())))
}

/// Whether a job outlived the manager that started it: the distribution took its request, so
/// the command replaced that manager and goes on, or ended while no manager was there to see it.
pub(crate) fn resumable(channel: &Channel, dir: &Path, job: &Job) -> bool {
    job.kind == channel.job
        && active(job)
        && std::fs::read_to_string(dir.join(channel.started))
            .is_ok_and(|started| started.lines().next().map(str::trim) == Some(job.id.as_str()))
}

/// Follow a command the previous manager asked for, to its end, and let `outcome` make the
/// job's result of how the service ended.
pub(crate) fn resume(
    app: Arc<App>,
    channel: &'static Channel,
    job: Job,
    timing: Timing,
    outcome: fn(&str) -> anyhow::Result<Value>,
) {
    tokio::spawn(async move {
        let id = job.id.clone();
        let result = async {
            // The log is read again from its start.
            app.db.restart_job_log(
                &id,
                &format!(
                    "The manager restarted while {} ran in the WSL distribution; its output follows.\n",
                    channel.command
                ),
            )?;
            let ended = follow(&app, channel, &id, timing).await?;
            outcome(&ended)
        }
        .await;
        let _ = jobs::finish(&app, &id, job, result);
    });
}

async fn follow(app: &App, channel: &Channel, id: &str, timing: Timing) -> anyhow::Result<String> {
    let dir = &app.config.update_dir;
    let mut log = LogReader::default();
    let mut waited = Duration::ZERO;
    loop {
        // The service writes how it ended after its last output, so the log is whole by then.
        let ended = read_text(&dir.join(channel.exit)).await?;
        let mut read = log.read(app, id, &dir.join(channel.log)).await?;
        if let Some(ended) = ended {
            while read > 0 {
                read = log.read(app, id, &dir.join(channel.log)).await?;
            }
            log.finish(app, id)?;
            return Ok(ended);
        }
        if waited >= timing.limit {
            bail!("{} did not end in time", channel.command);
        }
        tokio::time::sleep(timing.poll).await;
        waited += timing.poll;
    }
}

/// Whether the service ended well, from what it wrote when it ended: systemd's result for the
/// service and the command's exit status.
pub(crate) fn succeeded(ended: &str) -> bool {
    ended.split_whitespace().next() == Some("success")
}

/// The command's output as it grows, added to the job a whole line at a time.
#[derive(Default)]
struct LogReader {
    offset: u64,
    decoder: TextDecoder,
    partial: String,
}

impl LogReader {
    /// Add what was written since the last read; returns how many bytes that was.
    async fn read(&mut self, app: &App, id: &str, path: &Path) -> anyhow::Result<u64> {
        let mut file = match tokio::fs::File::open(path).await {
            Ok(file) => file,
            // The service has not started the command yet.
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error).context("Could not read the command's log"),
        };
        file.seek(SeekFrom::Start(self.offset)).await?;
        let mut bytes = Vec::new();
        file.take(READ_LIMIT).read_to_end(&mut bytes).await?;
        self.offset += bytes.len() as u64;
        self.partial.push_str(&self.decoder.push(&bytes));
        if let Some(end) = self.partial.rfind('\n') {
            let lines: String = self.partial.drain(..=end).collect();
            app.append_log(id, lines.trim_end_matches('\n'))?;
        }
        if self.partial.len() >= LINE_LIMIT {
            app.append_log(id, &std::mem::take(&mut self.partial))?;
        }
        Ok(bytes.len() as u64)
    }
    fn finish(&mut self, app: &App, id: &str) -> anyhow::Result<()> {
        self.partial.push_str(&self.decoder.finish());
        if !self.partial.is_empty() {
            app.append_log(id, &std::mem::take(&mut self.partial))?;
        }
        Ok(())
    }
}

async fn read_text(path: &Path) -> anyhow::Result<Option<String>> {
    match tokio::fs::read_to_string(path).await {
        Ok(text) => Ok(Some(text.trim().to_owned())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Could not read {}", path.display())),
    }
}

/// Write a new file only its owner reads: a settings change can carry a secret such as
/// `NOTIFY_URL`, and the folder is the distribution's, where anyone may list it. The service that
/// takes the request runs as root.
async fn write_private(path: &Path, body: &str) -> std::io::Result<()> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path).await?;
    file.write_all(body.as_bytes()).await?;
    // A tokio file finishes its last write in the background unless asked to wait for it.
    file.flush().await
}

async fn remove(path: &Path) -> anyhow::Result<()> {
    match tokio::fs::remove_file(path).await {
        Err(error) if error.kind() != ErrorKind::NotFound => {
            Err(error).with_context(|| format!("Could not remove {}", path.display()))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
#[path = "package_requests_tests.rs"]
mod tests;
