//! Pins down job logs in chunks: a log reads exactly as the whole-record log trimmed by
//! `Job::trim_log` after every addition read, with the log an older manager left in the record
//! before the chunks, while the chunks kept stay close to what the log keeps; a record and the
//! line saved with it arrive together; a log starts again empty of what it had but ends further on
//! than it did; and a job that is pruned, removed or gone takes its chunks with it.
use super::*;
use std::path::Path;

fn job(id: &str, log: &str) -> Job {
    Job {
        id: id.into(),
        kind: "exec".into(),
        resource: None,
        status: "running".into(),
        created_at: 1,
        finished_at: None,
        log: log.into(),
        result: None,
        progress: None,
        stages: Vec::new(),
        script: None,
        name: None,
    }
}

fn db() -> Db {
    Db::open(Path::new(":memory:")).unwrap()
}

/// The bytes the job's chunks hold.
fn stored(db: &Db, id: &str) -> usize {
    db.connection()
        .query_row(
            "SELECT coalesce(sum(length(CAST(text AS BLOB))), 0) FROM job_log WHERE job=?1",
            [id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap() as usize
}

fn chunks(db: &Db) -> i64 {
    db.connection()
        .query_row("SELECT count(*) FROM job_log", [], |row| row.get(0))
        .unwrap()
}

/// Text of `len` characters or so, of one, two, three and four bytes, so that cuts fall inside
/// characters.
fn text(seed: &mut u64, len: usize) -> String {
    const PIECES: [&str; 6] = ["a", "\n", "é", "日", "😀", "line "];
    let mut out = String::new();
    while out.len() < len {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        out.push_str(PIECES[(*seed % PIECES.len() as u64) as usize]);
    }
    out
}

#[test]
fn a_log_reads_as_the_record_trimmed_after_every_addition_kept_it() {
    for (case, inline) in [
        "",
        "an older manager wrote this\n",
        "and this without its end",
    ]
    .into_iter()
    .enumerate()
    {
        let db = db();
        let id = format!("job-{case}");
        db.put("job", &id, &job(&id, inline)).unwrap();
        // How the whole record kept its log.
        let mut expected = job(&id, inline);
        let mut seed = 0x9e37_79b9_7f4a_7c15 + case as u64;
        let mut largest = 0;
        for round in 0..400 {
            let size = match round % 50 {
                // Now and then a piece longer than the whole log keeps.
                49 => LOG_LIMIT + 3_001,
                n => 1 + (n * 211) % 9_000,
            };
            let piece = text(&mut seed, size);
            largest = largest.max(piece.len().min(LOG_LIMIT + 3));
            db.append_job_log(&id, &piece).unwrap();
            expected.log.push_str(&piece);
            expected.trim_log();
            if round % 37 == 0 || round % 50 == 49 {
                assert_eq!(db.job(&id).unwrap().log, expected.log, "round {round}");
            }
            assert!(stored(&db, &id) < LOG_LIMIT + largest, "round {round}");
        }
        assert_eq!(db.job(&id).unwrap().log, expected.log);
        // The record itself never changed.
        let record: Job = db.get("job", &id).unwrap();
        assert_eq!(record.log, inline);
    }
}

#[test]
fn a_short_log_is_whole_and_in_order() {
    let db = db();
    db.put("job", "j", &job("j", "")).unwrap();
    for line in ["first\n", "second\n", "third"] {
        db.append_job_log("j", line).unwrap();
    }
    db.append_job_log("j", "").unwrap();
    assert_eq!(db.job("j").unwrap().log, "first\nsecond\nthird");
    assert_eq!(chunks(&db), 3);
}

#[test]
fn a_job_that_is_not_there_takes_no_log() {
    let db = db();
    let error = db.append_job_log("missing", "text\n").unwrap_err();
    assert!(matches!(
        error.downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::QueryReturnedNoRows)
    ));
    assert!(db.restart_job_log("missing", "text\n").is_err());
    assert!(db.job("missing").is_err());
    assert_eq!(chunks(&db), 0);
}

#[test]
fn a_record_and_its_line_arrive_together_on_a_line_of_their_own() {
    let db = db();
    // A line the record's own log left open is ended first.
    let mut saved = job("j", "half a li");
    saved.status = "failed".into();
    db.put_job_logged(&saved, "Stage 1/2: Building\n", true)
        .unwrap();
    let read = db.job("j").unwrap();
    assert_eq!(read.status, "failed");
    assert_eq!(read.log, "half a li\nStage 1/2: Building\n");
    // After a whole line, nothing is added; without own_line, nothing either.
    db.put_job_logged(&saved, "Stage 2/2: Saving\n", true)
        .unwrap();
    db.append_job_log("j", "partial").unwrap();
    db.put_job_logged(&saved, "\nERROR: it broke\n", false)
        .unwrap();
    assert_eq!(
        db.job("j").unwrap().log,
        "half a li\nStage 1/2: Building\nStage 2/2: Saving\npartial\nERROR: it broke\n"
    );
    // An empty log takes no line end before its first line.
    db.put_job_logged(&job("k", ""), "Stage 1/1: Running\n", true)
        .unwrap();
    assert_eq!(db.job("k").unwrap().log, "Stage 1/1: Running\n");
}

#[test]
fn a_log_started_again_keeps_nothing_it_had() {
    let db = db();
    db.put("job", "j", &job("j", "from the record\n")).unwrap();
    db.append_job_log("j", "from a chunk\n").unwrap();
    let (_, ended) = db.job_and_end("j").unwrap();
    assert_eq!(
        ended,
        ("from the record\n".len() + "from a chunk\n".len()) as u64
    );
    db.restart_job_log("j", "Read again from its start.\n")
        .unwrap();
    db.append_job_log("j", "next\n").unwrap();
    let (read, end) = db.job_and_end("j").unwrap();
    assert_eq!(read.log, "Read again from its start.\nnext\n");
    assert_eq!(read.status, "running");
    // The positions go on from where the log ended.
    assert_eq!(end, ended + read.log.len() as u64);

    // Started again with nothing, it ends where it did, and its first line needs no line end.
    db.restart_job_log("j", "").unwrap();
    let (read, again) = db.job_and_end("j").unwrap();
    assert_eq!((read.log.as_str(), again), ("", end));
    let record: Job = db.get("job", "j").unwrap();
    db.put_job_logged(&record, "Stage 1/1: Running\n", true)
        .unwrap();
    let (read, after) = db.job_and_end("j").unwrap();
    assert_eq!(read.log, "Stage 1/1: Running\n");
    assert_eq!(after, end + read.log.len() as u64);
}

#[test]
fn a_job_pruned_removed_or_gone_takes_its_chunks() {
    let dir = std::env::temp_dir().join(format!("thoric-log-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(crate::db::FILE);
    let db = Db::open(&path).unwrap();
    for id in ["old", "new", "removed"] {
        let mut record = job(id, "");
        record.finished_at = Some(2);
        db.put("job", id, &record).unwrap();
        db.append_job_log(id, &format!("{id}\n")).unwrap();
    }
    db.remove("job", "removed").unwrap();
    let keep = crate::jobs::history::Kept {
        scripts: 1,
        others: 1,
    };
    assert_eq!(db.prune_jobs(keep).unwrap(), 1);
    let left: Vec<String> = {
        let conn = db.connection();
        let mut statement = conn.prepare("SELECT DISTINCT job FROM job_log").unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(left, ["new"]);
    // Chunks whose job went another way, such as an older manager pruning it, go when the
    // database opens.
    db.connection()
        .execute("DELETE FROM records WHERE kind='job'", [])
        .unwrap();
    drop(db);
    let db = Db::open(&path).unwrap();
    assert_eq!(chunks(&db), 0);
    drop(db);
    std::fs::remove_dir_all(&dir).unwrap();
}
