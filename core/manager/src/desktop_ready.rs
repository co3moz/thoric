//! When a desktop counts as started: once its session has written `/run/desktop/ready`, which the
//! session of every desktop image level writes when its window manager runs and removes when it
//! ends, and which the image's health check reads too. Every job that starts a desktop waits for
//! it before it reports done: a creation, a start or restart, a retry, a reset and a settings
//! change that creates the container again, which restores the previous container when the new
//! one does not get there. A desktop that stops, that Docker keeps restarting or that is not
//! ready in time fails the job with its exit code and the last lines it logged. A ready desktop
//! that Docker started at another resolution than its settings name is given theirs
//! (`resolution`).
use crate::{App, model::Container};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::time::{Duration, Instant};

/// The file the desktop session writes once its window manager runs.
const READY_FILE: &str = "/run/desktop/ready";

/// How long a desktop is waited for, how often it is looked at, and how many restarts by Docker's
/// restart policy while it is waited for mean that it keeps failing.
pub(crate) struct Patience {
    pub limit: Duration,
    pub interval: Duration,
    pub restarts: u64,
}

/// What every job waits with. A GNOME session or a GPU desktop on a busy host can take a minute;
/// a desktop that fails as it starts, such as at its GPU check, is restarted by Docker within
/// seconds, with a growing delay between attempts.
pub(crate) const PATIENCE: Patience = Patience {
    limit: Duration::from_secs(300),
    interval: Duration::from_secs(1),
    restarts: 3,
};

/// How many of the container's last log lines a failure quotes, and at most how much of them.
const LOG_LINES: &str = "20";
const LOG_BYTES: usize = 4096;

impl App {
    /// Wait until the desktop of a container that was just started is ready, give it the
    /// resolution its settings name when Docker started it at another, and return the
    /// container's ID.
    pub(crate) async fn desktop_ready(&self, env: &Container, job_id: &str) -> Result<String> {
        let info = self.wait_ready(env, job_id, &PATIENCE).await?;
        // The desktop works at either size, so one that kept its own still counts as started.
        match self.keep_resolution(env, &info).await {
            Ok(Some(resolution)) => self.append_log(
                job_id,
                &format!("Gave the desktop the resolution {resolution} its settings name."),
            )?,
            Ok(None) => {}
            Err(error) => self.append_log(
                job_id,
                &format!("The desktop kept the resolution it was created with: {error:#}"),
            )?,
        }
        Ok(info["Id"]
            .as_str()
            .context("Missing container ID")?
            .to_owned())
    }

    /// Whether the desktop of the Docker container `id` is ready now.
    pub(crate) async fn desktop_is_ready(&self, id: &str) -> bool {
        self.docker(&["exec", "--user", "1001:1001", id, "test", "-e", READY_FILE])
            .await
            .is_ok()
    }

    /// Wait as [`App::desktop_ready`] does, with the patience given, which tests shorten, and
    /// return what `docker inspect` said of the ready container.
    pub(crate) async fn wait_ready(
        &self,
        env: &Container,
        job_id: &str,
        patience: &Patience,
    ) -> Result<Value> {
        let started = Instant::now();
        let mut counted = None;
        let mut waited = false;
        loop {
            let info = self.owned(env).await?;
            let id = info["Id"]
                .as_str()
                .context("Missing container ID")?
                .to_owned();
            let state = &info["State"];
            // Restarts by hand count none, and the count from before the wait is not this one's.
            let restarts = info["RestartCount"].as_u64().unwrap_or(0);
            let restarted = restarts.saturating_sub(*counted.get_or_insert(restarts));
            // Docker reports a container it is about to restart as running and restarting.
            if state["Running"] == true && state["Restarting"] != true {
                if self.desktop_is_ready(&id).await {
                    if waited {
                        self.append_log(job_id, "The desktop is ready.")?;
                    }
                    return Ok(info);
                }
            } else if state["Restarting"] != true {
                bail!(
                    self.unready(&id, state, "The desktop stopped before it was ready".into())
                        .await
                );
            }
            if restarted >= patience.restarts {
                let what =
                    format!("The desktop restarted {restarted} times without becoming ready");
                bail!(self.unready(&id, state, what).await);
            }
            if started.elapsed() >= patience.limit {
                let what = format!(
                    "The desktop was not ready within {} seconds",
                    patience.limit.as_secs()
                );
                bail!(self.unready(&id, state, what).await);
            }
            if !waited {
                self.append_log(job_id, "Waiting for the desktop to be ready.")?;
                waited = true;
            }
            tokio::time::sleep(patience.interval).await;
        }
    }

    /// Why a desktop is not ready: `what` happened, the code it last exited with and the last
    /// lines it logged.
    async fn unready(&self, id: &str, state: &Value, what: String) -> String {
        let mut text = what;
        // A desktop that is still running has not exited since it started.
        if state["Running"] != true || state["Restarting"] == true {
            if let Some(code) = state["ExitCode"].as_i64() {
                text.push_str(&format!("; it last exited with code {code}"));
            }
            if let Some(error) = state["Error"].as_str().filter(|e| !e.is_empty()) {
                text.push_str(&format!(" ({error})"));
            }
        }
        text.push('.');
        let lines = self.last_lines(id).await;
        if !lines.is_empty() {
            text.push_str(" Its last log lines:\n");
            text.push_str(&lines);
        }
        text
    }

    /// The last lines a container logged, its output before its errors, or nothing when Docker
    /// cannot read them.
    async fn last_lines(&self, id: &str) -> String {
        let args = ["logs", "--tail", LOG_LINES, id].map(str::to_owned);
        let output = crate::process::capture(
            self.command(&args),
            Duration::from_secs(15),
            LOG_BYTES,
            crate::process::Overflow::Tail,
        )
        .await;
        match output {
            Ok(output) if output.status.success() => {
                let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&output.stderr));
                text.trim_end().to_owned()
            }
            _ => String::new(),
        }
    }
}

#[cfg(test)]
#[path = "desktop_ready_tests.rs"]
mod tests;
