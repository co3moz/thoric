//! Pins down what stays of ended jobs: pruning keeps the newest jobs of each class and every
//! unfinished one, so scripts never push a build out, and takes a pruned job's log and the
//! request kept to retry it; the ids pruned are remembered within a bound; a list names only the
//! kinds or the class it is asked for; and the hourly counts tell a day from the next, how each
//! job ended and which class it was, and drop the hours older than they are kept.
use super::*;
use crate::model::Job;
use std::path::Path;

fn job(id: &str, kind: &str, finished: bool) -> Job {
    Job {
        id: id.into(),
        kind: kind.into(),
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

fn db() -> Db {
    Db::open(Path::new(":memory:")).unwrap()
}

fn ids(db: &Db) -> Vec<String> {
    let mut ids = db.ids("job").unwrap();
    ids.sort();
    ids
}

fn keep(scripts: usize, others: usize) -> Kept {
    Kept { scripts, others }
}

#[test]
fn pruning_keeps_the_newest_jobs_and_the_unfinished_ones() {
    let db = db();
    for (id, finished) in [("old", true), ("running", false), ("new", true)] {
        db.put("job", id, &job(id, "exec", finished)).unwrap();
    }
    assert_eq!(db.prune_jobs(keep(2, 2)).unwrap(), 1);
    assert_eq!(ids(&db), ["new", "running"]);
    // The running job is older than the one kept, and stays.
    assert_eq!(db.prune_jobs(keep(1, 1)).unwrap(), 0);
    assert_eq!(ids(&db), ["new", "running"]);
    // A request kept to retry a job goes with it.
    let requests = crate::jobs::retry::REQUESTS;
    for id in ["new", "old"] {
        db.put(requests, id, &serde_json::json!({})).unwrap();
    }
    db.prune_jobs(keep(2, 2)).unwrap();
    assert!(db.contains(requests, "new").unwrap());
    assert!(!db.contains(requests, "old").unwrap());
}

#[test]
fn scripts_prune_scripts_and_leave_every_other_job() {
    let db = db();
    let requests = crate::jobs::retry::REQUESTS;
    // A build and a snapshot from before a day of scripts, each with a log and a request.
    for (id, kind) in [("snapshot-1", "snapshot"), ("build-1", "build")] {
        db.put("job", id, &job(id, kind, true)).unwrap();
        db.append_job_log(id, "its log\n").unwrap();
        db.put(requests, id, &serde_json::json!({})).unwrap();
    }
    for index in 0..40 {
        let id = format!("exec-{index:02}");
        db.put("job", &id, &job(&id, "exec", true)).unwrap();
        db.append_job_log(&id, "a script's line\n").unwrap();
    }
    db.put("job", "build-2", &job("build-2", "build", true))
        .unwrap();
    assert_eq!(db.prune_jobs(keep(3, 2)).unwrap(), 38);
    assert_eq!(
        ids(&db),
        ["build-1", "build-2", "exec-37", "exec-38", "exec-39"],
        "the oldest job of the other kinds goes, never for a script"
    );
    // The logs and the request of what went are gone, and those of what stays are not.
    let logged: Vec<String> = {
        let conn = db.connection();
        let mut statement = conn
            .prepare("SELECT DISTINCT job FROM job_log ORDER BY job")
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(logged, ["build-1", "exec-37", "exec-38", "exec-39"]);
    assert_eq!(db.ids(requests).unwrap(), ["build-1"]);
    // What went is remembered, what stays or never was is not.
    assert!(db.was_pruned("exec-00") && db.was_pruned("snapshot-1"));
    assert!(!db.was_pruned("exec-39") && !db.was_pruned("build-1") && !db.was_pruned("never"));
    // A thousand more scripts later the builds are still there.
    for index in 100..1100 {
        let id = format!("exec-{index}");
        db.put("job", &id, &job(&id, "exec", true)).unwrap();
    }
    db.prune_jobs(keep(3, 2)).unwrap();
    assert_eq!(
        ids(&db),
        ["build-1", "build-2", "exec-1097", "exec-1098", "exec-1099"]
    );
}

#[test]
fn the_pruned_ids_remembered_are_bounded() {
    let mut pruned = Pruned::default();
    for index in 0..REMEMBERED + 10 {
        pruned.remember(format!("job-{index}"));
    }
    // Again, which adds nothing.
    pruned.remember(format!("job-{}", REMEMBERED + 9));
    assert_eq!(pruned.ids.len(), REMEMBERED);
    assert_eq!(pruned.order.len(), REMEMBERED);
    assert!(!pruned.ids.contains("job-9"), "the oldest are forgotten");
    assert!(pruned.ids.contains("job-10"));
    assert!(pruned.ids.contains(&format!("job-{}", REMEMBERED + 9)));
}

#[test]
fn a_list_names_the_kinds_or_the_class_it_is_asked_for() {
    let db = db();
    for (id, kind) in [
        ("b1", "build"),
        ("e1", "exec"),
        ("s1", "snapshot"),
        ("e2", "exec"),
        ("c1", "create"),
    ] {
        let mut record = job(id, kind, true);
        record.log = "kept out of a list".into();
        db.put("job", id, &record).unwrap();
    }
    let listed = |kinds: &[&str], class| {
        let filter = Filter {
            kinds: kinds.iter().map(|kind| kind.to_string()).collect(),
            class,
        };
        let jobs = db.jobs_listed(&filter).unwrap();
        assert!(jobs.iter().all(|job| job.log.is_empty()));
        jobs.into_iter().map(|job| job.id).collect::<Vec<_>>()
    };
    assert_eq!(listed(&[], None), ["c1", "e2", "s1", "e1", "b1"]);
    assert_eq!(listed(&[], Some(Class::Other)), ["c1", "s1", "b1"]);
    assert_eq!(listed(&[], Some(Class::Script)), ["e2", "e1"]);
    assert_eq!(listed(&["build", "snapshot"], None), ["s1", "b1"]);
    assert_eq!(listed(&["build", "exec"], Some(Class::Other)), ["b1"]);
    // A kind is a value, never SQL.
    assert!(listed(&["exec' OR 1=1 --"], None).is_empty());
    // The newest 100 of what is asked for, however many scripts came after.
    for index in 0..150 {
        let id = format!("later-{index}");
        db.put("job", &id, &job(&id, "exec", true)).unwrap();
    }
    assert_eq!(listed(&[], None).len(), 100);
    assert!(!listed(&[], None).contains(&"b1".to_string()));
    assert_eq!(listed(&[], Some(Class::Other)), ["c1", "s1", "b1"]);
}

#[test]
fn counts_tell_the_hours_the_classes_and_how_a_job_ended() {
    let db = db();
    const DAY: u64 = 86_400;
    let kept = 90 * 24;
    // The last second of a day and the first of the next are two hours, in two days.
    let midnight = 20_000 * DAY;
    for (class, status, at) in [
        (Class::Script, "completed", midnight - 1),
        (Class::Script, "completed", midnight - 3000),
        (Class::Script, "failed", midnight - 1),
        (Class::Other, "completed", midnight - 1),
        (Class::Script, "completed", midnight),
        (Class::Other, "interrupted", midnight + 3599),
        (Class::Other, "anything else", midnight + 1),
    ] {
        db.count_job_end(class, status, at, kept).unwrap();
    }
    let count = |at, class: &str, completed, failed, interrupted| Count {
        at,
        class: class.into(),
        completed,
        failed,
        interrupted,
    };
    assert_eq!(
        db.job_counts(0).unwrap(),
        [
            count(midnight - 3600, "other", 1, 0, 0),
            count(midnight - 3600, "script", 2, 1, 0),
            count(midnight, "other", 0, 0, 2),
            count(midnight, "script", 1, 0, 0),
        ]
    );
    // From a time inside an hour, that hour is whole.
    assert_eq!(db.job_counts(midnight + 1800).unwrap().len(), 2);
    assert!(db.job_counts(midnight + 3600).unwrap().is_empty());
}

#[test]
fn counts_older_than_they_are_kept_go_when_a_job_ends() {
    let db = db();
    let hour = |n: u64| n * 3600;
    for n in [100, 101, 124, 125] {
        db.count_job_end(Class::Script, "completed", hour(n), 24)
            .unwrap();
    }
    let hours = |db: &Db| -> Vec<u64> {
        db.job_counts(0)
            .unwrap()
            .iter()
            .map(|count| count.at / 3600)
            .collect()
    };
    // The newest hour and the 23 before it stay.
    assert_eq!(hours(&db), [124, 125]);
    db.count_job_end(Class::Other, "failed", hour(148), 24)
        .unwrap();
    assert_eq!(hours(&db), [125, 148]);
    // A job whose clock reads earlier than the newest hour drops nothing newer.
    db.count_job_end(Class::Other, "failed", hour(130), 24)
        .unwrap();
    assert_eq!(hours(&db), [125, 130, 148]);
}
