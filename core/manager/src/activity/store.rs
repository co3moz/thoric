//! The activity table in the manager's SQLite: one row a call, or a run of the same read (see
//! `merge.rs`), the newest `ACTIVITY_KEEP` kept, the filtered, newest-first pages
//! `GET /api/activity` reads and the calls counted by container.
use super::Entry;
use crate::{
    db::Db,
    error::{Error, Result},
};
use anyhow::{Context, bail};
use axum::http::StatusCode;
use rusqlite::{Connection, Row, params, params_from_iter, types::Value as Sql};
use serde::Deserialize;

/// How many rows are kept unless `ACTIVITY_KEEP` says otherwise, and how few and how many it may
/// ask for: enough to say what happened, and no more than a page's count and a set's summary
/// read in a moment.
pub(crate) const KEPT: usize = 10_000;
const KEPT_MIN: usize = 100;
const KEPT_MAX: usize = 1_000_000;
/// Rows beyond the kept ones are removed every this many rows, not on every one.
const PRUNE_EVERY: i64 = 64;
/// The calls a page holds by default, and at most.
const PAGE: usize = 50;
pub(super) const PAGE_LIMIT: usize = 200;

/// `ACTIVITY_KEEP`: how many rows of the log are kept, the newest.
pub(crate) fn keep(value: Option<&str>) -> anyhow::Result<usize> {
    let refused = || format!("ACTIVITY_KEEP must be a whole number from {KEPT_MIN} to {KEPT_MAX}");
    let keep = match value.filter(|text| !text.is_empty()) {
        None => KEPT,
        Some(text) => text.parse::<usize>().with_context(refused)?,
    };
    if !(KEPT_MIN..=KEPT_MAX).contains(&keep) {
        bail!(refused());
    }
    Ok(keep)
}

/// Drop what is beyond `keep` when the manager starts, so a smaller `ACTIVITY_KEEP` applies at
/// once. The table's AUTOINCREMENT never gives an ID again after older rows are pruned, so a
/// page's cursor keeps its place.
pub(crate) fn trim(db: &Db, keep: usize) -> rusqlite::Result<()> {
    prune(&db.connection(), keep)?;
    Ok(())
}

/// Write a call as a row of its own, and every [`PRUNE_EVERY`] rows drop what is beyond `keep`.
pub(super) fn insert(db: &Db, entry: &Entry, keep: usize) -> anyhow::Result<i64> {
    let conn = db.connection();
    conn.execute(
        "INSERT INTO activity(at, endpoint, credential, client, source, tool, server, container_id,
            container_name, kind, arguments, summary, outcome, error, duration_ms, job_id, repeats,
            last_at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            signed(entry.at),
            entry.endpoint,
            entry.credential,
            entry.client,
            entry.source,
            entry.tool,
            entry.server,
            entry.container_id,
            entry.container_name,
            entry.kind,
            entry.arguments.to_string(),
            entry.summary,
            entry.outcome,
            entry.error,
            signed(entry.duration_ms),
            entry.job_id,
            signed(entry.repeats),
            entry.last_at.map(signed),
        ],
    )?;
    let id = conn.last_insert_rowid();
    if id % PRUNE_EVERY == 0 {
        prune(&conn, keep)?;
    }
    Ok(id)
}

/// Count one more call on the row `id`: it began at `at` and took `duration_ms`, and the row
/// keeps the longest of its calls. `false` when the row is gone, pruned since.
pub(super) fn repeat(db: &Db, id: i64, at: u64, duration_ms: u64) -> anyhow::Result<bool> {
    let changed = db.connection().execute(
        "UPDATE activity SET repeats = repeats + 1, last_at = max(coalesce(last_at, at), ?2),
            duration_ms = max(duration_ms, ?3)
         WHERE id = ?1",
        params![id, signed(at), signed(duration_ms)],
    )?;
    Ok(changed == 1)
}

fn signed(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn unsigned(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

/// Remove every row but the newest `keep`.
pub(super) fn prune(conn: &Connection, keep: usize) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM activity WHERE id <= (SELECT id FROM activity ORDER BY id DESC LIMIT 1 OFFSET ?1)",
        params![i64::try_from(keep).unwrap_or(i64::MAX)],
    )
}

/// The calls of one container among the kept rows.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Tally {
    pub container_id: String,
    /// Every call, a repeated read's each.
    pub calls: u64,
    /// Those that ended in an error or were refused.
    pub errors: u64,
    /// When the last one began, in Unix milliseconds.
    pub last_at: u64,
}

/// The calls of each of `containers` that has some, in one read of their rows through the
/// container index. The list is a scale set's instances, so it is short.
pub(crate) fn by_container(db: &Db, containers: &[&str]) -> anyhow::Result<Vec<Tally>> {
    if containers.is_empty() {
        return Ok(Vec::new());
    }
    let conn = db.connection();
    let mut statement = conn.prepare(&format!(
        "SELECT container_id, sum(1 + repeats), sum(outcome != 'ok'), max(coalesce(last_at, at))
         FROM activity WHERE container_id IN ({}) GROUP BY container_id",
        vec!["?"; containers.len()].join(", ")
    ))?;
    let rows = statement.query_map(params_from_iter(containers), |row| {
        Ok(Tally {
            container_id: row.get(0)?,
            calls: unsigned(row.get(1)?),
            errors: unsigned(row.get(2)?),
            last_at: unsigned(row.get(3)?),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Which calls a page lists: each filter given narrows it, and `before` continues from the last
/// ID of the page before.
#[derive(Clone, Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    /// A container's ID.
    pub container: Option<String>,
    /// An endpoint's name.
    pub endpoint: Option<String>,
    /// `administrator` for `/mcp`, `endpoint` for every endpoint.
    pub caller: Option<String>,
    /// Part of a tool's or a desktop server's name, in any case.
    pub tool: Option<String>,
    /// `read`, `change` or `destructive`, several separated by commas.
    pub kind: Option<String>,
    /// `ok`, `error` or `refused`, several separated by commas.
    pub outcome: Option<String>,
    pub before: Option<i64>,
    pub limit: Option<usize>,
}

const KINDS: [&str; 3] = ["read", "change", "destructive"];
const OUTCOMES: [&str; 3] = ["ok", "error", "refused"];

impl Filter {
    /// Refuse a value no row could have, so a typo does not read as an empty log.
    pub fn check(&self) -> Result<()> {
        let refuse = |message: String| Err(Error(StatusCode::BAD_REQUEST, message));
        if let Some(caller) = &self.caller
            && !["administrator", "endpoint"].contains(&caller.as_str())
        {
            return refuse("caller is administrator or endpoint".into());
        }
        for (name, list, allowed) in [
            ("kind", &self.kind, &KINDS),
            ("outcome", &self.outcome, &OUTCOMES),
        ] {
            if let Some(unknown) = values(list).into_iter().find(|v| !allowed.contains(v)) {
                return refuse(format!(
                    "{name} takes {}, not {unknown}",
                    allowed.join(", ")
                ));
            }
        }
        if self
            .limit
            .is_some_and(|limit| limit == 0 || limit > PAGE_LIMIT)
        {
            return refuse(format!("limit is 1 to {PAGE_LIMIT}"));
        }
        Ok(())
    }
    pub fn limit(&self) -> usize {
        self.limit.unwrap_or(PAGE)
    }
}

/// The comma-separated values of a filter, without empty ones.
fn values(list: &Option<String>) -> Vec<&str> {
    list.as_deref()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect()
}

/// A page of calls, newest first.
pub(super) fn query(db: &Db, filter: &Filter) -> anyhow::Result<Vec<Entry>> {
    let mut clauses: Vec<String> = Vec::new();
    let mut bound: Vec<Sql> = Vec::new();
    for (column, value) in [
        ("container_id", &filter.container),
        ("endpoint", &filter.endpoint),
    ] {
        if let Some(value) = value.as_deref().filter(|value| !value.is_empty()) {
            clauses.push(format!("{column} = ?"));
            bound.push(Sql::Text(value.into()));
        }
    }
    match filter.caller.as_deref() {
        Some("administrator") => clauses.push("endpoint IS NULL".into()),
        Some("endpoint") => clauses.push("endpoint IS NOT NULL".into()),
        _ => {}
    }
    if let Some(tool) = filter
        .tool
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        clauses.push(
            "(instr(lower(tool), lower(?)) > 0 OR instr(lower(coalesce(server, '')), lower(?)) > 0)"
                .into(),
        );
        bound.push(Sql::Text(tool.into()));
        bound.push(Sql::Text(tool.into()));
    }
    for (column, list) in [("kind", &filter.kind), ("outcome", &filter.outcome)] {
        let list = values(list);
        if !list.is_empty() {
            clauses.push(format!(
                "{column} IN ({})",
                vec!["?"; list.len()].join(", ")
            ));
            bound.extend(list.into_iter().map(|value| Sql::Text(value.into())));
        }
    }
    if let Some(before) = filter.before {
        clauses.push("id < ?".into());
        bound.push(Sql::Integer(before));
    }
    let condition = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    bound.push(Sql::Integer(i64::try_from(filter.limit()).unwrap_or(50)));
    let conn = db.connection();
    let mut statement = conn.prepare(&format!(
        "SELECT id, at, endpoint, credential, client, source, tool, server, container_id,
            container_name, kind, arguments, summary, outcome, error, duration_ms, job_id, repeats,
            last_at
         FROM activity {condition} ORDER BY id DESC LIMIT ?"
    ))?;
    let rows = statement.query_map(params_from_iter(bound), entry)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn entry(row: &Row) -> rusqlite::Result<Entry> {
    let arguments: String = row.get(11)?;
    Ok(Entry {
        id: row.get(0)?,
        at: unsigned(row.get(1)?),
        endpoint: row.get(2)?,
        credential: row.get(3)?,
        client: row.get(4)?,
        source: row.get(5)?,
        tool: row.get(6)?,
        server: row.get(7)?,
        container_id: row.get(8)?,
        container_name: row.get(9)?,
        kind: row.get(10)?,
        arguments: serde_json::from_str(&arguments).unwrap_or_default(),
        summary: row.get(12)?,
        outcome: row.get(13)?,
        error: row.get(14)?,
        duration_ms: unsigned(row.get(15)?),
        job_id: row.get(16)?,
        repeats: unsigned(row.get(17)?),
        last_at: row.get::<_, Option<i64>>(18)?.map(unsigned),
    })
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
