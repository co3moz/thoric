//! Pins down the schema versions: a new database and one from before versions both end at the
//! latest version with what they held, a job's log in its record included, a database a newer
//! release migrated is refused and left as it was, a migration that fails changes nothing, and the
//! job counts start from the ended jobs the database still has.
use super::*;
use crate::db::Db;
use std::path::{Path, PathBuf};

fn folder() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("thoric-schema-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn version(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap()
}

/// A database as releases up to 0.8.4 left it: their tables, without a version.
fn before_versions(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; CREATE TABLE IF NOT EXISTS records(kind TEXT NOT NULL, id TEXT NOT NULL, data TEXT NOT NULL, PRIMARY KEY(kind,id));
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
        CREATE INDEX IF NOT EXISTS activity_endpoint ON activity(endpoint, id);
        INSERT INTO records(kind, id, data) VALUES('volume-note', 'v', '{\"name\":\"kept\"}');
        INSERT INTO records(kind, id, data) VALUES('job', 'old', '{\"id\":\"old\",\"kind\":\"exec\",\"resource\":null,\"status\":\"running\",\"created_at\":1,\"finished_at\":null,\"log\":\"Written by 0.8.4\\nhalf a li\",\"result\":null}');
        INSERT INTO activity(at, credential, source, tool, kind, arguments, summary, outcome, duration_ms)
            VALUES(1, 'mcp_token', 'tool', 'list_containers', 'read', '{}', '', 'ok', 3);",
    )
    .unwrap();
    assert_eq!(version(&conn), 0);
}

#[test]
fn a_new_database_starts_at_the_latest_version() {
    let db = Db::open(Path::new(":memory:")).unwrap();
    let conn = db.connection();
    assert_eq!(version(&conn), MIGRATIONS.len() as i64);
    let tables: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('records', 'activity', 'job_log', 'job_counts')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tables, 4);
}

#[test]
fn a_database_from_before_versions_keeps_what_it_held() {
    let dir = folder();
    let path = dir.join(crate::db::FILE);
    before_versions(&path);
    for round in 0..2 {
        let db = Db::open(&path).unwrap();
        assert_eq!(version(&db.connection()), MIGRATIONS.len() as i64);
        let note: serde_json::Value = db.get("volume-note", "v").unwrap();
        assert_eq!(note["name"], "kept");
        let calls: i64 = db
            .connection()
            .query_row("SELECT count(*) FROM activity", [], |row| row.get(0))
            .unwrap();
        assert_eq!(calls, 1);
        // A job whose log is in its record reads as it did, and what is added follows it.
        if round == 0 {
            assert_eq!(db.job("old").unwrap().log, "Written by 0.8.4\nhalf a li");
            db.append_job_log("old", "ne\nand more\n").unwrap();
        }
        assert_eq!(
            db.job("old").unwrap().log,
            "Written by 0.8.4\nhalf a line\nand more\n"
        );
        assert!(db.jobs_summary().unwrap()[0].log.is_empty());
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_job_counts_start_from_the_ended_jobs_a_database_still_has() {
    let mut conn = Connection::open_in_memory().unwrap();
    run_these(&mut conn, &MIGRATIONS[..2]).unwrap();
    let job = |id: &str, kind: &str, status: &str, finished: Option<u64>| {
        let data = serde_json::json!({"id": id, "kind": kind, "resource": null, "status": status, "created_at": 1, "finished_at": finished, "log": "", "result": null});
        conn.execute(
            "INSERT INTO records(kind, id, data) VALUES('job', ?1, ?2)",
            [id, &data.to_string()],
        )
        .unwrap();
    };
    job("a", "exec", "completed", Some(7200));
    job("b", "exec", "completed", Some(7300));
    job("c", "exec", "failed", Some(10_799));
    job("d", "build", "interrupted", Some(7200));
    job("e", "build", "completed", Some(10_800));
    job("running", "exec", "running", None);
    // A record that does not read and one that is not a job's are not counted and fail nothing.
    conn.execute_batch(
        "INSERT INTO records(kind, id, data) VALUES('job', 'broken', 'not json');
        INSERT INTO records(kind, id, data) VALUES('image', 'i', '{\"kind\":\"exec\",\"status\":\"completed\",\"finished_at\":7200}');",
    )
    .unwrap();
    run_these(&mut conn, MIGRATIONS).unwrap();
    let mut statement = conn
        .prepare(
            "SELECT hour, class, completed, failed, interrupted FROM job_counts ORDER BY hour, class",
        )
        .unwrap();
    let counts: Vec<(i64, String, i64, i64, i64)> = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        counts,
        [
            (2, "other".to_string(), 0, 0, 1),
            (2, "script".to_string(), 2, 1, 0),
            (3, "other".to_string(), 1, 0, 0),
        ]
    );
}

#[test]
fn a_database_a_newer_release_migrated_is_refused_and_left_as_it_was() {
    let dir = folder();
    let path = dir.join(crate::db::FILE);
    before_versions(&path);
    let newer = MIGRATIONS.len() as i64 + 1;
    Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", newer)
        .unwrap();
    let error = format!("{:#}", Db::open(&path).err().unwrap());
    assert!(
        error.contains(&format!("schema version {newer}"))
            && error.contains(&format!("versions up to {}", MIGRATIONS.len()))
            && error.contains("Go back to an earlier release"),
        "{error}"
    );
    assert_eq!(version(&Connection::open(&path).unwrap()), newer);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_migration_that_fails_changes_nothing() {
    let mut conn = Connection::open_in_memory().unwrap();
    let error = run_these(
        &mut conn,
        &[records_and_activity, |transaction| {
            transaction.execute_batch("CREATE TABLE half(x); SELECT * FROM nowhere;")
        }],
    )
    .unwrap_err();
    assert!(error.to_string().contains("migration 2 failed"), "{error}");
    assert_eq!(version(&conn), 0);
    let tables: i64 = conn
        .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))
        .unwrap();
    assert_eq!(tables, 0);
}
