//! Which Docker Engines the Management page warns about: those whose API is older than what the
//! manager's own checks ask for, so the page and the manager's behavior never disagree.
use super::*;

#[test]
fn an_engine_lacks_what_its_api_version_does_not_reach() {
    // Docker Engine 27, 28 and 29.
    assert_eq!(lacking("1.47"), ["isolation", "mac_address"]);
    assert_eq!(lacking("1.48"), ["mac_address"]);
    assert_eq!(lacking("1.53"), ["mac_address"]);
    assert!(lacking("1.54").is_empty());
    assert!(lacking("1.55").is_empty());
    assert!(lacking("2.0").is_empty());
}

#[test]
fn a_version_that_does_not_read_lacks_everything() {
    for api in ["", "unknown", "1", "1.x"] {
        assert_eq!(lacking(api), ["isolation", "mac_address"], "{api:?}");
    }
}

#[tokio::test]
async fn an_engine_that_does_not_answer_is_not_reported_here() {
    let app = crate::test_support::app(false);
    assert_eq!(app.engine_gaps().await, None);
}
