use crate::{
    App,
    docker::logged::Ran,
    error::Result,
    files,
    human::Caller,
    jobs::{self, launch_shared},
    model::LOG_LIMIT,
};
use anyhow::{Context, bail};
use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

// The script is copied to an unlinked temporary file first, so commands that read stdin see
// end-of-file instead of consuming the rest of the script, and large scripts avoid argument limits.
const WRAPPER: &str = "script=$(mktemp /tmp/thoric-exec.XXXXXX) && cat >\"$script\" && exec 3<\"$script\" && rm -f -- \"$script\" && exec bash -e /dev/fd/3 </dev/null";

/// `docker exec` arguments up to the container, for a script that reads its input from the
/// manager and runs in the desktop session's environment. Root starts in its own home, since a
/// Python started there, such as the compose runner or a script's `python3 -m`, imports from the
/// folder it starts in, and the desktop user writes theirs.
pub(crate) fn desktop_exec(user: &str, root: bool) -> Vec<String> {
    [
        "exec",
        "--interactive",
        "--user",
        user,
        "--workdir",
        if root { "/root" } else { "/home/desktop" },
        "--env",
        if root {
            "HOME=/root"
        } else {
            "HOME=/home/desktop"
        },
        "--env",
        "DISPLAY=:0",
        "--env",
        "PULSE_SERVER=unix:/run/desktop/pulse/native",
        "--env",
        "XDG_RUNTIME_DIR=/run/desktop",
    ]
    .map(str::to_owned)
    .to_vec()
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Execute {
    pub script: String,
    #[serde(default)]
    pub root: bool,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    /// Files the script needs, such as the program it runs, written before it starts as
    /// `POST /api/containers/{id}/files/write` writes them: as the desktop user, also for a root
    /// script. At most 16 files of 512 KiB each and 1 MiB together.
    #[serde(default)]
    pub files: Vec<ScriptFile>,
    /// Wait up to this many seconds, at most 300, for the script to end and answer with its job
    /// as `GET /api/jobs/{id}` does, with `job_id` beside it. Without it the answer is `job_id`
    /// alone, at once.
    #[serde(default)]
    pub wait_seconds: Option<u64>,
    /// With `wait_seconds`, return at most the last this many bytes of the job's log. Left out,
    /// the log is its last 4096 bytes, or empty once `result.output_tail` carries the output.
    #[serde(default)]
    pub log_tail_bytes: Option<usize>,
}
fn default_timeout() -> u64 {
    300
}

/// One file written for a script before it runs.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScriptFile {
    /// Canonical absolute path of the file. Missing folders are created.
    pub path: String,
    /// The file's content, as `encoding` says.
    pub data: String,
    #[serde(default)]
    pub encoding: files::Encoding,
    /// Write over an existing file.
    #[serde(default)]
    pub replace: bool,
}

/// The most files one script takes with it.
pub(crate) const FILES: usize = 16;
/// The most bytes those files hold together.
pub(crate) const FILES_BYTES: usize = 1024 * 1024;
/// How long `exec_container` waits for its script when the agent does not say: most scripts end
/// within it, so their result needs no `get_job`, and one that does not still answers soon.
pub(crate) const MCP_WAIT: u64 = 5;

/// A script's file as it is written: where, what, and whether over an existing file.
struct Delivery {
    path: String,
    data: Vec<u8>,
    replace: bool,
}

impl Execute {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.script.trim().is_empty()
            || self.script.len() > 1_000_000
            || self.script.contains('\0')
        {
            bail!("Script must contain 1 to 1000000 bytes without NUL characters");
        }
        if !(1..=3500).contains(&self.timeout_seconds) {
            bail!("timeout_seconds must be between 1 and 3500");
        }
        if self
            .wait_seconds
            .is_some_and(|wait| wait > jobs::read::MAX_WAIT)
        {
            bail!("wait_seconds must be at most {}", jobs::read::MAX_WAIT);
        }
        if self.log_tail_bytes.is_some_and(|tail| tail > LOG_LIMIT) {
            bail!("log_tail_bytes must be at most {LOG_LIMIT}");
        }
        Ok(())
    }
    /// The files to write, decoded, or why the request is refused before anything runs: too
    /// many, too large, a path that is not canonical or named twice, or data that does not decode.
    fn deliveries(&self) -> anyhow::Result<Vec<Delivery>> {
        if self.files.len() > FILES {
            bail!("A script takes at most {FILES} files");
        }
        let mut deliveries: Vec<Delivery> = Vec::with_capacity(self.files.len());
        let mut total = 0;
        for file in &self.files {
            files::split_path(&file.path)?;
            if deliveries.iter().any(|other| other.path == file.path) {
                bail!("files names {} twice", file.path);
            }
            let data = file
                .encoding
                .decode(&file.data)
                .with_context(|| format!("files: {}", file.path))?;
            total += data.len();
            if total > FILES_BYTES {
                bail!("A script's files are limited to 1 MiB together");
            }
            deliveries.push(Delivery {
                path: file.path.clone(),
                data,
                replace: file.replace,
            });
        }
        Ok(deliveries)
    }
}
pub async fn execute(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<Execute>,
) -> Result<Json<Value>> {
    Ok(Json(run(app, id, body, None).await?))
}

/// Start the script's job and, when the request waits, answer with the job as it is once it ends
/// or the wait is over, as a read of the job answers. `job_id` is in every answer. `caller` is the
/// MCP call that waits, which hears progress and may give the wait up.
pub(crate) async fn run(
    app: Arc<App>,
    id: String,
    body: Execute,
    caller: Option<&Caller>,
) -> Result<Value> {
    body.validate()?;
    let deliveries = body.deliveries()?;
    let wait = body.wait_seconds.filter(|wait| *wait > 0);
    let tail = body.log_tail_bytes;
    let Json(started) = start(app.clone(), id, body, deliveries)?;
    let Some(wait) = wait else {
        return Ok(started);
    };
    let job_id = started["job_id"].as_str().unwrap_or_default().to_owned();
    let read = jobs::read::Read {
        wait_seconds: Some(wait),
        log_tail_bytes: Some(tail.unwrap_or(jobs::read::MCP_TAIL)),
        log_offset: None,
    };
    let mut job = jobs::read::answer(&app, &job_id, &read, caller).await?;
    if job["finished_at"].is_null() {
        job["hint"] = json!(STILL_RUNNING);
    } else if tail.is_none() && job["result"]["output_tail"].is_string() {
        // The result carries the script's output; the log would repeat it.
        job["log"] = json!("");
        job["log_start"] = job["log_end"].clone();
    }
    job["job_id"] = json!(job_id);
    Ok(job)
}

/// What an answer says of a script that outlived the call's wait.
const STILL_RUNNING: &str = "The script is still running. Call get_job with this job_id and wait_seconds, up to 300, to wait for it to end; do not poll without wait_seconds.";

fn start(
    app: Arc<App>,
    id: String,
    body: Execute,
    deliveries: Vec<Delivery>,
) -> Result<Json<Value>> {
    app.container(&id)?;
    launch_shared(
        app,
        "exec",
        format!("container:{id}"),
        move |app, job_id| async move {
            let env = app.container(&id).map_err(|e| anyhow::anyhow!(e.1))?;
            let info = app.owned(&env).await?;
            if info["State"]["Running"] != true {
                bail!("Start the container before executing a script");
            }
            let user = if body.root { "root" } else { "1001:1001" };
            app.append_log(
                &job_id,
                &format!(
                    "Running a script in \"{}\" as {} (timeout: {} seconds).",
                    env.spec.name, user, body.timeout_seconds
                ),
            )?;
            let container = info["Id"].as_str().context("Missing container ID")?;
            // The desktop user's files whoever runs the script: root reads them all the same,
            // and nothing is written where only root may write.
            for file in deliveries {
                let size = file.data.len();
                files::deliver(&app, container, &file.path, file.data, file.replace)
                    .await
                    .map_err(|error| {
                        let hint = if error.0 == axum::http::StatusCode::CONFLICT {
                            " Set replace to true for that file to write over it."
                        } else {
                            ""
                        };
                        anyhow::anyhow!(
                            "Could not write {}, so the script did not run: {}{hint}",
                            file.path,
                            error.1
                        )
                    })?;
                app.append_log(
                    &job_id,
                    &format!("Wrote {} ({size} bytes) as the desktop user.", file.path),
                )?;
            }
            // The script's name, its $0, marks its `timeout` for a cancel to find.
            let marker = format!("thoric-exec-{job_id}");
            let mut args = desktop_exec(user, body.root);
            args.extend([
                container.into(),
                "timeout".into(),
                "--signal=TERM".into(),
                "--kill-after=5s".into(),
                format!("{}s", body.timeout_seconds),
                "bash".into(),
                "-c".into(),
                WRAPPER.into(),
                marker.clone(),
            ]);
            app.cancels.on_stop(&job_id, {
                let app = app.clone();
                let args = stop_args(container, user, &marker);
                async move {
                    if let Err(error) = app.docker_args(&args).await {
                        tracing::debug!("Stopping a cancelled script: {error:#}");
                    }
                }
            });
            let started = std::time::Instant::now();
            let ran = app
                .docker_logged_outcome(&job_id, &args, body.script.into_bytes())
                .await?;
            let limit = std::time::Duration::from_secs(body.timeout_seconds);
            let outcome = Outcome::of(&ran, started.elapsed() >= limit);
            let failure = outcome.failure(body.timeout_seconds);
            let result = json!({
                "container_id": id,
                "user": user,
                "exit_code": outcome.exit_code,
                "timed_out": outcome.timed_out,
                "output_tail": outcome.output_tail,
            });
            match failure {
                None => {
                    app.append_log(&job_id, "Script completed successfully.")?;
                    Ok(result)
                }
                Some(reason) => {
                    app.append_log(&job_id, &format!("{reason}."))?;
                    Err(crate::jobs::Failed::new(reason, result).into())
                }
            }
        },
    )
}

/// The `docker exec` that stops a cancelled script in its desktop as its time limit would: `timeout`,
/// found by the `marker` it was started with, passes the TERM on to the script's processes and
/// sends KILL five seconds later. It runs as the script's user, so it reaches nothing else.
pub(crate) fn stop_args(container: &str, user: &str, marker: &str) -> Vec<String> {
    [
        "exec",
        "--user",
        user,
        container,
        "pkill",
        "--signal",
        "TERM",
        "--full",
        &format!("^timeout .* {marker}$"),
    ]
    .map(str::to_owned)
    .to_vec()
}

/// How a script ended, which its job's result carries whether it succeeded or failed.
#[derive(Debug, PartialEq)]
pub(crate) struct Outcome {
    /// The script's exit code, `0` when it succeeded; `null` when it ran out of the manager's own
    /// time or a signal ended it.
    pub exit_code: Option<i32>,
    /// Whether it ran out of `timeout_seconds` and was stopped.
    pub timed_out: bool,
    /// The end of what it printed, at most [`crate::docker::logged::TAIL_LIMIT`] bytes, without the manager's lines.
    pub output_tail: String,
}

impl Outcome {
    /// `ran` of a script whose time was up when it ended (`over`). `timeout` reports a script it
    /// stopped as 124, or as 137 when the script outlived the TERM by the five seconds it is
    /// given; a script that exits with either code before its time is not taken for one.
    pub(crate) fn of(ran: &Ran, over: bool) -> Self {
        let exit_code = ran.status.and_then(|status| status.code());
        Self {
            exit_code,
            timed_out: ran.status.is_none() || (over && matches!(exit_code, Some(124 | 137))),
            output_tail: ran.tail.clone(),
        }
    }
    /// Why the script failed, for its job's error and log, or `None` when it succeeded.
    fn failure(&self, timeout_seconds: u64) -> Option<String> {
        if self.timed_out {
            let code = self
                .exit_code
                .map(|code| format!(" (exit code {code})"))
                .unwrap_or_default();
            return Some(format!(
                "Script ran out of its time limit of {timeout_seconds} seconds and was stopped{code}"
            ));
        }
        match self.exit_code {
            Some(0) => None,
            Some(code) => Some(format!("Script exited with code {code}")),
            None => Some("Script was ended by a signal".into()),
        }
    }
}

#[cfg(all(test, unix))]
#[path = "exec_tests.rs"]
mod tests;
