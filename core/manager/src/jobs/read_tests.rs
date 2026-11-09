//! Pins down how an agent reads a job: the whole job as before unless it asks for part of the log,
//! a part that says where it lies in the job's whole output so the next read gets only what was
//! added, also after a new manager started the log again, cut at a character's start, the end of
//! the log by default over MCP, and a wait that answers as soon as the job ends and no later than
//! it was allowed to.
use super::*;
use crate::test_support::app;
use std::sync::Arc;

fn job(log: &str) -> Job {
    Job {
        id: "j".into(),
        kind: "exec".into(),
        resource: None,
        name: None,
        status: "running".into(),
        created_at: 1,
        finished_at: None,
        log: log.into(),
        result: None,
        progress: None,
        stages: Vec::new(),
        script: None,
    }
}

#[test]
fn a_part_of_the_log_says_where_it_lies_in_the_whole_output() {
    // The log keeps the last ten bytes of the hundred written.
    let cut = |offset, tail| {
        let mut job = job("0123456789");
        let at = window(&mut job, 100, offset, tail);
        (job.log, at)
    };
    assert_eq!(cut(None, Some(4)), ("6789".into(), (96, 100)));
    assert_eq!(cut(Some(95), None), ("56789".into(), (95, 100)));
    // What the log no longer keeps is left out, and the start says so.
    assert_eq!(cut(Some(20), None), ("0123456789".into(), (90, 100)));
    // Nothing new since the end.
    assert_eq!(cut(Some(100), None), (String::new(), (100, 100)));
    assert_eq!(cut(Some(400), Some(4)), (String::new(), (100, 100)));
    // From an offset, at most the last bytes.
    assert_eq!(cut(Some(92), Some(3)), ("789".into(), (97, 100)));
    // A cut inside a character starts after it.
    let mut job = job("aéb");
    assert_eq!(window(&mut job, 4, None, Some(2)), (3, 4));
    assert_eq!(job.log, "b");
}

#[test]
fn a_follower_reads_on_in_order_across_a_log_started_again() {
    // An update job, whose log a new manager starts again when it takes the job over.
    let db = crate::db::Db::open(std::path::Path::new(":memory:")).unwrap();
    db.put("job", "j", &job("from an older manager\n")).unwrap();
    db.append_job_log("j", "the update's first lines\n")
        .unwrap();
    let (_, followed) = db.job_and_end("j").unwrap();
    db.restart_job_log("j", "The manager restarted; its output follows.\n")
        .unwrap();
    db.append_job_log("j", "the update's first lines\nand more\n")
        .unwrap();
    let again = "The manager restarted; its output follows.\nthe update's first lines\nand more\n";

    // A follower that read up to the restart gets what came since, from where it stopped.
    let (mut read, end) = db.job_and_end("j").unwrap();
    assert_eq!(
        window(&mut read, end, Some(followed), None),
        (followed, end)
    );
    assert_eq!(read.log, again);
    assert_eq!(end, followed + again.len() as u64);
    // A reader from the start gets the log as it is, from where it now begins.
    let (mut read, end) = db.job_and_end("j").unwrap();
    assert_eq!(window(&mut read, end, Some(0), None), (followed, end));
    assert_eq!(read.log, again);
}

/// A running job whose log holds `log`, which ends when `end` is sent.
async fn running(app: &Arc<App>, log: &str) -> (String, tokio::sync::oneshot::Sender<()>) {
    let (end, ended) = tokio::sync::oneshot::channel::<()>();
    let (id, _) = super::super::start(
        app.clone(),
        "exec",
        "container:one".into(),
        true,
        |_, _| async {
            let _ = ended.await;
            Ok(json!({"exit_code": 0}))
        },
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while app.db.job(&id).unwrap().status != "running" {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    app.append_log(&id, log).unwrap();
    (id, end)
}

#[tokio::test]
async fn a_read_without_a_part_is_the_whole_job_and_one_with_a_part_follows_it() {
    let app = app(false);
    let (id, _end) = running(&app, "first line").await;
    let whole = answer(&app, &id, &Read::default(), None).await.unwrap();
    assert_eq!(whole["log"], "first line\n");
    assert!(whole.get("log_start").is_none() && whole.get("log_end").is_none());
    let read = Read {
        log_tail_bytes: Some(5),
        ..Default::default()
    };
    let tail = answer(&app, &id, &read, None).await.unwrap();
    assert_eq!(
        (tail["log"].as_str(), tail["log_start"].as_u64()),
        (Some("line\n"), Some(6))
    );
    let end = tail["log_end"].as_u64().unwrap();
    app.append_log(&id, "second").unwrap();
    let read = Read {
        log_offset: Some(end),
        ..Default::default()
    };
    let new = answer(&app, &id, &read, None).await.unwrap();
    assert_eq!(new["log"], "second\n");
    assert_eq!(new["log_start"].as_u64(), Some(end));
    // Over MCP the end of the log is what an agent gets when it does not say.
    let (_, read) = mcp_arguments(json!({"job_id": id})).unwrap();
    assert_eq!(read.log_tail_bytes, Some(MCP_TAIL));
    assert!(mcp_arguments(json!({"job_id": id, "log_tail_bytes": 0})).is_ok());
    let refused = Read {
        wait_seconds: Some(MAX_WAIT + 1),
        ..Default::default()
    };
    assert!(answer(&app, &id, &refused, None).await.is_err());
}

#[tokio::test]
async fn a_wait_answers_once_the_job_ends_and_no_later_than_it_may() {
    let app = app(false);
    let (id, end) = running(&app, "working").await;
    let waiting = Read {
        wait_seconds: Some(60),
        ..Default::default()
    };
    let read = tokio::spawn({
        let app = app.clone();
        let id = id.clone();
        async move { answer(&app, &id, &waiting, None).await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!read.is_finished(), "the job is still running");
    end.send(()).unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(5), read)
        .await
        .expect("the wait ends with the job")
        .unwrap()
        .unwrap();
    assert_eq!(ended["status"], "completed");
    assert_eq!(ended["result"]["exit_code"], 0);
    // A job that does not end is answered as it is once the wait is over.
    let (id, _end) = running(&app, "still working").await;
    let started = std::time::Instant::now();
    let read = Read {
        wait_seconds: Some(1),
        ..Default::default()
    };
    let still = answer(&app, &id, &read, None).await.unwrap();
    assert_eq!(still["status"], "running");
    assert!(started.elapsed() >= Duration::from_secs(1));
    // The manager stopping ends the wait too.
    let read = tokio::spawn({
        let app = app.clone();
        async move {
            let read = Read {
                wait_seconds: Some(60),
                ..Default::default()
            };
            answer(&app, &id, &read, None).await
        }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    app.events.close();
    let stopped = tokio::time::timeout(Duration::from_secs(5), read)
        .await
        .expect("a stop ends the wait")
        .unwrap()
        .unwrap();
    assert_eq!(stopped["status"], "running");
}

#[tokio::test]
async fn a_job_read_again_without_a_wait_is_answered_with_a_hint_to_wait() {
    let app = app(false);
    let (id, end) = running(&app, "working").await;
    let (_, poll) = mcp_arguments(json!({"job_id": id})).unwrap();
    let read = |read: Read| {
        let (app, id) = (app.clone(), id.clone());
        async move { mcp_answer(&app, &id, &read, None).await.unwrap() }
    };
    let again = || Read {
        log_tail_bytes: poll.log_tail_bytes,
        ..Default::default()
    };
    // The first read is not a poll yet.
    assert!(read(again()).await.get("hint").is_none());
    let second = read(again()).await;
    let hint = second["hint"].as_str().unwrap();
    assert!(
        hint.contains("2 times") && hint.contains("wait_seconds"),
        "{hint}"
    );
    assert!(
        read(again()).await["hint"]
            .as_str()
            .unwrap()
            .contains("3 times")
    );
    // The dashboard's own reads of the job, which it repeats, are never counted or hinted.
    assert!(
        answer(&app, &id, &Read::default(), None)
            .await
            .unwrap()
            .get("hint")
            .is_none()
    );
    // A read that waits is what the hint asks for, and starts the count again.
    let waited = read(Read {
        wait_seconds: Some(1),
        ..again()
    })
    .await;
    assert!(waited.get("hint").is_none());
    assert!(read(again()).await.get("hint").is_none());
    assert!(read(again()).await.get("hint").is_some());
    // An ended job is read as often as anyone likes.
    end.send(()).unwrap();
    let ended = read(Read {
        wait_seconds: Some(5),
        ..again()
    })
    .await;
    assert_eq!(ended["status"], "completed");
    for _ in 0..3 {
        assert!(read(again()).await.get("hint").is_none());
    }
    assert!(app.polls.0.lock().unwrap().is_empty());
}

#[test]
fn the_counted_jobs_stay_bounded() {
    let polls = Polls::default();
    for job in 0..POLLED_JOBS * 3 {
        polls.count(&job.to_string());
        assert!(polls.0.lock().unwrap().len() <= POLLED_JOBS);
    }
    // A job that is counted keeps its count while the table is full.
    assert_eq!(polls.count(&(POLLED_JOBS * 3 - 1).to_string()), 2);
}
