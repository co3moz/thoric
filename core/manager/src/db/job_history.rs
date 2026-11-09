//! What stays of the jobs that ended: the newest records of each class, with their logs and the
//! requests kept to retry them, a filtered list of them, the ids pruned since the manager started,
//! and how many jobs ended in each hour, which outlives the records.
//!
//! Script jobs come by the thousand while an agent works, so they are a class of their own: a
//! day of scripts prunes scripts, and a build from last week stays among the other jobs.
use super::{Db, job_log::JOB};
use crate::jobs::history::{Class, Count, Kept, SCRIPT_KINDS};
use anyhow::Result;
use rusqlite::{params, params_from_iter};
use std::collections::{HashSet, VecDeque};

/// How many pruned job ids are remembered, the newest; an id is 32 characters.
const REMEMBERED: usize = 4096;

/// The ids of the jobs pruned since the manager started, the newest [`REMEMBERED`].
#[derive(Default)]
pub(super) struct Pruned {
    order: VecDeque<String>,
    ids: HashSet<String>,
}

impl Pruned {
    fn remember(&mut self, id: String) {
        if self.ids.insert(id.clone()) {
            self.order.push_back(id);
        }
        while self.order.len() > REMEMBERED {
            if let Some(oldest) = self.order.pop_front() {
                self.ids.remove(&oldest);
            }
        }
    }
}

/// Which jobs a list names.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Filter {
    /// Only these kinds, when any is named.
    pub kinds: Vec<String>,
    /// Only the script jobs, or only the others.
    pub class: Option<Class>,
}

/// The SQL that is true for a job record of a script kind. The kinds are names in the code.
fn is_script() -> String {
    let kinds: Vec<String> = SCRIPT_KINDS
        .iter()
        .map(|kind| format!("'{kind}'"))
        .collect();
    format!("json_extract(data, '$.kind') IN ({})", kinds.join(", "))
}

impl Db {
    /// Remove the finished jobs beyond the newest of their class, the script jobs and the others
    /// each counted alone, with their logs and the requests kept to retry them, and return how
    /// many went. A job still running or queued stays whatever its age; its end is what marks it
    /// finished. The ids that went are remembered, so a read can say the job was pruned.
    pub fn prune_jobs(&self, keep: Kept) -> Result<usize> {
        let script = is_script();
        let mut removed = Vec::new();
        {
            let mut conn = self.connection();
            let transaction = conn.transaction()?;
            for (class, keep) in [(Class::Script, keep.scripts), (Class::Other, keep.others)] {
                let of_class = match class {
                    Class::Script => script.clone(),
                    Class::Other => format!("NOT ({script})"),
                };
                let pruned = format!(
                    "kind='job' AND {of_class} AND json_extract(data, '$.finished_at') IS NOT NULL AND rowid NOT IN (SELECT rowid FROM records WHERE kind='job' AND {of_class} ORDER BY rowid DESC LIMIT ?1)"
                );
                // The logs first, while the records still say whose they are.
                transaction.execute(
                    &format!(
                        "DELETE FROM job_log WHERE job IN (SELECT id FROM records WHERE {pruned})"
                    ),
                    params![keep as i64],
                )?;
                let mut statement = transaction
                    .prepare(&format!("DELETE FROM records WHERE {pruned} RETURNING id"))?;
                let ids =
                    statement.query_map(params![keep as i64], |row| row.get::<_, String>(0))?;
                for id in ids {
                    removed.push(id?);
                }
            }
            // A request kept to start a job again goes with its job.
            transaction.execute(
                "DELETE FROM records WHERE kind=?1 AND id NOT IN (SELECT id FROM records WHERE kind='job')",
                params![crate::jobs::retry::REQUESTS],
            )?;
            transaction.commit()?;
        }
        let count = removed.len();
        let mut pruned = self.pruned.lock().unwrap();
        for id in removed {
            pruned.remember(id);
        }
        Ok(count)
    }

    /// Whether the job `id` was pruned since the manager started, among the newest ids kept.
    pub fn was_pruned(&self, id: &str) -> bool {
        self.pruned.lock().unwrap().ids.contains(id)
    }

    /// The newest 100 jobs `filter` names, without their logs or script reports, which are
    /// stripped in SQLite before a row is read.
    pub fn jobs_listed(&self, filter: &Filter) -> Result<Vec<crate::model::Job>> {
        let mut conditions = vec!["kind='job'".to_string()];
        match filter.class {
            Some(Class::Script) => conditions.push(is_script()),
            Some(Class::Other) => conditions.push(format!("NOT ({})", is_script())),
            None => {}
        }
        if !filter.kinds.is_empty() {
            let places: Vec<String> = (1..=filter.kinds.len()).map(|n| format!("?{n}")).collect();
            conditions.push(format!(
                "json_extract(data, '$.kind') IN ({})",
                places.join(", ")
            ));
        }
        let query = format!(
            "SELECT id, json_remove(json_set(data, '$.log', ''), '$.script') FROM records WHERE {} ORDER BY rowid DESC LIMIT 100",
            conditions.join(" AND ")
        );
        let rows: Vec<(String, String)> = {
            let conn = self.connection();
            let mut statement = conn.prepare(&query)?;
            let rows = statement.query_map(params_from_iter(filter.kinds.iter()), |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        Ok(rows
            .into_iter()
            .filter_map(|(id, data)| self.readable(JOB, &id, &data))
            .collect())
    }

    /// Count a job of `class` that ended at `at` with `status`, in the hour it ended in, and drop
    /// the hours more than `kept_hours` before that one. A status that is neither completed nor
    /// failed counts as interrupted.
    pub fn count_job_end(
        &self,
        class: Class,
        status: &str,
        at: u64,
        kept_hours: u64,
    ) -> Result<()> {
        let hour = (at / 3600) as i64;
        let column = match status {
            "completed" => "completed",
            "failed" => "failed",
            _ => "interrupted",
        };
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        transaction.execute(
            &format!(
                "INSERT INTO job_counts(hour, class, {column}) VALUES(?1, ?2, 1) ON CONFLICT(hour, class) DO UPDATE SET {column} = {column} + 1"
            ),
            params![hour, class.name()],
        )?;
        transaction.execute(
            "DELETE FROM job_counts WHERE hour <= ?1",
            params![hour - kept_hours as i64],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// The counts of the hours from `since`, a time in seconds, oldest first.
    pub fn job_counts(&self, since: u64) -> Result<Vec<Count>> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT hour, class, completed, failed, interrupted FROM job_counts WHERE hour >= ?1 ORDER BY hour, class",
        )?;
        let rows = statement.query_map(params![(since / 3600) as i64], |row| {
            let hour: i64 = row.get(0)?;
            let class: String = row.get(1)?;
            let count = |index| -> rusqlite::Result<u64> {
                Ok(u64::try_from(row.get::<_, i64>(index)?).unwrap_or_default())
            };
            Ok(Count {
                at: u64::try_from(hour).unwrap_or_default() * 3600,
                class,
                completed: count(2)?,
                failed: count(3)?,
                interrupted: count(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
#[path = "job_history_tests.rs"]
mod tests;
