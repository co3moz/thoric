//! The manager's SQLite: a table of JSON records by kind and id, the agent activity, the jobs'
//! logs ([`job_log`]) and what the numbered [`migrations`] bring, which every start runs before
//! anything reads.
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use std::{collections::HashSet, path::Path, sync::Mutex};

mod job_log;
mod migrations;

/// The database's file in the data folder.
pub(crate) const FILE: &str = "state.sqlite";

pub struct Db {
    conn: Mutex<Connection>,
    /// The records a list skipped because they did not read, by kind and id, each said once.
    unreadable: Mutex<HashSet<(String, String)>>,
}
impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        // In WAL mode NORMAL syncs at checkpoints rather than at every commit, which a job's log
        // makes often; a power loss can lose the last commits, never the database.
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;",
        )?;
        migrations::run(&mut conn)
            .with_context(|| format!("Could not open the database {}", path.display()))?;
        job_log::opened(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            unreadable: Mutex::new(HashSet::new()),
        })
    }
    /// The connection itself, for a module with a table of its own, such as the agent activity.
    pub(crate) fn connection(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }
    pub fn put<T: Serialize>(&self, kind: &str, id: &str, value: &T) -> Result<()> {
        self.connection().execute("INSERT INTO records(kind,id,data) VALUES(?1,?2,?3) ON CONFLICT(kind,id) DO UPDATE SET data=excluded.data", params![kind,id,serde_json::to_string(value)?])?;
        Ok(())
    }
    pub fn get<T: DeserializeOwned>(&self, kind: &str, id: &str) -> Result<T> {
        let text: String = self.connection().query_row(
            "SELECT data FROM records WHERE kind=?1 AND id=?2",
            params![kind, id],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_str(&text)?)
    }
    /// Every record of `kind` that reads, newest first. One that does not, such as a record a
    /// later release wrote in another shape, is left out and logged, so the others stay usable;
    /// a decision that needs them all takes [`Db::list_complete`].
    pub fn list<T: DeserializeOwned>(&self, kind: &str) -> Result<Vec<T>> {
        Ok(self
            .rows(kind, "rowid DESC")?
            .into_iter()
            .filter_map(|(id, data)| self.readable(kind, &id, &data))
            .collect())
    }
    /// The records of `kind` whose id starts with `prefix`, such as one container's in ids of the
    /// form `container:name`, as [`Db::list`] reads them: newest first, and without one that does
    /// not read. Only those rows are read, as a range of the primary key, so `prefix` is matched
    /// as it is: `%` and `_` mean themselves.
    pub fn list_prefixed<T: DeserializeOwned>(&self, kind: &str, prefix: &str) -> Result<Vec<T>> {
        let rows: Vec<(String, String)> = {
            let conn = self.connection();
            let row = |r: &rusqlite::Row| Ok((r.get(0)?, r.get(1)?));
            match past(prefix) {
                Some(end) => conn
                    .prepare("SELECT id, data FROM records WHERE kind=?1 AND id>=?2 AND id<?3 ORDER BY rowid DESC")?
                    .query_map(params![kind, prefix, end], row)?
                    .collect::<rusqlite::Result<_>>()?,
                None => conn
                    .prepare("SELECT id, data FROM records WHERE kind=?1 AND id>=?2 ORDER BY rowid DESC")?
                    .query_map(params![kind, prefix], row)?
                    .collect::<rusqlite::Result<_>>()?,
            }
        };
        Ok(rows
            .into_iter()
            .filter_map(|(id, data)| self.readable(kind, &id, &data))
            .collect())
    }
    /// Every record of `kind`, newest first, or an error that names the first that does not
    /// read: for a decision that must see them all, such as whether anything still uses an
    /// image or a volume, or a backup.
    pub fn list_complete<T: DeserializeOwned>(&self, kind: &str) -> Result<Vec<T>> {
        self.rows(kind, "rowid DESC")?
            .into_iter()
            .map(|(id, data)| complete(kind, &id, &data))
            .collect()
    }
    /// Every record of `kind` with its id, oldest first, for records that do not carry their
    /// id, or an error that names the first that does not read, as [`Db::list_complete`] gives.
    /// Only a backup reads such records all at once.
    pub fn entries_complete<T: DeserializeOwned>(&self, kind: &str) -> Result<Vec<(String, T)>> {
        self.rows(kind, "rowid")?
            .into_iter()
            .map(|(id, data)| Ok((id.clone(), complete(kind, &id, &data)?)))
            .collect()
    }
    /// The ids of every record of `kind`, without reading one, so a record that does not read
    /// still counts as taken.
    pub fn ids(&self, kind: &str) -> Result<Vec<String>> {
        let conn = self.connection();
        let mut stmt = conn.prepare("SELECT id FROM records WHERE kind=?1 ORDER BY rowid")?;
        let rows = stmt.query_map([kind], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
    fn rows(&self, kind: &str, order: &str) -> Result<Vec<(String, String)>> {
        let conn = self.connection();
        let mut stmt = conn.prepare(&format!(
            "SELECT id, data FROM records WHERE kind=?1 ORDER BY {order}"
        ))?;
        let rows = stmt.query_map([kind], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
    /// The record when it reads; otherwise its kind and id are logged, once, and never its data.
    fn readable<T: DeserializeOwned>(&self, kind: &str, id: &str, data: &str) -> Option<T> {
        match serde_json::from_str(data) {
            Ok(value) => Some(value),
            Err(error) => {
                if self
                    .unreadable
                    .lock()
                    .unwrap()
                    .insert((kind.into(), id.into()))
                {
                    tracing::warn!("Skipping the {kind} record {id}: {}", unread(&error));
                }
                None
            }
        }
    }
    /// Insert records that must arrive together, as `(kind, id, value)`, in one transaction: all
    /// of them or none. An existing record is never replaced; one of the same kind and id fails it.
    pub fn insert_all(&self, records: &[(&str, &str, serde_json::Value)]) -> Result<()> {
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        for (kind, id, value) in records {
            transaction.execute(
                "INSERT INTO records(kind,id,data) VALUES(?1,?2,?3)",
                params![kind, id, serde_json::to_string(value)?],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
    /// Remove a record; a job's log goes with it.
    pub fn remove(&self, kind: &str, id: &str) -> Result<()> {
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        if kind == job_log::JOB {
            transaction.execute("DELETE FROM job_log WHERE job=?1", [id])?;
        }
        transaction.execute(
            "DELETE FROM records WHERE kind=?1 AND id=?2",
            params![kind, id],
        )?;
        transaction.commit()?;
        Ok(())
    }
    /// Remove every record of `kind` whose id starts with `prefix`, the range
    /// [`Db::list_prefixed`] reads, also those that do not read, and return how many went. Not
    /// for jobs, which [`Db::remove`] takes one at a time with their logs.
    pub fn remove_prefixed(&self, kind: &str, prefix: &str) -> Result<usize> {
        anyhow::ensure!(kind != job_log::JOB, "A job is removed with its log");
        let conn = self.connection();
        Ok(match past(prefix) {
            Some(end) => conn.execute(
                "DELETE FROM records WHERE kind=?1 AND id>=?2 AND id<?3",
                params![kind, prefix, end],
            )?,
            None => conn.execute(
                "DELETE FROM records WHERE kind=?1 AND id>=?2",
                params![kind, prefix],
            )?,
        })
    }
    /// Remove the finished jobs beyond the newest `keep`, with their logs. A job still running
    /// or queued stays whatever its age; its end is what marks it finished.
    pub fn prune_jobs(&self, keep: usize) -> Result<usize> {
        const PRUNED: &str = "kind='job' AND json_extract(data, '$.finished_at') IS NOT NULL AND rowid NOT IN (SELECT rowid FROM records WHERE kind='job' ORDER BY rowid DESC LIMIT ?1)";
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        // The logs first, while the records still say whose they are.
        transaction.execute(
            &format!("DELETE FROM job_log WHERE job IN (SELECT id FROM records WHERE {PRUNED})"),
            params![keep as i64],
        )?;
        let removed = transaction.execute(
            &format!("DELETE FROM records WHERE {PRUNED}"),
            params![keep as i64],
        )?;
        // A request kept to start a job again goes with its job.
        transaction.execute(
            "DELETE FROM records WHERE kind=?1 AND id NOT IN (SELECT id FROM records WHERE kind='job')",
            params![crate::jobs::retry::REQUESTS],
        )?;
        transaction.commit()?;
        Ok(removed)
    }
    /// Remove a record and return it, or `None` when there is none: of two callers that take the
    /// same record, only one gets it.
    pub fn take<T: DeserializeOwned>(&self, kind: &str, id: &str) -> Result<Option<T>> {
        let text: Option<String> = self
            .connection()
            .query_row(
                "DELETE FROM records WHERE kind=?1 AND id=?2 RETURNING data",
                params![kind, id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(text.map(|text| serde_json::from_str(&text)).transpose()?)
    }
    /// Whether a record of `kind` is saved under `id`, without reading it.
    pub fn contains(&self, kind: &str, id: &str) -> Result<bool> {
        let found = self.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM records WHERE kind=?1 AND id=?2)",
            params![kind, id],
            |row| row.get(0),
        )?;
        Ok(found)
    }
    pub fn jobs_summary(&self) -> Result<Vec<crate::model::Job>> {
        // Strip potentially large logs and script reports in SQLite, before allocating/deserializing rows.
        self.jobs("SELECT id, json_remove(json_set(data, '$.log', ''), '$.script') FROM records WHERE kind='job' ORDER BY rowid DESC LIMIT 100")
    }
    /// The jobs that are queued or running, newest first, without their logs or script reports.
    pub fn active_jobs(&self) -> Result<Vec<crate::model::Job>> {
        self.jobs("SELECT id, json_remove(json_set(data, '$.log', ''), '$.script') FROM records WHERE kind='job' AND json_extract(data, '$.status') IN ('queued', 'running') ORDER BY rowid DESC")
    }
    /// The jobs `query` selects as id and data, without those that do not read.
    fn jobs(&self, query: &str) -> Result<Vec<crate::model::Job>> {
        let rows: Vec<(String, String)> = {
            let conn = self.connection();
            let mut stmt = conn.prepare(query)?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        Ok(rows
            .into_iter()
            .filter_map(|(id, data)| self.readable("job", &id, &data))
            .collect())
    }
    /// Write a consistent copy of the whole database to `path`, a file that does not exist yet,
    /// while the manager goes on using it.
    pub fn copy_to(&self, path: &Path) -> Result<()> {
        let target = path
            .to_str()
            .with_context(|| format!("{} is not a UTF-8 path", path.display()))?;
        self.connection().execute("VACUUM INTO ?1", [target])?;
        Ok(())
    }
}

/// A record a decision needs whole, or an error that names it without its data.
fn complete<T: DeserializeOwned>(kind: &str, id: &str, data: &str) -> Result<T> {
    serde_json::from_str(data).map_err(|error| {
        anyhow::anyhow!(
            "The {kind} record {id} cannot be read by this release of Thoric ({})",
            unread(&error)
        )
    })
}

/// Why a record did not read, by where it went wrong: serde's own message can quote a value.
fn unread(error: &serde_json::Error) -> String {
    let what = match error.classify() {
        serde_json::error::Category::Data => "fields in a shape this release does not read",
        serde_json::error::Category::Syntax | serde_json::error::Category::Eof => "invalid JSON",
        serde_json::error::Category::Io => "a read error",
    };
    format!("{what} at line {}, column {}", error.line(), error.column())
}

/// The first text past every id that starts with `prefix`, or `None` when no text is. SQLite
/// compares text byte by byte, which for UTF-8 is the order of the characters, and no character's
/// bytes begin another's, so raising the last character that can be raised bounds the prefix.
fn past(prefix: &str) -> Option<String> {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        if let Some(next) = (u32::from(last) + 1..=u32::from(char::MAX)).find_map(char::from_u32) {
            chars.push(next);
            return Some(chars.into_iter().collect());
        }
    }
    None
}

#[cfg(test)]
#[path = "db_tests.rs"]
mod tests;
