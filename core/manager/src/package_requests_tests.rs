//! The reader of a package command's log: it adds the log to the job a whole line at a time,
//! and keeps a line that has no end yet only up to `LINE_LIMIT`, so a command that redraws one
//! line for the two hours an update may take never grows the manager's memory with it.
use super::*;
use crate::test_support;
use std::sync::Arc;

/// A job that runs until `end` is dropped, so the reader has one to write to.
fn job(app: &Arc<App>) -> (String, tokio::sync::oneshot::Sender<()>) {
    let (end, ended) = tokio::sync::oneshot::channel::<()>();
    let (id, _) = jobs::start(
        app.clone(),
        "test",
        "test".into(),
        false,
        |_, _| async move {
            let _ = ended.await;
            Ok(Value::Null)
        },
    )
    .unwrap();
    (id, end)
}

#[tokio::test]
async fn a_line_without_an_end_is_added_once_it_reaches_the_limit() {
    let app = test_support::app(false);
    let (id, _end) = job(&app);
    let dir = std::env::temp_dir().join(format!("thoric-log-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("log");
    let mut reader = LogReader::default();
    // A progress line redrawn with carriage returns, longer than the limit, and its end later.
    let redrawn = "\rPulling 12%".repeat(LINE_LIMIT / 10);
    std::fs::write(&path, &redrawn).unwrap();
    assert_eq!(
        reader.read(&app, &id, &path).await.unwrap(),
        redrawn.len() as u64
    );
    assert!(reader.partial.is_empty(), "the long line was added");
    assert_eq!(app.db.job(&id).unwrap().log, format!("{redrawn}\n"));
    // What comes next is the line's rest, kept until its end as before.
    std::fs::write(&path, format!("{redrawn}\rdone\nnext")).unwrap();
    reader.read(&app, &id, &path).await.unwrap();
    assert_eq!(reader.partial, "next");
    assert_eq!(app.db.job(&id).unwrap().log, format!("{redrawn}\n\rdone\n"));
    reader.finish(&app, &id).unwrap();
    assert_eq!(
        app.db.job(&id).unwrap().log,
        format!("{redrawn}\n\rdone\nnext\n")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
