//! Pins down the activity table: pruning keeps the newest calls and never gives an ID again,
//! `ACTIVITY_KEEP` decides how many stay, as rows arrive and when the manager starts, each filter
//! narrows a page, pages follow each other by their cursor without a gap, a filter value no row
//! could have is refused, and the calls of a set of containers are counted with their repeats.
use super::*;
use serde_json::json;

/// A row of its own under the default `ACTIVITY_KEEP`.
fn insert(db: &Db, entry: &Entry) -> anyhow::Result<i64> {
    super::insert(db, entry, KEPT)
}

fn db() -> Db {
    Db::open(std::path::Path::new(":memory:")).unwrap()
}

fn entry(tool: &str) -> Entry {
    Entry {
        at: 1_760_000_000_000,
        credential: "mcp_token".into(),
        source: "tool".into(),
        tool: tool.into(),
        kind: "read".into(),
        arguments: json!({}),
        outcome: "ok".into(),
        ..Default::default()
    }
}

fn tools(entries: &[Entry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.tool.as_str()).collect()
}

fn page(db: &Db, filter: Filter) -> Vec<Entry> {
    filter.check().unwrap();
    query(db, &filter).unwrap()
}

#[test]
fn pruning_keeps_the_newest_calls_and_never_gives_an_id_again() {
    let db = db();
    for n in 0..30 {
        insert(&db, &entry(&format!("t{n}"))).unwrap();
    }
    assert_eq!(prune(&db.connection(), 10).unwrap(), 20);
    let kept = page(&db, Filter::default());
    assert_eq!(kept.len(), 10);
    assert_eq!(kept.first().unwrap().tool, "t29");
    assert_eq!(kept.last().unwrap().tool, "t20");
    // Nothing more to remove, and a new call follows the last ID even after the old ones went.
    assert_eq!(prune(&db.connection(), 10).unwrap(), 0);
    assert_eq!(insert(&db, &entry("t30")).unwrap(), 31);
}

/// `count` rows as an older manager left them, written at once.
fn filled(db: &Db, count: usize) {
    db.connection()
        .execute_batch(&format!(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {count})
             INSERT INTO activity(at, credential, source, tool, kind, arguments, summary, outcome, duration_ms)
             SELECT i, 'none', 'tool', 'old', 'read', '{{}}', '', 'ok', 0 FROM n"
        ))
        .unwrap();
}

fn rows(db: &Db) -> usize {
    db.connection()
        .query_row("SELECT count(*) FROM activity", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap() as usize
}

#[test]
fn a_full_table_is_pruned_to_the_setting_as_calls_arrive() {
    for keep in [KEPT, 150] {
        let db = db();
        filled(&db, keep + 10);
        assert_eq!(rows(&db), keep + 10);
        // The next ID that is a multiple of PRUNE_EVERY prunes.
        for _ in 0..PRUNE_EVERY {
            super::insert(&db, &entry("new"), keep).unwrap();
        }
        let left = rows(&db);
        assert!(
            (keep..keep + PRUNE_EVERY as usize).contains(&left),
            "{left} rows are left of {keep} to keep"
        );
        let newest = page(&db, Filter::default());
        assert_eq!(newest[0].tool, "new");
    }
}

#[test]
fn a_smaller_setting_drops_the_older_rows_when_the_manager_starts() {
    let db = db();
    filled(&db, 400);
    trim(&db, 1000).unwrap();
    assert_eq!(rows(&db), 400);
    trim(&db, 100).unwrap();
    assert_eq!(rows(&db), 100);
    // The newest stay.
    let oldest: i64 = db
        .connection()
        .query_row("SELECT min(id) FROM activity", [], |row| row.get(0))
        .unwrap();
    assert_eq!(oldest, 301);
}

#[test]
fn the_setting_is_a_whole_number_within_its_bounds() {
    assert_eq!(keep(None).unwrap(), 10_000);
    // Compose passes an empty value for a line `.env` leaves out.
    assert_eq!(keep(Some("")).unwrap(), KEPT);
    assert_eq!(keep(Some("100")).unwrap(), 100);
    assert_eq!(keep(Some("1000000")).unwrap(), 1_000_000);
    for refused in ["99", "1000001", "0", "-5", "many", "1e4", " 500"] {
        let error = keep(Some(refused)).unwrap_err();
        assert!(
            format!("{error:#}")
                .contains("ACTIVITY_KEEP must be a whole number from 100 to 1000000"),
            "{refused}: {error:#}"
        );
    }
}

#[test]
fn the_calls_of_containers_are_counted_with_their_repeats_errors_and_last_call() {
    let db = db();
    let row = |container: &str, at: u64, outcome: &str| {
        let mut row = entry("get_job");
        row.container_id = Some(container.into());
        row.at = at;
        row.outcome = outcome.into();
        insert(&db, &row).unwrap()
    };
    let polled = row("c1", 1000, "ok");
    row("c1", 2000, "error");
    row("c1", 3000, "refused");
    row("c2", 500, "ok");
    row("other", 9000, "error");
    // A read that repeated twice more, the last at 7000.
    assert!(repeat(&db, polled, 6000, 5).unwrap());
    assert!(repeat(&db, polled, 7000, 50).unwrap());
    assert!(repeat(&db, polled, 6500, 9).unwrap());
    let mut tallies = by_container(&db, &["c1", "c2", "never"]).unwrap();
    tallies.sort_by(|a, b| a.container_id.cmp(&b.container_id));
    assert_eq!(
        tallies,
        [
            Tally {
                container_id: "c1".into(),
                calls: 6,
                errors: 2,
                last_at: 7000
            },
            Tally {
                container_id: "c2".into(),
                calls: 1,
                errors: 0,
                last_at: 500
            },
        ]
    );
    assert!(by_container(&db, &[]).unwrap().is_empty());
    // The row keeps its first call's time, the last one's and the longest duration.
    let first = page(&db, Filter::default())
        .into_iter()
        .find(|row| row.id == polled)
        .unwrap();
    assert_eq!(
        (first.at, first.last_at, first.repeats, first.duration_ms),
        (1000, Some(7000), 3, 50)
    );
    // A row that was pruned takes no repeat.
    assert!(!repeat(&db, 999, 1, 1).unwrap());
}

#[test]
fn every_filter_narrows_the_page() {
    let db = db();
    let rows = [
        ("screenshot", None, Some("c1"), None, "read", "ok"),
        (
            "exec_container",
            None,
            Some("c2"),
            None,
            "destructive",
            "error",
        ),
        (
            "browser_navigate",
            Some("chrome-dev"),
            Some("c1"),
            Some("playwright"),
            "destructive",
            "ok",
        ),
        (
            "get_container",
            Some("chrome-dev"),
            Some("c1"),
            None,
            "read",
            "refused",
        ),
        (
            "write_file",
            Some("other"),
            Some("c2"),
            None,
            "destructive",
            "ok",
        ),
    ];
    for (tool, endpoint, container, server, kind, outcome) in rows {
        let mut row = entry(tool);
        row.endpoint = endpoint.map(Into::into);
        row.container_id = container.map(Into::into);
        row.server = server.map(Into::into);
        row.kind = kind.into();
        row.outcome = outcome.into();
        insert(&db, &row).unwrap();
    }
    let filter = |f: fn(&mut Filter)| {
        let mut filter = Filter::default();
        f(&mut filter);
        tools(&page(&db, filter))
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        filter(|f| f.container = Some("c1".into())),
        ["get_container", "browser_navigate", "screenshot"]
    );
    assert_eq!(
        filter(|f| f.endpoint = Some("chrome-dev".into())),
        ["get_container", "browser_navigate"]
    );
    assert_eq!(
        filter(|f| f.caller = Some("administrator".into())),
        ["exec_container", "screenshot"]
    );
    assert_eq!(
        filter(|f| f.caller = Some("endpoint".into())),
        ["write_file", "get_container", "browser_navigate"]
    );
    // A tool's name in any case, or its server's.
    assert_eq!(filter(|f| f.tool = Some("EXEC".into())), ["exec_container"]);
    assert_eq!(
        filter(|f| f.tool = Some("playwright".into())),
        ["browser_navigate"]
    );
    assert_eq!(
        filter(|f| f.kind = Some("destructive".into())),
        ["write_file", "browser_navigate", "exec_container"]
    );
    assert_eq!(
        filter(|f| f.outcome = Some("error,refused".into())),
        ["get_container", "exec_container"]
    );
    assert_eq!(
        filter(|f| {
            f.container = Some("c2".into());
            f.kind = Some("destructive".into());
            f.outcome = Some("ok".into());
        }),
        ["write_file"]
    );
}

#[test]
fn pages_follow_each_other_by_their_cursor() {
    let db = db();
    for n in 0..7 {
        insert(&db, &entry(&format!("t{n}"))).unwrap();
    }
    let mut seen = Vec::new();
    let mut before = None;
    loop {
        let page = page(
            &db,
            Filter {
                before,
                limit: Some(3),
                ..Default::default()
            },
        );
        seen.extend(page.iter().map(|entry| entry.tool.clone()));
        if page.len() < 3 {
            break;
        }
        before = page.last().map(|entry| entry.id);
    }
    assert_eq!(seen, ["t6", "t5", "t4", "t3", "t2", "t1", "t0"]);
}

#[test]
fn a_filter_no_row_could_match_is_refused() {
    for filter in [
        Filter {
            kind: Some("delete".into()),
            ..Default::default()
        },
        Filter {
            outcome: Some("ok,failed".into()),
            ..Default::default()
        },
        Filter {
            caller: Some("root".into()),
            ..Default::default()
        },
        Filter {
            limit: Some(0),
            ..Default::default()
        },
        Filter {
            limit: Some(PAGE_LIMIT + 1),
            ..Default::default()
        },
    ] {
        assert!(filter.check().is_err(), "{filter:?}");
    }
}

#[test]
fn a_row_reads_back_as_it_was_written() {
    let db = db();
    let mut written = entry("write_file");
    written.endpoint = Some("chrome-dev".into());
    written.client = Some("claude-code 2.1.0".into());
    written.container_id = Some("c1".into());
    written.container_name = Some("Chrome dev".into());
    written.arguments = json!({"path": "/tmp/x", "data": {"$masked": "string", "length": 3}});
    written.summary = "/tmp/x".into();
    written.outcome = "error".into();
    written.error = Some("No such folder".into());
    written.duration_ms = 42;
    written.job_id = Some("j1".into());
    written.id = insert(&db, &written).unwrap();
    assert_eq!(page(&db, Filter::default()), [written]);
}
