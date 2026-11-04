//! Pins down when a desktop's screen goes back to the resolution its settings name: only once the
//! last viewer that fits it to their window turns that off or closes its socket, a closed socket
//! only after the grace a reload needs, and only on the messages the dashboard sends.
use super::*;

/// The text frames the dashboard's `desktop-fit.ts` sends, which its own test holds to the same file.
fn sent(which: &str) -> String {
    let messages: serde_json::Value =
        serde_json::from_str(include_str!("../../../web/tests/desktop-fit.json")).unwrap();
    messages[which].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn the_dashboard_turns_fitting_on_and_off_and_nothing_else_is_taken() {
    let app = crate::test_support::app(false);
    let mut fit = Fit::new(app.clone(), "e");
    assert!(!app.fitters.fitted("e"), "a new socket fits nothing");
    fit.message(&sent("on")).unwrap();
    assert!(app.fitters.fitted("e"));
    fit.message(&sent("off")).unwrap();
    assert!(!app.fitters.fitted("e"));
    let long = format!("{{\"fit\":true{}}}", " ".repeat(MESSAGE_LIMIT));
    for text in [
        "fit",
        "{}",
        r#"{"fit":"true"}"#,
        r#"{"fit":true,"width":800}"#,
        long.as_str(),
    ] {
        assert!(fit.message(text).is_err(), "{text}");
    }
    assert!(!app.fitters.fitted("e"));
}

#[tokio::test]
async fn the_screen_returns_only_once_no_viewer_fits_it() {
    let app = crate::test_support::app(false);
    let mut first = Fit::new(app.clone(), "e");
    let mut second = Fit::new(app.clone(), "e");
    let watching = Fit::new(app.clone(), "e");
    first.set(true);
    // Said twice, it still counts once.
    first.set(true);
    second.set(true);
    first.set(false);
    assert!(app.fitters.fitted("e"), "the second viewer still fits it");
    // A viewer that only watched leaves nothing to undo.
    drop(watching);
    assert!(app.fitters.fitted("e"));
    drop(first);
    assert!(app.fitters.fitted("e"));
    drop(second);
    assert!(!app.fitters.fitted("e"));
    assert!(app.fitters.counts().is_empty(), "nothing is kept for it");
}

#[tokio::test(start_paused = true)]
async fn a_reload_within_the_grace_keeps_the_viewers_size() {
    let app = crate::test_support::app(false);
    let closed = tokio::spawn({
        let app = app.clone();
        async move { return_after(&app, "e", GRACE).await.unwrap() }
    });
    tokio::time::sleep(GRACE / 2).await;
    let mut reloaded = Fit::new(app.clone(), "e");
    reloaded.set(true);
    assert_eq!(closed.await.unwrap(), Returned::Fitted);
    // Once none fits it, the return goes on to the container, which this installation lacks.
    reloaded.set(false);
    assert_eq!(
        return_after(&app, "e", GRACE).await.unwrap(),
        Returned::Skipped
    );
}
