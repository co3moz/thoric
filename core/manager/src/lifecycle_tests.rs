//! The lifecycle policy as requests give it and the reaper reads it: what a creation and a
//! settings change make of it, what each check refuses, what is due when, what a restore keeps,
//! and which requests count as a container's use.
use super::*;
use serde_json::json;

const NOW: u64 = 1_800_000_000;

fn idle(minutes: u32) -> Lifecycle {
    Lifecycle {
        idle_stop_minutes: minutes,
        ..Lifecycle::default()
    }
}

#[test]
fn a_record_without_a_policy_reads_as_never() {
    let spec = crate::test_support::spec();
    assert_eq!(spec.lifecycle, Lifecycle::default());
    assert!(spec.lifecycle.is_never());
    let record: Lifecycle = serde_json::from_value(json!({})).unwrap();
    assert!(record.is_never());
    assert_eq!(record.on_expiry, OnExpiry::Stop);
}

#[test]
fn a_creation_gets_the_installation_idle_stop_unless_it_names_one() {
    assert_eq!(for_creation(None, 30, NOW).unwrap(), idle(30));
    assert_eq!(
        for_creation(Some(&json!({"idle_stop_minutes": 0})), 30, NOW).unwrap(),
        idle(0)
    );
    let throwaway = for_creation(
        Some(&json!({"expires_in_minutes": 90, "on_expiry": "delete", "delete_volumes": true})),
        30,
        NOW,
    )
    .unwrap();
    assert_eq!(
        throwaway,
        Lifecycle {
            idle_stop_minutes: 30,
            expires_at: Some(NOW + 90 * 60),
            on_expiry: OnExpiry::Delete,
            delete_volumes: true,
        }
    );
}

#[test]
fn a_change_keeps_the_fields_it_leaves_out() {
    let current = Lifecycle {
        idle_stop_minutes: 20,
        expires_at: Some(NOW + 60),
        on_expiry: OnExpiry::Delete,
        delete_volumes: true,
    };
    let extended = current
        .requested(&json!({"expires_in_minutes": 120}), NOW)
        .unwrap();
    assert_eq!(
        extended,
        Lifecycle {
            expires_at: Some(NOW + 120 * 60),
            ..current.clone()
        }
    );
    let cleared = current
        .requested(&json!({"expires_at": null, "idle_stop_minutes": 0}), NOW)
        .unwrap();
    assert!(cleared.is_never());
    // Stopping keeps every volume, so a policy changed to stop deletes none.
    let stopping = current
        .requested(&json!({"on_expiry": "stop"}), NOW)
        .unwrap();
    assert_eq!(stopping.on_expiry, OnExpiry::Stop);
    assert!(!stopping.delete_volumes);
    assert_eq!(current.requested(&json!({}), NOW).unwrap(), current);
}

#[test]
fn an_expiry_that_passed_is_kept_unless_the_request_changes_it() {
    let passed = Lifecycle {
        expires_at: Some(NOW - 10),
        ..Lifecycle::default()
    };
    // Saving other settings while the reaper has not come yet keeps it as it is.
    let same = passed
        .requested(
            &json!({"expires_at": NOW - 10, "idle_stop_minutes": 5}),
            NOW,
        )
        .unwrap();
    assert_eq!(same.expires_at, Some(NOW - 10));
    for request in [
        json!({"expires_at": NOW - 5}),
        json!({"expires_at": NOW}),
        json!({"expires_at": NOW + MAX_EXPIRY_MINUTES * 60 + 1}),
    ] {
        let error = passed.requested(&request, NOW).unwrap_err();
        assert!(error.to_string().contains("in the future"), "{error}");
    }
}

#[test]
fn a_request_out_of_bounds_is_refused() {
    let never = Lifecycle::default();
    for (request, refusal) in [
        (json!({"idle_stop_minutes": 10_081}), "at most 10080"),
        (json!({"expires_in_minutes": 0}), "expires_in_minutes"),
        (
            json!({"expires_in_minutes": MAX_EXPIRY_MINUTES + 1}),
            "expires_in_minutes",
        ),
        (
            json!({"expires_in_minutes": 5, "expires_at": NOW + 600}),
            "not both",
        ),
        (
            json!({"delete_volumes": true}),
            "delete_volumes needs on_expiry delete",
        ),
        (json!({"on_expiry": "archive"}), "Invalid lifecycle"),
        (json!({"expires_at_minutes": 5}), "Invalid lifecycle"),
        (json!(null), "must be an object"),
    ] {
        let error = never.requested(&request, NOW).unwrap_err();
        assert!(
            format!("{error:#}").contains(refusal),
            "{request}: {error:#}"
        );
    }
}

#[test]
fn the_installation_idle_stop_is_whole_minutes_up_to_a_week() {
    assert_eq!(idle_stop_default(None).unwrap(), 0);
    assert_eq!(idle_stop_default(Some("45")).unwrap(), 45);
    assert_eq!(idle_stop_default(Some("10080")).unwrap(), 10_080);
    for refused in ["10081", "-1", "1.5", "an hour", ""] {
        assert!(idle_stop_default(Some(refused)).is_err(), "{refused}");
    }
}

#[test]
fn an_expiry_comes_first_whatever_the_state() {
    let expired = Lifecycle {
        idle_stop_minutes: 10,
        expires_at: Some(NOW),
        on_expiry: OnExpiry::Delete,
        delete_volumes: false,
    };
    assert_eq!(
        due(&expired, false, None, NOW),
        Some(Due::Expiry(OnExpiry::Delete))
    );
    assert_eq!(
        due(&expired, true, Some(Duration::from_secs(3600)), NOW),
        Some(Due::Expiry(OnExpiry::Delete))
    );
    assert_eq!(due(&expired, true, None, NOW - 1), None);
}

#[test]
fn an_idle_stop_comes_only_to_a_running_desktop_nothing_uses() {
    let policy = idle(30);
    let long = Some(Duration::from_secs(30 * 60));
    assert_eq!(due(&policy, true, long, NOW), Some(Due::IdleStop(30)));
    assert_eq!(due(&policy, false, long, NOW), None);
    assert_eq!(due(&policy, true, None, NOW), None);
    assert_eq!(
        due(&policy, true, Some(Duration::from_secs(30 * 60 - 1)), NOW),
        None
    );
    assert_eq!(due(&idle(0), true, Some(Duration::MAX), NOW), None);
}

#[test]
fn a_restore_clears_an_expiry_that_passed_and_keeps_the_rest() {
    let mut passed = Lifecycle {
        idle_stop_minutes: 15,
        expires_at: Some(NOW - 1),
        on_expiry: OnExpiry::Delete,
        delete_volumes: true,
    };
    assert!(restored(&mut passed, NOW));
    assert_eq!(passed.expires_at, None);
    assert_eq!(passed.idle_stop_minutes, 15);
    let mut ahead = Lifecycle {
        expires_at: Some(NOW + 60),
        ..Lifecycle::default()
    };
    assert!(!restored(&mut ahead, NOW));
    assert_eq!(ahead.expires_at, Some(NOW + 60));
}

#[test]
fn a_saved_policy_with_a_passed_expiry_still_validates() {
    let mut spec = crate::test_support::spec();
    spec.lifecycle.expires_at = Some(1);
    spec.validate().unwrap();
    spec.lifecycle.delete_volumes = true;
    assert!(spec.validate().is_err());
}

#[test]
fn only_a_container_route_names_a_container() {
    assert_eq!(container_of("/api/containers/abc/files/write"), Some("abc"));
    assert_eq!(container_of("/api/containers/abc"), Some("abc"));
    assert_eq!(container_of("/api/containers"), None);
    assert_eq!(container_of("/api/containers/"), None);
    assert_eq!(container_of("/api/images/abc/build"), None);
    assert_eq!(
        container_of(&format!("/api/containers/{}", "x".repeat(65))),
        None
    );
}

#[tokio::test]
async fn a_request_that_changes_a_container_uses_it_and_a_read_does_not() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let app = crate::test_support::app(false);
    let read = Request::get("/api/containers/missing/screenshot")
        .header("host", "localhost:8080")
        .body(Body::empty())
        .unwrap();
    crate::router(app.clone()).oneshot(read).await.unwrap();
    app.lifecycle.retain(|id| id != "missing");
    let since_start = app.lifecycle.idle_for("missing");
    assert!(since_start.is_some());
    let used = Request::post("/api/containers/used/action")
        .header("host", "localhost:8080")
        .header("x-thoric-request", "1")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"action":"start"}"#))
        .unwrap();
    crate::router(app.clone()).oneshot(used).await.unwrap();
    // Noted under its own name, also when the request found no such container.
    let mut seen = Vec::new();
    app.lifecycle.retain(|id| {
        seen.push(id.to_owned());
        true
    });
    assert_eq!(seen, ["used"]);
}

#[tokio::test]
async fn a_download_keeps_its_container_in_use_until_it_has_been_sent() {
    let app = crate::test_support::app(false);
    let response = Response::new(axum::body::Body::from("file"));
    let response = lasting(response, app.lifecycle.open("desk"));
    assert_eq!(app.lifecycle.idle_for("desk"), None);
    // It reads as the body it carries.
    assert_eq!(
        axum::body::HttpBody::size_hint(response.body()).exact(),
        Some(4)
    );
    let sent = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    assert_eq!(&sent[..], b"file");
    assert!(app.lifecycle.idle_for("desk").is_some());
}

#[tokio::test(start_paused = true)]
async fn the_lists_say_when_an_idle_stop_comes_if_nothing_uses_the_desktop_from_now_on() {
    let app = crate::test_support::app(false);
    let mut spec = crate::test_support::spec();
    spec.lifecycle = idle(30);
    let record: Container = serde_json::from_value(json!({
        "id": "desk", "container": "thoric-desk", "private_network": "p", "egress_network": "e",
        "source_image_id": "sha256:desktop", "created_at": 1, "spec": spec,
    }))
    .unwrap();
    let close = |at: Option<u64>, ahead: u64| {
        let at = at.expect("a time");
        let expected = crate::model::now() + ahead;
        assert!(at.abs_diff(expected) <= 2, "{at} is not {expected}");
    };
    tokio::time::advance(Duration::from_secs(10 * 60)).await;
    let (in_use, at) = use_of(&app, &record, true);
    assert!(!in_use);
    close(at, 20 * 60);
    // An MCP call that asks is a use, so a whole idle time lies ahead of it.
    let call = app.lifecycle.open("desk");
    let (in_use, at) = use_of(&app, &record, true);
    assert!(in_use);
    close(at, 30 * 60);
    drop(call);
    assert_eq!(use_of(&app, &record, false), (false, None));
    let mut never = record.clone();
    never.spec.lifecycle = Lifecycle::default();
    assert_eq!(use_of(&app, &never, true), (false, None));
}

#[test]
fn the_dashboard_offers_the_idle_stops_the_manager_takes() {
    let lifecycle = include_str!("../../web/src/lib/lifecycle.ts");
    assert!(lifecycle.contains(&format!("MAX_IDLE_MINUTES = {MAX_IDLE_MINUTES}\n")));
    let settings = include_str!("../../web/src/lib/settings.ts");
    assert!(settings.contains(&format!(
        "IDLE_STOP_DEFAULT: {{ kind: \"number\", min: 0, max: {MAX_IDLE_MINUTES}, unit: \"minutes\" }}"
    )));
}

#[test]
fn a_creation_request_carries_the_policy_in_the_stored_shape() {
    let mut config = crate::config::Config::for_tests();
    config.defaults.idle_stop_minutes = 25;
    let request = crate::api::creation_request(
        &config,
        json!({"name": "Agent", "image": "ghcr.io/co3moz/thoric/ubuntu-xfce:24.04", "lifecycle": {"expires_in_minutes": 60, "on_expiry": "delete"}}),
    )
    .unwrap();
    let lifecycle = &request.spec.lifecycle;
    assert_eq!(lifecycle.idle_stop_minutes, 25);
    assert_eq!(lifecycle.on_expiry, OnExpiry::Delete);
    let at = lifecycle.expires_at.unwrap();
    assert!(at > crate::model::now() + 59 * 60 && at <= crate::model::now() + 60 * 60);
    let plain = crate::api::creation_request(
        &config,
        json!({"name": "Plain", "image": "ghcr.io/co3moz/thoric/ubuntu-xfce:24.04"}),
    )
    .unwrap();
    assert_eq!(plain.spec.lifecycle, idle(25));
    let refused = crate::api::creation_request(
        &config,
        json!({"name": "Bad", "image": "ghcr.io/co3moz/thoric/ubuntu-xfce:24.04", "lifecycle": {"expires_in_minutes": 0}}),
    );
    assert!(refused.is_err());
}
