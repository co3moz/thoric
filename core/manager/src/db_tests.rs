//! Pins down that the unfinished jobs are found without reading the logs of the others (pruning
//! is pinned down in `db/job_history_tests.rs`). A record that does not read leaves the others of its kind readable, while the
//! complete lists refuse without its data, and a copy of the database is whole. Also that an image
//! record keeps its snapshot parent, and that the job list is bounded and leaves the logs out.
use super::*;
use crate::model::{self, Job};
use crate::test_support::app;
use serde::Deserialize;

/// A record kind of the tests' own; a record without `name` does not read as one.
#[derive(Debug, Deserialize, PartialEq)]
struct Named {
    name: String,
}

/// The value that keeps the record from reading, which serde's own message would quote.
const SECRET: u64 = 424_242;

fn with_unreadable() -> Db {
    let db = Db::open(std::path::Path::new(":memory:")).unwrap();
    db.put("named", "a", &serde_json::json!({"name": "first"}))
        .unwrap();
    db.put("named", "broken", &serde_json::json!({"name": SECRET}))
        .unwrap();
    db.put("named", "b", &serde_json::json!({"name": "second"}))
        .unwrap();
    db
}

#[test]
fn a_record_that_does_not_read_is_left_out_of_the_lists() {
    let db = with_unreadable();
    let names = |list: Vec<Named>| list.into_iter().map(|n| n.name).collect::<Vec<_>>();
    assert_eq!(names(db.list("named").unwrap()), ["second", "first"]);
    // Said once, however often it is listed.
    db.list::<Named>("named").unwrap();
    assert_eq!(
        *db.unreadable.lock().unwrap(),
        HashSet::from([("named".to_owned(), "broken".to_owned())])
    );
    // Its id still counts as taken.
    assert_eq!(db.ids("named").unwrap(), ["a", "broken", "b"]);
}

#[test]
fn a_complete_list_names_the_record_that_does_not_read_without_its_data() {
    let db = with_unreadable();
    for error in [
        db.list_complete::<Named>("named").unwrap_err(),
        db.entries_complete::<Named>("named").unwrap_err(),
    ] {
        let error = format!("{error:#}");
        assert!(error.contains("named record broken"), "{error}");
        assert!(!error.contains(&SECRET.to_string()), "{error}");
        // What serde itself says of it does quote the value.
        let serde = serde_json::from_value::<Named>(serde_json::json!({"name": SECRET}));
        assert!(serde.unwrap_err().to_string().contains(&SECRET.to_string()));
    }
    db.remove("named", "broken").unwrap();
    assert_eq!(db.list_complete::<Named>("named").unwrap().len(), 2);
    let entries: Vec<(String, Named)> = db.entries_complete("named").unwrap();
    let ids: Vec<&str> = entries.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["a", "b"]);
}

#[test]
fn a_prefixed_list_reads_the_ids_that_begin_with_it_and_nothing_else() {
    let db = Db::open(std::path::Path::new(":memory:")).unwrap();
    for id in [
        "a:one",
        "ab:two",
        "a:two",
        "b:one",
        "a_:x",
        "a%:y",
        "A:z",
        "ç:1",
        "è:2",
        "\u{10FFFF}:3",
        "\u{10FFFF}",
    ] {
        db.put("named", id, &serde_json::json!({"name": id}))
            .unwrap();
    }
    db.put("other", "a:three", &serde_json::json!({"name": "a:three"}))
        .unwrap();
    let named = |prefix: &str| {
        db.list_prefixed::<Named>("named", prefix)
            .unwrap()
            .into_iter()
            .map(|n| n.name)
            .collect::<Vec<_>>()
    };
    // Newest first, as a list, and never a longer id that only shares the start.
    assert_eq!(named("a:"), ["a:two", "a:one"]);
    assert_eq!(named("ab:"), ["ab:two"]);
    // Case counts, and `_` and `%` are no wildcards.
    assert_eq!(named("a"), ["a%:y", "a_:x", "a:two", "ab:two", "a:one"]);
    assert_eq!(named("a_"), ["a_:x"]);
    assert_eq!(named("a%"), ["a%:y"]);
    assert_eq!(named("A"), ["A:z"]);
    // Beyond ASCII, up to the last character there is.
    assert_eq!(named("ç"), ["ç:1"]);
    assert_eq!(named("\u{10FFFF}"), ["\u{10FFFF}", "\u{10FFFF}:3"]);
    assert_eq!(named("\u{10FFFF}:"), ["\u{10FFFF}:3"]);
    assert_eq!(named("").len(), 11);
    assert!(named("c").is_empty());
}

#[test]
fn a_prefixed_list_leaves_out_a_record_that_does_not_read() {
    let db = with_unreadable();
    let names = |list: Vec<Named>| list.into_iter().map(|n| n.name).collect::<Vec<_>>();
    assert_eq!(names(db.list_prefixed("named", "b").unwrap()), ["second"]);
    assert_eq!(
        *db.unreadable.lock().unwrap(),
        HashSet::from([("named".to_owned(), "broken".to_owned())])
    );
}

#[test]
fn a_prefixed_removal_takes_every_id_of_its_range_whether_or_not_it_reads() {
    let db = Db::open(std::path::Path::new(":memory:")).unwrap();
    for id in ["a:one", "ab:two", "a%:y", "b:one"] {
        db.put("named", id, &serde_json::json!({"name": id}))
            .unwrap();
    }
    db.put("named", "a:broken", &serde_json::json!({"name": SECRET}))
        .unwrap();
    db.put("other", "a:three", &serde_json::json!({"name": "a:three"}))
        .unwrap();
    assert_eq!(db.remove_prefixed("named", "a:").unwrap(), 2);
    assert_eq!(db.ids("named").unwrap(), ["ab:two", "a%:y", "b:one"]);
    assert_eq!(db.remove_prefixed("named", "a%").unwrap(), 1);
    assert_eq!(db.remove_prefixed("named", "c").unwrap(), 0);
    assert!(db.contains("other", "a:three").unwrap());
    // A job goes with its log, which only a removal of the job itself takes.
    assert!(db.remove_prefixed("job", "").is_err());
}

#[test]
fn past_a_prefix_is_the_first_text_no_id_with_it_reaches() {
    assert_eq!(past("a:").as_deref(), Some("a;"));
    assert_eq!(past("ç").as_deref(), Some("è"));
    // Surrogates are no characters, so the one after U+D7FF is U+E000.
    assert_eq!(past("\u{D7FF}").as_deref(), Some("\u{E000}"));
    assert_eq!(past("a\u{10FFFF}").as_deref(), Some("b"));
    assert_eq!(past("\u{10FFFF}"), None);
    assert_eq!(past(""), None);
}

#[test]
fn a_job_that_does_not_read_leaves_the_others_listed() {
    let db = Db::open(std::path::Path::new(":memory:")).unwrap();
    db.put("job", "good", &job("good", false)).unwrap();
    // A field renamed: running, but without the kind this release reads.
    let renamed = serde_json::json!({"id": "renamed", "type": "exec", "status": "running"});
    db.put("job", "renamed", &renamed).unwrap();
    let ids = |jobs: Vec<Job>| jobs.into_iter().map(|job| job.id).collect::<Vec<_>>();
    assert_eq!(ids(db.list("job").unwrap()), ["good"]);
    assert_eq!(ids(db.jobs_summary().unwrap()), ["good"]);
    assert_eq!(ids(db.active_jobs().unwrap()), ["good"]);
}

#[test]
fn a_copy_of_the_database_holds_its_records() {
    let dir = std::env::temp_dir().join(format!("thoric-db-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = Db::open(&dir.join(FILE)).unwrap();
    db.put("named", "a", &serde_json::json!({"name": "first"}))
        .unwrap();
    db.copy_to(&dir.join("copy.sqlite")).unwrap();
    // The copy is a database of its own, of the same version, and never overwritten.
    assert!(db.copy_to(&dir.join("copy.sqlite")).is_err());
    drop(db);
    let copy = Db::open(&dir.join("copy.sqlite")).unwrap();
    assert_eq!(copy.list::<Named>("named").unwrap()[0].name, "first");
    drop(copy);
    std::fs::remove_dir_all(&dir).unwrap();
}

fn job(id: &str, finished: bool) -> Job {
    Job {
        id: id.into(),
        kind: "exec".into(),
        resource: None,
        status: if finished { "completed" } else { "running" }.into(),
        created_at: 1,
        finished_at: finished.then_some(2),
        log: String::new(),
        result: None,
        progress: None,
        stages: Vec::new(),
        script: None,
        name: None,
    }
}

#[test]
fn active_jobs_are_the_queued_and_running_ones_without_their_logs() {
    let db = Db::open(std::path::Path::new(":memory:")).unwrap();
    for (id, status) in [
        ("queued", "queued"),
        ("done", "completed"),
        ("running", "running"),
        ("failed", "failed"),
    ] {
        let mut record = job(id, false);
        record.status = status.into();
        record.log = "a long log".into();
        db.put("job", id, &record).unwrap();
    }
    let active = db.active_jobs().unwrap();
    let ids: Vec<&str> = active.iter().map(|job| job.id.as_str()).collect();
    assert_eq!(ids, ["running", "queued"]);
    assert!(active.iter().all(|job| job.log.is_empty()));
}

#[test]
fn job_list_is_bounded_and_strips_logs_before_deserialization() {
    let app = app(false);
    for i in 0..120 {
        let id = i.to_string();
        let j = model::Job {
            id: id.clone(),
            kind: "build".into(),
            resource: None,
            status: "completed".into(),
            created_at: 1,
            finished_at: Some(2),
            log: "x".repeat(262144),
            result: None,
            progress: Some(serde_json::json!({"current": 1})),
            stages: Vec::new(),
            script: None,
            name: None,
        };
        app.db.put("job", &id, &j).unwrap();
    }
    let jobs = app.db.jobs_summary().unwrap();
    assert_eq!(jobs.len(), 100);
    assert_eq!(jobs[0].id, "119");
    assert!(jobs.iter().all(|j| j.log.is_empty()));
    // Progress is small and is what the job list shows for a running download.
    assert!(jobs.iter().all(|j| j.progress.is_some()));
}

#[test]
fn database_round_trip_keeps_snapshot_parent() {
    let db = Db::open(std::path::Path::new(":memory:")).unwrap();
    let record = model::ImageRecord {
        id: "a".into(),
        name: "Snapshot".into(),
        image_id: "sha256:new".into(),
        reference: "snapshot:a".into(),
        parent_image_id: Some("sha256:base".into()),
        source_container: Some("env".into()),
        kind: "snapshot".into(),
        created_at: 1,
        description: String::new(),
        desktop: None,
        build: None,
        compose: None,
        icon: None,
        template_ids: Vec::new(),
    };
    db.put("image", "a", &record).unwrap();
    assert_eq!(
        db.get::<model::ImageRecord>("image", "a")
            .unwrap()
            .parent_image_id,
        record.parent_image_id
    );
    assert_eq!(db.list::<model::ImageRecord>("image").unwrap().len(), 1);
    db.remove("image", "a").unwrap();
    assert!(db.list::<model::ImageRecord>("image").unwrap().is_empty());
}
