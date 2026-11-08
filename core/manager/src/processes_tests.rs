//! Pins down what a process list and a signal accept before any Docker work, the arguments the
//! helper gets, what its answers mean, and that the helper's list reads as these types: the
//! fixture `core/manager/tests/process-list.json` is the helper's shape, which its Python tests
//! hold the helper to.
use super::*;
use crate::test_support::app;
use serde_json::json;

fn list(value: Value) -> std::result::Result<ListProcesses, serde_json::Error> {
    serde_json::from_value(value)
}

#[test]
fn a_process_list_request_has_bounded_defaults() {
    let options = list(json!({})).unwrap();
    assert_eq!(options.sort, Order::Cpu);
    assert_eq!(options.limit, 50);
    assert_eq!(options.interval_ms, 500);
    assert!(options.query.is_none());
    assert!(options.validate().is_ok());
    for bad in [
        json!({"limit": 0}),
        json!({"limit": 1001}),
        json!({"interval_ms": 99}),
        json!({"interval_ms": 5001}),
        json!({"query": "a\nb"}),
        json!({"query": "x".repeat(201)}),
    ] {
        assert!(list(bad.clone()).unwrap().validate().is_err(), "{bad}");
    }
    assert!(list(json!({"sort": "name"})).is_err());
    assert!(list(json!({"unexpected": true})).is_err());
    assert!(
        list(json!({"sort": "memory", "limit": 1000, "query": "chrome", "interval_ms": 5000}))
            .unwrap()
            .validate()
            .is_ok()
    );
}

#[test]
fn the_helper_gets_the_query_only_when_there_is_one() {
    let options = list(json!({"sort": "memory", "limit": 20, "query": ""})).unwrap();
    assert_eq!(list_arguments(&options), ["list", "500", "memory", "20"]);
    let options = list(json!({"query": "Chrome --type", "interval_ms": 250})).unwrap();
    assert_eq!(
        list_arguments(&options),
        ["list", "250", "cpu", "50", "Chrome --type"]
    );
    let args = helper_args("abc", "1001:1001", &list_arguments(&options));
    assert_eq!(
        &args[..7],
        ["exec", "--user", "1001:1001", "abc", "python3", "-I", "-c"]
    );
    assert_eq!(args[7], HELPER);
    assert_eq!(args[8..], list_arguments(&options));
}

#[test]
fn a_signal_without_a_body_asks_what_an_empty_body_asks() {
    let empty: Kill = serde_json::from_value(json!({})).unwrap();
    let default = Kill::default();
    assert_eq!(signal_arguments(42, &empty), signal_arguments(42, &default));
    assert_eq!(
        signal_arguments(42, &default),
        ["signal", "42", "term", "0", "2000"]
    );
    assert!(!default.root);
    let tree: Kill =
        serde_json::from_value(json!({"signal": "kill", "tree": true, "wait_ms": 0})).unwrap();
    assert_eq!(
        signal_arguments(7, &tree),
        ["signal", "7", "kill", "1", "0"]
    );
    assert!(serde_json::from_value::<Kill>(json!({"signal": "stop"})).is_err());
    assert!(serde_json::from_value::<Kill>(json!({"pid": 7})).is_err());
}

#[test]
fn pid_one_and_out_of_range_signals_are_refused() {
    let options = Kill::default();
    let refused = options.validate(1).unwrap_err().to_string();
    assert!(
        refused.contains("stop or restart the container"),
        "{refused}"
    );
    assert!(options.validate(0).is_err());
    assert!(options.validate(PID_MAX + 1).is_err());
    assert!(options.validate(2).is_ok());
    let long = Kill {
        wait_ms: 10_001,
        ..Kill::default()
    };
    assert!(long.validate(2).is_err());
}

#[test]
fn the_helpers_answers_read_as_their_statuses() {
    let options = Kill::default();
    let missing = signal_answer(5, &options, json!({"error": "missing"})).unwrap_err();
    assert_eq!(missing.0, StatusCode::NOT_FOUND);
    assert!(missing.1.contains("No process 5"), "{}", missing.1);
    let refused =
        signal_answer(5, &options, json!({"error": "permission", "user": "root"})).unwrap_err();
    assert_eq!(refused.0, StatusCode::CONFLICT);
    assert!(refused.1.contains("runs as root"), "{}", refused.1);
    assert!(refused.1.contains("root=true"), "{}", refused.1);
    let done = signal_answer(
        5,
        &options,
        json!({"signalled": [5, 6], "ended": [5], "running": [6], "refused": [9]}),
    )
    .unwrap();
    assert_eq!(done.pid, 5);
    assert_eq!(done.signal, SignalName::Term);
    assert_eq!(done.signalled, [5, 6]);
    assert_eq!(done.ended, [5]);
    assert_eq!(done.running, [6]);
    assert_eq!(done.refused, [9]);
    assert!(signal_answer(5, &options, json!({"signalled": "all"})).is_err());
}

#[test]
fn the_helpers_list_reads_as_a_process_list() {
    let fixture: Value = serde_json::from_str(include_str!("../tests/process-list.json")).unwrap();
    let parsed: ProcessList = serde_json::from_value(fixture.clone()).unwrap();
    assert_eq!(parsed.processes.len(), 3);
    assert_eq!(
        parsed
            .processes
            .iter()
            .map(|process| process.role)
            .collect::<Vec<_>>(),
        [Role::Program, Role::Desktop, Role::Thoric]
    );
    // Every field the helper writes is one of these types', and back again.
    assert_eq!(serde_json::to_value(&parsed).unwrap(), fixture);
    let mut extra = fixture.clone();
    extra["processes"][0]["unknown"] = json!(1);
    assert!(serde_json::from_value::<ProcessList>(extra).is_err());
}

#[tokio::test]
async fn bad_requests_are_refused_before_docker() {
    let app = app(false);
    let bad = list(json!({"limit": 0})).unwrap();
    let refused = list_processes(&app, "missing", &bad).await.unwrap_err();
    assert_eq!(refused.0, StatusCode::BAD_REQUEST);
    let refused = kill_process(&app, "missing", 1, &Kill::default())
        .await
        .unwrap_err();
    assert_eq!(refused.0, StatusCode::BAD_REQUEST);
    // A good request for a container that does not exist goes no further than its record.
    let options = list(json!({})).unwrap();
    let missing = list_processes(&app, "missing", &options).await.unwrap_err();
    assert_eq!(missing.0, StatusCode::NOT_FOUND);
}
