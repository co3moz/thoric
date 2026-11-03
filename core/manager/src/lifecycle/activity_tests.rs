//! How the use of a container is kept: idle time from the manager's start or the last use, none
//! while a use lasts, the end of a use as the last one, and what is forgotten.
use super::*;

const MINUTE: Duration = Duration::from_secs(60);

#[tokio::test(start_paused = true)]
async fn a_container_nothing_used_counts_from_the_start() {
    let activity = Activity::default();
    tokio::time::advance(5 * MINUTE).await;
    assert_eq!(activity.idle_for("unused"), Some(5 * MINUTE));
}

#[tokio::test(start_paused = true)]
async fn a_touch_starts_the_idle_time_again() {
    let activity = Activity::default();
    tokio::time::advance(10 * MINUTE).await;
    activity.touch("desk");
    tokio::time::advance(3 * MINUTE).await;
    assert_eq!(activity.idle_for("desk"), Some(3 * MINUTE));
}

#[tokio::test(start_paused = true)]
async fn a_use_that_lasts_keeps_it_in_use_and_its_end_counts() {
    let activity = Activity::default();
    let viewer = activity.open("desk");
    let script = activity.open("desk");
    tokio::time::advance(90 * MINUTE).await;
    assert_eq!(activity.idle_for("desk"), None);
    drop(viewer);
    assert_eq!(activity.idle_for("desk"), None);
    tokio::time::advance(MINUTE).await;
    drop(script);
    tokio::time::advance(2 * MINUTE).await;
    assert_eq!(activity.idle_for("desk"), Some(2 * MINUTE));
}

#[tokio::test(start_paused = true)]
async fn forgetting_keeps_what_is_in_use() {
    let activity = Activity::default();
    activity.touch("deleted");
    let _open = activity.open("deleted-but-open");
    tokio::time::advance(MINUTE).await;
    activity.retain(|_| false);
    // Forgotten, it counts from the start again; one in use stays in use.
    assert_eq!(activity.idle_for("deleted"), Some(MINUTE));
    assert_eq!(activity.idle_for("deleted-but-open"), None);
}
