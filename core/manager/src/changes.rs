//! What changed in a container's own files (`GET /api/containers/{id}/changes`, `get_changes`):
//! Docker's change list of the writable layer of the container's Docker container, relative to
//! the image that Docker container was created from. A reset, a retry that recreates it, a
//! settings change that recreates it from a commit of its files and a restore of a backup with its
//! files each begin the list again, the last two with the earlier changes inside that commit, so
//! the answer names the image and when the Docker container was created. Named volumes and other mounts are not in the list, and the
//! answer names them; a snapshot is not a point Docker keeps a list from. What the desktop
//! changes on its own is left out unless asked for (`changes/noise.rs`), and the files under the
//! desktop home of a running desktop get their type, size and time from `file_ops.py`.
mod noise;
mod source;

use crate::{
    App,
    error::{Error, Result},
    process,
};
use anyhow::{Context, bail};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use noise::within;
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use source::{Change, Kind};
use std::{collections::HashMap, sync::Arc, time::Duration};

/// The most entries an answer lists.
pub const LIMIT: usize = 5000;
/// The folders the summary counts, the largest first.
const FOLDER_LIMIT: usize = 100;
/// The entries under the desktop home whose type, size and time one answer reads.
const DETAILS: usize = 1000;
/// The desktop user's home, whose folders the summary counts apart.
const HOME: &str = "/home/desktop";
/// The change lists read at once. Docker walks the container's files for each, and the answer
/// holds the whole list until it is counted, so more wait their turn.
static READING: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeQuery {
    /// Only what is at or below this canonical absolute path, such as /home/desktop/project.
    #[serde(default)]
    pub path: Option<String>,
    /// The kinds to list, separated by commas: added, changed and deleted. All three when left out.
    #[serde(default)]
    pub kinds: Option<String>,
    /// Also list what the desktop changes on its own, each entry with the noise rule it matches.
    #[serde(default)]
    pub all: bool,
    /// The most entries to list (1 to 5000).
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 5000))]
    pub limit: usize,
}
fn default_limit() -> usize {
    500
}

/// What a query asks for, checked.
#[derive(Debug, PartialEq)]
struct Selection {
    /// The folder the answer is limited to, `None` for the whole container.
    path: Option<String>,
    kinds: Vec<Kind>,
    all: bool,
    limit: usize,
}
impl ChangeQuery {
    fn selection(&self) -> anyhow::Result<Selection> {
        let path = match self.path.as_deref() {
            None | Some("" | "/") => None,
            Some(path) => {
                crate::apps::directory(path)
                    .context("path must be a canonical absolute path, such as /home/desktop")?;
                Some(path.to_owned())
            }
        };
        let kinds = match self.kinds.as_deref().map(str::trim) {
            None | Some("") => Kind::ALL.to_vec(),
            Some(kinds) => {
                let mut chosen = Vec::new();
                for name in kinds.split(',').map(str::trim) {
                    let Some(kind) = Kind::ALL.into_iter().find(|kind| kind.id() == name) else {
                        bail!("kinds takes added, changed and deleted, separated by commas");
                    };
                    if !chosen.contains(&kind) {
                        chosen.push(kind);
                    }
                }
                chosen
            }
        };
        if self.limit == 0 || self.limit > LIMIT {
            bail!("limit must be 1 to {LIMIT}");
        }
        Ok(Selection {
            path,
            kinds,
            all: self.all,
            limit: self.limit,
        })
    }
}
impl Selection {
    fn covers(&self, change: &Change) -> bool {
        self.kinds.contains(&change.kind)
            && self
                .path
                .as_deref()
                .is_none_or(|folder| within(&change.path, folder))
    }
}

/// An entry of the answer.
#[derive(Debug, Serialize)]
struct Listed {
    path: String,
    kind: Kind,
    /// The noise rule it matches, listed only with `all`.
    #[serde(skip_serializing_if = "Option::is_none")]
    noise: Option<&'static str>,
    /// `file`, `directory`, `symlink` or `special`, read in a running desktop.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    file_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modified_at: Option<i64>,
}

/// What an answer lists of the entries, with its counts.
#[derive(Debug)]
struct Listing {
    changes: Vec<Listed>,
    /// The entries the selection lists, of which `changes` holds the first.
    total: usize,
    summary: Value,
    noise: Value,
}

/// The folder an entry is counted in: the one right below `base` that holds it or that it is,
/// where the whole container counts the desktop home's folders apart. `base` itself is in none.
fn folder_of(path: &str, base: Option<&str>) -> Option<String> {
    let base = match base {
        Some(base) => base,
        None if path.len() > HOME.len() && within(path, HOME) => HOME,
        None => "",
    };
    let name = path
        .get(base.len()..)?
        .strip_prefix('/')?
        .split('/')
        .next()?;
    (!name.is_empty()).then(|| format!("{base}/{name}"))
}

/// The selection's part of `entries`, in [`noise::sort`]'s order with the rule each matches.
fn listing(
    entries: Vec<Change>,
    rules: Vec<Option<&'static str>>,
    selection: &Selection,
) -> Listing {
    let mut matched: HashMap<&'static str, usize> = HashMap::new();
    let mut kinds: HashMap<Kind, usize> = HashMap::new();
    let mut folders: HashMap<String, HashMap<Kind, usize>> = HashMap::new();
    let mut changes = Vec::new();
    let mut total = 0;
    for (entry, rule) in entries.into_iter().zip(rules) {
        if !selection.covers(&entry) {
            continue;
        }
        if let Some(rule) = rule {
            *matched.entry(rule).or_default() += 1;
            if !selection.all {
                continue;
            }
        }
        total += 1;
        *kinds.entry(entry.kind).or_default() += 1;
        if let Some(folder) = folder_of(&entry.path, selection.path.as_deref()) {
            *folders
                .entry(folder)
                .or_default()
                .entry(entry.kind)
                .or_default() += 1;
        }
        if changes.len() < selection.limit {
            changes.push(Listed {
                path: entry.path,
                kind: entry.kind,
                noise: rule,
                file_type: None,
                size: None,
                modified_at: None,
            });
        }
    }
    let counted = |counts: &HashMap<Kind, usize>| {
        let mut value = json!({"total": counts.values().sum::<usize>()});
        for kind in Kind::ALL {
            value[kind.id()] = json!(counts.get(&kind).copied().unwrap_or(0));
        }
        value
    };
    let mut folders: Vec<(String, HashMap<Kind, usize>)> = folders.into_iter().collect();
    folders.sort_by(|(a, a_counts), (b, b_counts)| {
        let size = |counts: &HashMap<Kind, usize>| counts.values().sum::<usize>();
        size(b_counts).cmp(&size(a_counts)).then_with(|| a.cmp(b))
    });
    let mut summary = counted(&kinds);
    summary["folders_truncated"] = json!(folders.len() > FOLDER_LIMIT);
    summary["folders"] = folders
        .iter()
        .take(FOLDER_LIMIT)
        .map(|(path, counts)| {
            let mut folder = counted(counts);
            folder["path"] = json!(path);
            folder
        })
        .collect();
    let rule = |id: &'static str, description: &'static str, paths: Option<Vec<String>>| {
        let count = matched.get(id).copied().unwrap_or(0);
        let mut rule = json!({"id": id, "description": description, "count": count});
        if let Some(paths) = paths.filter(|_| count > 0) {
            rule["paths"] = json!(paths);
        }
        rule
    };
    let mut rules: Vec<Value> = noise::RULES
        .iter()
        .chain([&noise::TOOLKIT])
        .map(|noise| {
            let paths = noise
                .paths
                .iter()
                .map(|(path, reach)| match reach {
                    noise::Reach::Exact => (*path).to_owned(),
                    noise::Reach::Tree => format!("{path}/**"),
                    noise::Reach::Prefix => format!("{path}*"),
                })
                .collect();
            rule(noise.id, noise.description, Some(paths))
        })
        .collect();
    rules.push(rule(noise::MOUNTS.0, noise::MOUNTS.1, None));
    rules.push(rule(noise::FOLDERS.0, noise::FOLDERS.1, None));
    Listing {
        changes,
        total,
        summary,
        noise: json!({
            "left_out": !selection.all,
            "count": matched.values().sum::<usize>(),
            "rules": rules,
        }),
    }
}

/// What is mounted in the Docker container `info` describes, whose files the list leaves out: its
/// volumes and host folders, without a host folder's own path, and its folders in memory.
fn mounts(info: &Value) -> Vec<Value> {
    let mut mounts: Vec<Value> = info["Mounts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|mount| {
            let mut entry = json!({
                "path": mount["Destination"].as_str()?,
                "type": mount["Type"].as_str().unwrap_or("bind"),
                "read_only": mount["RW"] == false,
            });
            if mount["Type"] == "volume"
                && let Some(name) = mount["Name"].as_str()
            {
                entry["volume"] = json!(name);
            }
            Some(entry)
        })
        .collect();
    if let Some(tmpfs) = info["HostConfig"]["Tmpfs"].as_object() {
        mounts.extend(tmpfs.iter().map(|(path, options)| {
            let read_only = options
                .as_str()
                .is_some_and(|options| options.split(',').any(|option| option == "ro"));
            json!({"path": path, "type": "tmpfs", "read_only": read_only})
        }));
    }
    mounts.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    mounts
}

/// What a listed path is, as `file_ops.py stats` describes it.
#[derive(Deserialize)]
struct Detail {
    #[serde(rename = "type")]
    file_type: String,
    size: u64,
    modified_at: i64,
}

/// Read the type, size and time of the listed entries under the desktop home that are still there,
/// the first [`DETAILS`] of them, as the desktop user without following symlinks. Says how far it
/// got; a failure leaves the entries as they are, since the list stands without them.
async fn read_details(app: &App, docker_id: &str, changes: &mut [Listed]) -> &'static str {
    let wanted: Vec<usize> = changes
        .iter()
        .enumerate()
        .filter(|(_, change)| {
            change.kind != Kind::Deleted
                && change.path.len() > HOME.len()
                && within(&change.path, HOME)
        })
        .map(|(index, _)| index)
        .collect();
    let read = &wanted[..wanted.len().min(DETAILS)];
    if read.is_empty() {
        return "read";
    }
    let paths: Vec<&str> = read
        .iter()
        .map(|&index| changes[index].path.as_str())
        .collect();
    let described = match stats(app, docker_id, &paths).await {
        Ok(described) if described.len() == read.len() => described,
        Ok(_) => {
            tracing::debug!("file_ops.py described another number of paths than it was given");
            return "failed";
        }
        Err(error) => {
            tracing::debug!("Details of changed files: {error:#}");
            return "failed";
        }
    };
    for (&index, detail) in read.iter().zip(described) {
        if let Some(detail) = detail {
            let change = &mut changes[index];
            change.size = (detail.file_type == "file").then_some(detail.size);
            change.file_type = Some(detail.file_type);
            change.modified_at = Some(detail.modified_at);
        }
    }
    if wanted.len() > read.len() {
        "partial"
    } else {
        "read"
    }
}

async fn stats(app: &App, docker_id: &str, paths: &[&str]) -> anyhow::Result<Vec<Option<Detail>>> {
    let args = [
        "exec",
        "--interactive",
        "--user",
        "1001:1001",
        docker_id,
        "python3",
        "-c",
        crate::files::FILE_OPS,
        "stats",
    ]
    .map(str::to_owned);
    let output = process::capture_input(
        app.command(&args),
        Duration::from_secs(30),
        1_048_576,
        process::Overflow::Reject,
        Some(serde_json::to_vec(paths)?),
    )
    .await?;
    if !output.status.success() {
        bail!(
            "file_ops.py: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout).context("file_ops.py answered unreadably")
}

/// The change list of container `id` as `query` asks for it.
pub async fn changes(app: &App, id: &str, query: &ChangeQuery) -> Result<Value> {
    let selection = query.selection()?;
    let _reading = READING
        .acquire()
        .await
        .map_err(|error| anyhow::anyhow!(error))?;
    let _guard = app.share(&format!("container:{id}")).await?;
    let env = app.container(id)?;
    let Some(info) = app.optional_inspect("container", &env.container).await? else {
        return Err(Error(
            StatusCode::CONFLICT,
            format!(
                "{} has no Docker container, so Docker keeps no change list for it; a retry creates one from its image",
                env.spec.name
            ),
        ));
    };
    if info["Config"]["Labels"][crate::model::LABEL] != "true"
        || info["Config"]["Labels"]["thoric.environment"] != env.id
    {
        return Err(anyhow::anyhow!("Container ownership does not match").into());
    }
    let docker_id = info["Id"].as_str().context("Missing container ID")?;
    let mut entries = app.change_list(docker_id).await?;
    noise::sort(&mut entries);
    let mounts = mounts(&info);
    let desktop = noise::Desktop {
        mounts: mounts
            .iter()
            .filter_map(|mount| mount["path"].as_str().map(str::to_owned))
            .collect(),
        toolkit: info["HostConfig"]["DeviceRequests"]
            .as_array()
            .is_some_and(|requests| !requests.is_empty()),
    };
    let rules = noise::classify(&entries, &desktop);
    let mut listing = listing(entries, rules, &selection);
    let running = info["State"]["Running"] == true;
    let details = if running {
        read_details(app, docker_id, &mut listing.changes).await
    } else {
        "stopped"
    };
    let image_id = info["Image"].as_str().unwrap_or_default();
    let created_at = info["Created"]
        .as_str()
        .and_then(crate::usage::stats::unix_nanos)
        .and_then(|nanos| i64::try_from(nanos.div_euclid(1_000_000_000)).ok());
    Ok(json!({
        "container_id": env.id,
        "running": running,
        "base": {
            "image": info["Config"]["Image"],
            "image_id": image_id,
            // A settings change or a restore created it from this commit of its earlier files.
            "commit": env.working_image.as_ref().is_some_and(|image| image.image_id == image_id),
            "created_at": created_at,
        },
        "mounts": mounts,
        "path": selection.path.as_deref().unwrap_or("/"),
        "summary": listing.summary,
        "noise": listing.noise,
        "total": listing.total,
        "returned": listing.changes.len(),
        "truncated": listing.total > listing.changes.len(),
        "details": details,
        "changes": listing.changes,
    }))
}

pub async fn list(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Query(query): Query<ChangeQuery>,
) -> Result<Json<Value>> {
    Ok(Json(changes(&app, &id, &query).await?))
}

#[cfg(test)]
#[path = "changes_tests.rs"]
mod tests;
