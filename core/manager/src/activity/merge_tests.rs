//! Pins down which calls share a row: a read that repeats the call before it by the same caller
//! on the same container with the same arguments, within the window, and nothing else. An
//! error, another argument, another tool, another credential, endpoint, client or container, a
//! change, a call that started a job, a desktop server's tool, a transfer and a read after the
//! window each get a row of their own, and whatever the caller does in between ends the run.
use super::*;
use crate::activity::store::{Filter, KEPT, query};
use serde_json::json;

/// What makes a call another one than the poll it is made of.
type Change = fn(&mut Entry);

fn db() -> Db {
    Db::open(std::path::Path::new(":memory:")).unwrap()
}

/// A poll of the job `job` by the endpoint `set` in the container `c1`, begun at `at`.
fn poll(job: &str, at: u64) -> Entry {
    Entry {
        at,
        endpoint: Some("set".into()),
        credential: "endpoint_token".into(),
        client: Some("opencode 1.0".into()),
        source: "tool".into(),
        tool: "get_job".into(),
        container_id: Some("c1".into()),
        container_name: Some("Worker 1".into()),
        kind: "read".into(),
        arguments: json!({"job_id": job}),
        summary: job.into(),
        outcome: "ok".into(),
        duration_ms: 4,
        ..Default::default()
    }
}

fn all(db: &Db) -> Vec<Entry> {
    let mut rows = query(db, &Filter::default()).unwrap();
    rows.reverse();
    rows
}

#[test]
fn a_poll_of_one_job_is_one_row_that_counts_its_calls() {
    let (db, recent) = (db(), Recent::default());
    let first = recent.record(&db, &poll("j1", 1_000), KEPT).unwrap();
    for n in 1..=1000u64 {
        let mut again = poll("j1", 1_000 + n * 500);
        again.duration_ms = n % 40;
        assert_eq!(recent.record(&db, &again, KEPT).unwrap(), first);
    }
    let rows = all(&db);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!((row.at, row.repeats), (1_000, 1000));
    assert_eq!(row.last_at, Some(501_000));
    // The longest of its calls.
    assert_eq!(row.duration_ms, 39);
    assert_eq!(row.arguments, json!({"job_id": "j1"}));
}

#[test]
fn only_the_same_read_by_the_same_caller_on_the_same_container_joins() {
    let different: [(&str, Change); 11] = [
        ("another job", |e| e.arguments = json!({"job_id": "j2"})),
        ("another argument", |e| {
            e.arguments = json!({"job_id": "j1", "wait_seconds": 30});
        }),
        ("another tool", |e| e.tool = "list_jobs".into()),
        ("another credential", |e| e.credential = "mcp_token".into()),
        ("another endpoint", |e| e.endpoint = Some("other".into())),
        ("the administrator", |e| e.endpoint = None),
        ("another client", |e| {
            e.client = Some("claude-code 2".into())
        }),
        ("another container", |e| e.container_id = Some("c2".into())),
        ("no container", |e| e.container_id = None),
        ("an error", |e| {
            e.outcome = "error".into();
            e.error = Some("No such job".into());
        }),
        ("a refusal", |e| e.outcome = "refused".into()),
    ];
    for (what, change) in different {
        let (db, recent) = (db(), Recent::default());
        let first = recent.record(&db, &poll("j1", 1_000), KEPT).unwrap();
        let mut other = poll("j1", 2_000);
        change(&mut other);
        let second = recent.record(&db, &other, KEPT).unwrap();
        assert_ne!(first, second, "{what} joined the row before it");
        let rows = all(&db);
        assert_eq!(rows.len(), 2, "{what}");
        assert!(rows.iter().all(|row| row.repeats == 0), "{what}");
    }
}

#[test]
fn a_call_that_is_no_plain_read_never_joins_and_never_is_joined() {
    let unlike: [(&str, Change); 5] = [
        ("a change", |e| e.kind = "change".into()),
        ("a destructive call", |e| e.kind = "destructive".into()),
        ("a call that started a job", |e| {
            e.job_id = Some("j9".into())
        }),
        ("a desktop server's tool", |e| {
            e.source = "server".into();
            e.server = Some("playwright".into());
        }),
        ("a transfer", |e| e.source = "transfer".into()),
    ];
    for (what, change) in unlike {
        let (db, recent) = (db(), Recent::default());
        let mut call = poll("j1", 1_000);
        change(&mut call);
        let mut again = poll("j1", 2_000);
        change(&mut again);
        assert_ne!(
            recent.record(&db, &call, KEPT).unwrap(),
            recent.record(&db, &again, KEPT).unwrap(),
            "{what} was repeated on one row"
        );
        assert_eq!(all(&db).len(), 2, "{what}");
    }
}

#[test]
fn an_error_ends_a_run_and_the_next_read_starts_a_row() {
    let (db, recent) = (db(), Recent::default());
    let first = recent.record(&db, &poll("j1", 1_000), KEPT).unwrap();
    assert_eq!(recent.record(&db, &poll("j1", 2_000), KEPT).unwrap(), first);
    let mut failed = poll("j1", 3_000);
    failed.outcome = "error".into();
    let error = recent.record(&db, &failed, KEPT).unwrap();
    let after = recent.record(&db, &poll("j1", 4_000), KEPT).unwrap();
    assert_eq!(recent.record(&db, &poll("j1", 5_000), KEPT).unwrap(), after);
    assert!(first < error && error < after);
    let rows = all(&db);
    assert_eq!(
        rows.iter()
            .map(|row| (row.outcome.as_str(), row.repeats))
            .collect::<Vec<_>>(),
        [("ok", 1), ("error", 0), ("ok", 1)]
    );
}

#[test]
fn what_the_caller_does_in_between_ends_the_run_so_rows_keep_their_order() {
    let (db, recent) = (db(), Recent::default());
    let first = recent.record(&db, &poll("j1", 1_000), KEPT).unwrap();
    // A script in the same container by the same caller.
    let mut script = poll("j1", 2_000);
    script.tool = "exec_container".into();
    script.kind = "destructive".into();
    script.job_id = Some("j2".into());
    recent.record(&db, &script, KEPT).unwrap();
    let second = recent.record(&db, &poll("j1", 3_000), KEPT).unwrap();
    assert_ne!(first, second);
    // Another job polled in between ends it too.
    recent.record(&db, &poll("j2", 4_000), KEPT).unwrap();
    assert_ne!(
        recent.record(&db, &poll("j1", 5_000), KEPT).unwrap(),
        second
    );
    assert_eq!(all(&db).len(), 5);
}

#[test]
fn another_caller_or_container_in_between_leaves_a_run_alone() {
    let (db, recent) = (db(), Recent::default());
    let first = recent.record(&db, &poll("j1", 1_000), KEPT).unwrap();
    // Another agent of the same endpoint polls its own job in another instance, and the
    // administrator reads something.
    let mut elsewhere = poll("j7", 1_500);
    elsewhere.container_id = Some("c2".into());
    let other = recent.record(&db, &elsewhere, KEPT).unwrap();
    let mut administrator = poll("j1", 1_600);
    administrator.endpoint = None;
    administrator.credential = "mcp_token".into();
    administrator.tool = "list_containers".into();
    recent.record(&db, &administrator, KEPT).unwrap();
    assert_eq!(recent.record(&db, &poll("j1", 2_000), KEPT).unwrap(), first);
    elsewhere.at = 2_500;
    assert_eq!(recent.record(&db, &elsewhere, KEPT).unwrap(), other);
    let rows = all(&db);
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows.iter().map(|row| row.repeats).collect::<Vec<_>>(),
        [1, 1, 0]
    );
}

#[test]
fn a_read_after_the_window_starts_a_row_and_the_window_follows_the_last_call() {
    let (db, recent) = (db(), Recent::default());
    let first = recent.record(&db, &poll("j1", 1_000), KEPT).unwrap();
    // Each within the window of the one before, though the last is far from the first.
    let mut at = 1_000;
    for _ in 0..3 {
        at += WINDOW;
        assert_eq!(recent.record(&db, &poll("j1", at), KEPT).unwrap(), first);
    }
    let late = recent
        .record(&db, &poll("j1", at + WINDOW + 1), KEPT)
        .unwrap();
    assert_ne!(late, first);
    let rows = all(&db);
    assert_eq!(
        rows.iter()
            .map(|row| (row.repeats, row.last_at))
            .collect::<Vec<_>>(),
        [(3, Some(at)), (0, None)]
    );
}

#[test]
fn a_run_whose_row_was_pruned_starts_a_row() {
    let (db, recent) = (db(), Recent::default());
    let first = recent.record(&db, &poll("j1", 1_000), KEPT).unwrap();
    store::prune(&db.connection(), 0).unwrap();
    let second = recent.record(&db, &poll("j1", 2_000), KEPT).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        recent.record(&db, &poll("j1", 3_000), KEPT).unwrap(),
        second
    );
    assert_eq!(all(&db).len(), 1);
}

#[test]
fn the_runs_remembered_stay_bounded() {
    let (db, recent) = (db(), Recent::default());
    for n in 0..(RUNS as u64 + 50) {
        let mut call = poll("j1", n);
        call.container_id = Some(format!("c{n}"));
        recent.record(&db, &call, KEPT).unwrap();
    }
    assert!(recent.0.lock().unwrap().len() <= RUNS);
    // Runs too old to be joined are the ones forgotten once the table is full.
    let recent = Recent::default();
    for n in 0..RUNS as u64 {
        let mut call = poll("j1", if n < 10 { 20 * WINDOW } else { n });
        call.container_id = Some(format!("c{n}"));
        recent.record(&db, &call, KEPT).unwrap();
    }
    let mut late = poll("j1", 20 * WINDOW);
    late.container_id = Some("late".into());
    recent.record(&db, &late, KEPT).unwrap();
    assert_eq!(recent.0.lock().unwrap().len(), 11);
}
