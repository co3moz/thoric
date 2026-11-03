//! Pins down the request log: a line for every request with its method, path, status, latency
//! and ID, at debug only for a timer's successful reads, an ID the answer returns, a line of its
//! own in the request's span for a server error, and never a query, a transfer link's token or a
//! desktop service's own path. The tests' own subscriber also sees a line that another test's
//! thread reached first.
use super::*;
use crate::test_support::capture;
use axum::body::Body;
use tower::ServiceExt;

/// Answer `GET uri` with `id` as the client's `X-Request-Id`, and return the answer's ID and the
/// request's line.
async fn logged(uri: &str, id: Option<&str>) -> (String, String) {
    let (answered, log) = log_of(uri, id).await;
    let line = log
        .lines()
        .find(|line| line.contains(": answered "))
        .unwrap_or_else(|| panic!("no line for {uri} in {log:?}"))
        .to_owned();
    (answered, line)
}

/// Answer `GET uri` with `id` as the client's `X-Request-Id`, and return the answer's ID and all
/// the log wrote meanwhile.
async fn log_of(uri: &str, id: Option<&str>) -> (String, String) {
    // A test runs on one thread, which the request does not leave.
    let (written, _logging) = capture();
    let mut request = axum::http::Request::get(uri).header("host", "localhost:8080");
    if let Some(id) = id {
        request = request.header("x-request-id", id);
    }
    let response = crate::router(crate::test_support::app(false))
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let answered = response.headers()[&REQUEST_ID].to_str().unwrap().to_owned();
    let log = written.text();
    (answered, log)
}

#[test]
fn a_line_first_logged_on_another_thread_still_reaches_the_test() {
    let (written, _logging) = capture();
    fn line() {
        tracing::info!("first logged elsewhere");
    }
    // Another test's thread, which has no subscriber, reaches the line first.
    std::thread::spawn(line).join().unwrap();
    line();
    let log = written.text();
    assert!(log.contains("first logged elsewhere"), "{log:?}");
}

#[tokio::test]
async fn every_request_has_a_line_with_its_id() {
    let (id, line) = logged("/api/session", None).await;
    assert!(uuid::Uuid::parse_str(&id).is_ok(), "{id}");
    assert!(line.contains(" INFO "), "{line}");
    assert!(line.contains("method=GET"), "{line}");
    assert!(line.contains("path=/api/session"), "{line}");
    assert!(line.contains(&format!("request_id={id}")), "{line}");
    assert!(line.contains("status=200"), "{line}");
    assert!(line.contains("latency_ms="), "{line}");

    // A client's own ID is kept, one that could forge a field is replaced.
    let (id, line) = logged("/api/session", Some("dashboard-7f.3:1")).await;
    assert_eq!(id, "dashboard-7f.3:1");
    assert!(line.contains("request_id=dashboard-7f.3:1"), "{line}");
    let (id, line) = logged("/api/session", Some("x status=200")).await;
    assert!(uuid::Uuid::parse_str(&id).is_ok(), "{id}");
    assert!(!line.contains("request_id=x"), "{line}");
}

#[tokio::test]
async fn a_timer_s_reads_are_logged_at_debug_while_they_succeed() {
    let (_, line) = logged("/healthz", None).await;
    assert!(line.contains("DEBUG"), "{line}");
    assert!(line.contains("status=200"), "{line}");
    // Without Docker the manager is not ready, which is worth a line at info.
    let (_, line) = logged("/readyz", None).await;
    assert!(line.contains(" INFO "), "{line}");
    assert!(line.contains("status=503"), "{line}");
}

#[tokio::test]
async fn a_server_error_has_a_line_of_its_own_in_the_requests_span() {
    // Without Docker the manager is not ready: the trace layer adds an error line to the request's
    // own, and both carry its ID.
    let (id, log) = log_of("/readyz", None).await;
    let failed = log
        .lines()
        .find(|line| line.contains("response failed"))
        .unwrap_or_else(|| panic!("no failure line in {log:?}"));
    assert!(failed.contains(" ERROR "), "{failed}");
    assert!(failed.contains(&format!("request_id={id}")), "{failed}");
    assert!(failed.contains("503"), "{failed}");
    // A success has the request's line alone.
    let (_, log) = log_of("/api/session", None).await;
    let lines: Vec<&str> = log
        .lines()
        .filter(|line| line.contains("request{"))
        .collect();
    assert_eq!(lines.len(), 1, "{log}");
}

#[tokio::test]
async fn a_line_never_holds_a_credential_or_a_query() {
    let token = "a".repeat(64);
    let (_, line) = logged(&format!("/api/transfers/{token}?size=5"), None).await;
    assert!(line.contains("path=/api/transfers/<token>"), "{line}");
    assert!(!line.contains(&token) && !line.contains("size"), "{line}");

    let (_, line) = logged("/api/containers/e/ports/8888/lab/tree?token=secret", None).await;
    assert!(
        line.contains("path=/api/containers/e/ports/8888/<path>"),
        "{line}"
    );
    assert!(!line.contains("lab") && !line.contains("secret"), "{line}");

    let (_, line) = logged(
        "/api/containers/e/files/download?path=/home/desktop/notes.txt",
        None,
    )
    .await;
    assert!(
        line.contains("path=/api/containers/e/files/download "),
        "{line}"
    );
    assert!(!line.contains("notes"), "{line}");
}

#[test]
fn paths_are_recorded_without_what_may_be_secret() {
    let path = |text: &str| path(&text.parse::<Uri>().unwrap());
    assert_eq!(path("/api/transfers/abc?size=5"), "/api/transfers/<token>");
    assert_eq!(
        path("/api/containers/e/ports/80"),
        "/api/containers/e/ports/80"
    );
    assert_eq!(
        path("/api/containers/e/ports/80/"),
        "/api/containers/e/ports/80/"
    );
    assert_eq!(
        path("/api/containers/e/ports/80/a/b?c=d"),
        "/api/containers/e/ports/80/<path>"
    );
    assert_eq!(path("/api/containers/e/ports"), "/api/containers/e/ports");
    assert_eq!(path("/api/jobs/j?x=1"), "/api/jobs/j");
    assert_eq!(path("/mcp/agents"), "/mcp/agents");
}

#[test]
fn only_a_timer_s_reads_are_repeated() {
    for (method, path) in [
        (Method::GET, "/healthz"),
        (Method::HEAD, "/readyz"),
        (Method::GET, "/metrics"),
        (Method::GET, "/api/usage"),
        (Method::GET, "/api/containers/e/usage"),
        (Method::GET, "/api/containers/e/screenshot"),
        (Method::GET, "/api/jobs/j"),
        (Method::GET, "/api/activity"),
        (Method::GET, "/assets/index-3f2a.js"),
    ] {
        assert!(repeated(&method, path), "{method} {path}");
    }
    for (method, path) in [
        (Method::POST, "/api/containers/e/screenshots"),
        (Method::DELETE, "/api/jobs/j"),
        (Method::POST, "/api/jobs/j/retry"),
        (Method::GET, "/api/jobs/j/request"),
        (Method::GET, "/api/containers"),
        (Method::GET, "/api/containers/e/vnc"),
        (Method::GET, "/api/events"),
        (Method::GET, "/"),
        (Method::POST, "/mcp"),
    ] {
        assert!(!repeated(&method, path), "{method} {path}");
    }
}

#[test]
fn only_a_plain_id_is_logged_as_sent() {
    let id = |text: &str| loggable(&HeaderValue::from_str(text).unwrap());
    assert!(id("0b7c9a3e-1f2d-4c5b-8a6e-9d0f1e2a3b4c"));
    assert!(id(&"a".repeat(ID_LIMIT)));
    assert!(!id(&"a".repeat(ID_LIMIT + 1)));
    assert!(!id(""));
    for forged in ["a b", "a=b", "a\"b", "a\tb", "a,b"] {
        assert!(!id(forged), "{forged}");
    }
}

#[test]
fn the_api_guide_explains_the_request_id() {
    let api = include_str!("../../../docs/API.md");
    assert!(
        api.contains("`X-Request-Id`"),
        "docs/API.md must explain it"
    );
    assert!(api.contains(&format!("up to {ID_LIMIT} letters")));
}
