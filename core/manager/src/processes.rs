//! The processes of a running desktop, whatever started them, with what each uses, and signals to
//! one of them: `GET /api/containers/{id}/processes`, `POST .../processes/{pid}/kill`, the MCP
//! tools `list_processes` and `kill_process` and the dashboard's process list. The helper reads
//! /proc as the desktop user, who sees every process of the container; a signal goes as the
//! desktop user, who reaches only its own processes, unless root is asked for explicitly.
use crate::{
    App,
    error::{Error, Result},
};
use anyhow::Context;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

const HELPER: &str = include_str!("../helpers/processes.py");
pub const MAX_LIMIT: usize = 1000;
const QUERY_LIMIT: usize = 200;
/// The kernel's highest process ID on 64-bit Linux.
const PID_MAX: u32 = 4_194_304;

/// The order of a process list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    /// The busiest first.
    #[default]
    Cpu,
    /// The most resident memory first.
    Memory,
    /// By process ID.
    Pid,
}
impl Order {
    fn id(self) -> &'static str {
        match self {
            Order::Cpu => "cpu",
            Order::Memory => "memory",
            Order::Pid => "pid",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListProcesses {
    /// cpu (default) lists the busiest first, memory the largest and pid by process ID.
    #[serde(default)]
    pub sort: Order,
    /// At most this many processes, 1 to 1000; total and matched count every one.
    #[serde(default = "limit")]
    pub limit: usize,
    /// Only processes whose name, command line or user contains this text, ignoring case.
    #[serde(default)]
    pub query: Option<String>,
    /// How long CPU use is measured, in milliseconds (100 to 5000).
    #[serde(default = "interval")]
    pub interval_ms: u64,
}
fn limit() -> usize {
    50
}
fn interval() -> u64 {
    500
}
impl ListProcesses {
    pub fn validate(&self) -> anyhow::Result<()> {
        if !(1..=MAX_LIMIT).contains(&self.limit) {
            anyhow::bail!("limit must be 1 to {MAX_LIMIT}");
        }
        if !(100..=5000).contains(&self.interval_ms) {
            anyhow::bail!("interval_ms must be 100 to 5000");
        }
        if let Some(query) = &self.query
            && (query.chars().count() > QUERY_LIMIT || query.chars().any(char::is_control))
        {
            anyhow::bail!(
                "query must be at most {QUERY_LIMIT} characters without control characters"
            );
        }
        Ok(())
    }
}

/// What a process is to the desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Part of the desktop itself, its screen, sound or session, which end with it.
    Desktop,
    /// Started by the manager through `docker exec`: a helper, a script, a terminal or a desktop
    /// MCP server.
    Thoric,
    /// Anything else, started from the desktop, a terminal, a script or an application.
    Program,
}

/// One process as the helper reports it; `core/manager/tests/process-list.json` holds the helper
/// and these types to the same fields.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Process {
    pub pid: u32,
    /// The parent's process ID; 0 for one started through `docker exec`.
    pub ppid: u32,
    pub uid: u32,
    pub user: String,
    /// The kernel's state letter: R running, S sleeping, D waiting on a device, T stopped, Z ended
    /// and not yet reaped by its parent, I idle.
    pub state: String,
    pub name: String,
    /// The command line on one line and at most 512 characters; a multi-line argument keeps its
    /// first line.
    pub command: String,
    pub role: Role,
    pub threads: u32,
    /// Resident memory; pages shared between processes count in each of them.
    pub rss_bytes: u64,
    /// CPU used over the interval, 100 for one full core.
    pub cpu_percent: f64,
    /// CPU time used since the process started.
    pub cpu_seconds: f64,
    /// When it started, in Unix seconds.
    pub started_at: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcessList {
    pub processes: Vec<Process>,
    /// Every process of the container.
    pub total: usize,
    /// Those the query matched, of which `processes` holds at most the limit.
    pub matched: usize,
    /// CPU use of every process, 100 for one full core.
    pub cpu_percent: f64,
    /// Resident memory of every process, shared pages counted in each.
    pub rss_bytes: u64,
    /// The container's memory as its cgroup counts it, page cache included.
    pub memory_bytes: Option<u64>,
    /// The container's memory limit; none without one.
    pub memory_limit_bytes: Option<u64>,
    /// The CPUs the container may use.
    pub cpus: f64,
    /// The interval CPU use was measured over.
    pub interval_ms: u64,
}

/// The signal to send.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SignalName {
    /// SIGTERM, which asks the process to end.
    #[default]
    Term,
    /// SIGKILL, which ends it at once.
    Kill,
    /// SIGINT, as Ctrl+C in a terminal.
    Int,
    /// SIGHUP, a hang-up, which some programs read as an order to reload.
    Hup,
}
impl SignalName {
    fn id(self) -> &'static str {
        match self {
            SignalName::Term => "term",
            SignalName::Kill => "kill",
            SignalName::Int => "int",
            SignalName::Hup => "hup",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Kill {
    /// term (default) asks the process to end, kill ends it at once, int is Ctrl+C and hup a
    /// hang-up.
    #[serde(default)]
    pub signal: SignalName,
    /// Signal the process's descendants too, as they are at the call, parents first.
    #[serde(default)]
    pub tree: bool,
    /// Signal as root, which reaches a process of any user. Without it the signal goes as the
    /// desktop user, UID 1001, which reaches only its own processes.
    #[serde(default)]
    pub root: bool,
    /// How long to wait for the signalled processes to end, in milliseconds (0 to 10000).
    #[serde(default = "wait")]
    pub wait_ms: u64,
}
fn wait() -> u64 {
    2000
}
/// What a request without a body asks for, as the body's own defaults.
impl Default for Kill {
    fn default() -> Self {
        Self {
            signal: SignalName::default(),
            tree: false,
            root: false,
            wait_ms: wait(),
        }
    }
}
impl Kill {
    pub fn validate(&self, pid: u32) -> anyhow::Result<()> {
        if pid == 1 {
            anyhow::bail!(
                "PID 1 starts the desktop, and the container stops with it; stop or restart the container instead"
            );
        }
        if pid == 0 || pid > PID_MAX {
            anyhow::bail!("pid must be 2 to {PID_MAX}");
        }
        if self.wait_ms > 10_000 {
            anyhow::bail!("wait_ms must be at most 10000");
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct Signalled {
    pub pid: u32,
    pub signal: SignalName,
    /// The processes the signal reached: the process and, with tree, its descendants.
    pub signalled: Vec<u32>,
    /// Those that had ended when the call returned.
    pub ended: Vec<u32>,
    /// Those still running once wait_ms had passed.
    pub running: Vec<u32>,
    /// Descendants of another user, which the desktop user could not signal.
    pub refused: Vec<u32>,
}

/// The Docker container of a running desktop.
async fn running(app: &App, id: &str) -> Result<String> {
    let env = app.container(id)?;
    let info = app.owned(&env).await?;
    if info["State"]["Running"] != true {
        return Err(Error(
            StatusCode::CONFLICT,
            "Start the container to work with its processes".into(),
        ));
    }
    Ok(info["Id"]
        .as_str()
        .context("Missing container ID")?
        .to_owned())
}

/// `docker exec` of the helper as `user`, in isolated mode, so nothing in the working directory or
/// the user's site packages is imported.
fn helper_args(container: &str, user: &str, arguments: &[String]) -> Vec<String> {
    let mut args: Vec<String> = [
        "exec", "--user", user, container, "python3", "-I", "-c", HELPER,
    ]
    .map(str::to_owned)
    .to_vec();
    args.extend(arguments.iter().cloned());
    args
}

fn list_arguments(options: &ListProcesses) -> Vec<String> {
    let mut arguments = vec![
        "list".to_owned(),
        options.interval_ms.to_string(),
        options.sort.id().to_owned(),
        options.limit.to_string(),
    ];
    if let Some(query) = options.query.as_deref().filter(|query| !query.is_empty()) {
        arguments.push(query.to_owned());
    }
    arguments
}

fn signal_arguments(pid: u32, options: &Kill) -> Vec<String> {
    vec![
        "signal".to_owned(),
        pid.to_string(),
        options.signal.id().to_owned(),
        if options.tree { "1" } else { "0" }.to_owned(),
        options.wait_ms.to_string(),
    ]
}

pub async fn list_processes(app: &App, id: &str, options: &ListProcesses) -> Result<ProcessList> {
    options.validate()?;
    let container = running(app, id).await?;
    let output = app
        .docker_args(&helper_args(
            &container,
            "1001:1001",
            &list_arguments(options),
        ))
        .await?;
    Ok(serde_json::from_slice(&output).context("The process list was not valid")?)
}

/// What the helper's answer to a signal means.
fn signal_answer(pid: u32, options: &Kill, answer: Value) -> Result<Signalled> {
    match answer["error"].as_str() {
        Some("missing") => Err(Error(
            StatusCode::NOT_FOUND,
            format!("No process {pid} runs in the container"),
        )),
        Some("permission") => Err(Error(
            StatusCode::CONFLICT,
            format!(
                "Process {pid} runs as {}, and the desktop user may not signal it; root=true signals it as root, on the administrator's MCP channel and in the dashboard",
                answer["user"].as_str().unwrap_or("another user")
            ),
        )),
        Some(other) => Err(Error(
            StatusCode::BAD_GATEWAY,
            format!("The process helper failed: {other}"),
        )),
        None => {
            let lists = |key: &str| -> Result<Vec<u32>> {
                Ok(serde_json::from_value(answer[key].clone())
                    .context("The signal's answer was not valid")?)
            };
            Ok(Signalled {
                pid,
                signal: options.signal,
                signalled: lists("signalled")?,
                ended: lists("ended")?,
                running: lists("running")?,
                refused: lists("refused")?,
            })
        }
    }
}

pub async fn kill_process(app: &Arc<App>, id: &str, pid: u32, options: &Kill) -> Result<Signalled> {
    options.validate(pid)?;
    let _guard = app.share(&format!("container:{id}")).await?;
    let container = running(app, id).await?;
    let user = if options.root { "0:0" } else { "1001:1001" };
    let output = app
        .docker_args(&helper_args(
            &container,
            user,
            &signal_arguments(pid, options),
        ))
        .await?;
    let answer: Value =
        serde_json::from_slice(&output).context("The signal's answer was not valid")?;
    signal_answer(pid, options, answer)
}

pub async fn list(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Query(options): Query<ListProcesses>,
) -> Result<Json<ProcessList>> {
    Ok(Json(list_processes(&app, &id, &options).await?))
}

pub async fn kill(
    State(app): State<Arc<App>>,
    Path((id, pid)): Path<(String, u32)>,
    body: Option<Json<Kill>>,
) -> Result<Json<Signalled>> {
    let options = body.map(|Json(body)| body).unwrap_or_default();
    Ok(Json(kill_process(&app, &id, pid, &options).await?))
}

#[cfg(test)]
#[path = "processes_tests.rs"]
mod tests;
