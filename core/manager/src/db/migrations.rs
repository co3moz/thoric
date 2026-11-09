//! The database's schema versions. The database keeps the number of migrations it has run in
//! `PRAGMA user_version`, and every start runs the ones it lacks, all in one transaction, before
//! anything reads it. A database from before versions has 0 and the tables of version 1.
//!
//! A migration a release shipped never changes, since databases out there already ran it; a
//! change of the schema is the next migration. A database of a later version than this
//! manager knows is refused, since the release that migrated it may have moved what this one
//! reads.
use anyhow::bail;
use rusqlite::{Connection, Transaction, TransactionBehavior};

type Migration = fn(&Transaction) -> rusqlite::Result<()>;

/// Migration `n` brings a database of version `n - 1` to version `n`.
const MIGRATIONS: &[Migration] = &[records_and_activity, job_log, activity_repeats];

/// Bring the database to the latest version this manager knows.
pub(super) fn run(conn: &mut Connection) -> anyhow::Result<()> {
    run_these(conn, MIGRATIONS)
}

fn run_these(conn: &mut Connection, migrations: &[Migration]) -> anyhow::Result<()> {
    let latest = migrations.len();
    // Immediate, so a second manager on the same file waits instead of migrating it as well.
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: i64 = transaction.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let Some(done) = usize::try_from(version)
        .ok()
        .filter(|&version| version <= latest)
    else {
        bail!(
            "A newer release of Thoric wrote this database: it has schema version {version}, and this release knows versions up to {latest}. Run that release again, or put back the copy of the database the dashboard's update made before it moved to it (docs/INSTALLATION.md, \"Go back to an earlier release\")"
        );
    };
    if done == latest {
        return Ok(());
    }
    for (index, migration) in migrations.iter().enumerate().skip(done) {
        migration(&transaction)
            .map_err(|error| anyhow::anyhow!("Database migration {} failed: {error}", index + 1))?;
    }
    transaction.pragma_update(None, "user_version", latest as i64)?;
    transaction.commit()?;
    tracing::info!("Database schema moved from version {done} to {latest}");
    Ok(())
}

/// Version 1: the tables before versions were kept, the records by kind and id and the agent
/// activity, which a database from before versions already has.
fn records_and_activity(transaction: &Transaction) -> rusqlite::Result<()> {
    // AUTOINCREMENT: an activity ID is never given again after older rows are pruned, so a page's
    // cursor keeps its place.
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS records(kind TEXT NOT NULL, id TEXT NOT NULL, data TEXT NOT NULL, PRIMARY KEY(kind,id));
        CREATE TABLE IF NOT EXISTS activity(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at INTEGER NOT NULL,
            endpoint TEXT,
            credential TEXT NOT NULL,
            client TEXT,
            source TEXT NOT NULL,
            tool TEXT NOT NULL,
            server TEXT,
            container_id TEXT,
            container_name TEXT,
            kind TEXT NOT NULL,
            arguments TEXT NOT NULL,
            summary TEXT NOT NULL,
            outcome TEXT NOT NULL,
            error TEXT,
            duration_ms INTEGER NOT NULL,
            job_id TEXT
        );
        CREATE INDEX IF NOT EXISTS activity_container ON activity(container_id, id);
        CREATE INDEX IF NOT EXISTS activity_endpoint ON activity(endpoint, id);",
    )
}

/// Version 2: job logs apart from their records, in the chunks they were written in; see
/// `db/job_log.rs`. `ends_at` is the length in bytes of the job's log up to the end of the
/// chunk, which orders a job's chunks and finds those to drop. A log a record already carries
/// stays in it.
fn job_log(transaction: &Transaction) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "CREATE TABLE job_log(
            job TEXT NOT NULL,
            text TEXT NOT NULL,
            ends_at INTEGER NOT NULL
        );
        CREATE INDEX job_log_job ON job_log(job, ends_at);",
    )
}

/// Version 3: a repeated read joins the row of the call before it instead of adding one; see
/// `activity/merge.rs`. `repeats` counts the calls a row stands for beyond its first, and
/// `last_at` is when the last of them began, empty for a row of one call.
fn activity_repeats(transaction: &Transaction) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "ALTER TABLE activity ADD COLUMN repeats INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE activity ADD COLUMN last_at INTEGER;",
    )
}

#[cfg(test)]
#[path = "migrations_tests.rs"]
mod tests;
