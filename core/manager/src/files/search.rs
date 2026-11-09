//! Searching a running desktop's files by content, by name or both: the MCP tool `search_files`
//! and `POST /api/containers/{id}/files/search`, so an agent does not run grep or find as a script
//! job and poll it. The helper runs as the desktop user and follows no symlink, as reading a file
//! does, and every direction is bounded: its time, the entries it looks at, the bytes it reads of
//! a file, the characters of a line and the size of its answer.
use super::running_container;
use crate::{
    App,
    error::{Error, Result},
    process,
};
use anyhow::{Context, bail};
use axum::{
    Json,
    extract::{Path, State},
};
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{sync::Arc, time::Duration};

const HELPER: &str = include_str!("../../helpers/search_files.py");
/// The most results one search returns.
pub const MAX_RESULTS: usize = 1000;
/// The most lines of context on either side of a match.
pub const MAX_CONTEXT: u8 = 10;
/// The longest a search may run, in seconds.
pub const MAX_SECONDS: u64 = 30;
/// The characters of a pattern, which is one line of text.
const PATTERN: usize = 1000;
/// The characters of a name glob, which no file name outgrows.
const NAME: usize = 255;
/// How long past its own time the helper gets to start in the container, end its search and
/// answer, before the Docker CLI is ended, which closes its stdin and stops it.
const GRACE: Duration = Duration::from_secs(15);
/// The bytes of an answer, well above the 256 KiB of results the helper stops at.
const ANSWER: usize = 1_048_576;

/// A search of a desktop's files. It is also the request the helper reads, as one line of JSON.
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchFiles {
    /// Canonical absolute path of the folder to search with everything below it, or of one
    /// regular file. No symlink in it is followed.
    pub path: String,
    /// What to look for in file contents: a regular expression in Python's syntax, matched within
    /// each line, or plain text with literal=true. Binary files are skipped.
    #[serde(default)]
    pub pattern: Option<String>,
    /// Take pattern as plain text instead of a regular expression.
    #[serde(default)]
    pub literal: bool,
    /// A glob for names, with *, ? and [abc], matched against the whole name of an entry, never
    /// its folders: *.py or config.*. Alone it finds files, folders and symlinks by name; with
    /// pattern it limits which files are read.
    #[serde(default)]
    pub name: Option<String>,
    /// Whether upper and lower case differ, for pattern and name alike (default true).
    #[serde(default = "yes")]
    pub case_sensitive: bool,
    /// The most matches, or names, to return (1 to 1000, default 100).
    #[serde(default = "max_results")]
    pub max_results: usize,
    /// Lines to return before and after each match (0 to 10, default 0).
    #[serde(default)]
    pub context: u8,
    /// Also enter the folders left out by default: .git, .hg, .svn, node_modules, target,
    /// __pycache__, .cache, .venv and the kernel's filesystems such as /proc and /sys.
    #[serde(default)]
    pub include_ignored: bool,
    /// The longest the search runs, in seconds (1 to 30, default 10). It then ends with what it
    /// found and truncated_by "time".
    #[serde(default = "max_seconds")]
    pub max_seconds: u64,
}
fn yes() -> bool {
    true
}
fn max_results() -> usize {
    100
}
fn max_seconds() -> u64 {
    10
}
impl SearchFiles {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.path != "/" {
            super::split_path(&self.path)?;
        }
        if self.pattern.is_none() && self.name.is_none() {
            bail!("Give pattern, name or both");
        }
        if let Some(pattern) = &self.pattern
            && (pattern.is_empty()
                || pattern.chars().count() > PATTERN
                || pattern.contains(['\n', '\r', '\0']))
        {
            bail!(
                "pattern must be 1 to {PATTERN} characters on one line; lines are searched one at a time"
            );
        }
        if let Some(name) = &self.name
            && (name.is_empty()
                || name.chars().count() > NAME
                || name.contains(['/', '\0', '\n', '\r']))
        {
            bail!(
                "name must be a glob of 1 to {NAME} characters for a name alone, without a slash"
            );
        }
        if self.max_results == 0 || self.max_results > MAX_RESULTS {
            bail!("max_results must be 1 to {MAX_RESULTS}");
        }
        if self.context > MAX_CONTEXT {
            bail!("context must be 0 to {MAX_CONTEXT}");
        }
        if self.max_seconds == 0 || self.max_seconds > MAX_SECONDS {
            bail!("max_seconds must be 1 to {MAX_SECONDS}");
        }
        Ok(())
    }
}

/// Search the files of the running container `id` as the desktop user. The request goes to the
/// helper on stdin, never on a command line, and stdin stays open until the answer is read, so a
/// search nobody waits for any more is stopped in the container too.
pub async fn search_files(app: &App, id: &str, body: &SearchFiles) -> Result<Value> {
    body.validate()?;
    let _guard = app.share(&format!("container:{id}")).await?;
    let container = running_container(app, id).await?;
    let args = [
        "exec",
        "--interactive",
        "--user",
        "1001:1001",
        container["Id"].as_str().context("Missing container ID")?,
        "python3",
        "-c",
        HELPER,
    ]
    .map(str::to_owned);
    let mut request = serde_json::to_vec(body).map_err(anyhow::Error::from)?;
    request.push(b'\n');
    let output = process::capture_held(
        app.command(&args),
        Duration::from_secs(body.max_seconds) + GRACE,
        ANSWER,
        process::Overflow::Reject,
        request,
    )
    .await?;
    if !output.status.success() {
        return Err(Error::helper(&output));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| anyhow::anyhow!(e).into())
}
pub async fn search(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<SearchFiles>,
) -> Result<Json<Value>> {
    Ok(Json(search_files(&app, &id, &body).await?))
}
#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
