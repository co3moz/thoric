//! One log line for every request, with an ID the answer carries.
//!
//! The line is written at info when the answer starts, inside a span that names the method, the
//! path and the request ID, so whatever a handler logs carries them too. The ID is the client's
//! own `X-Request-Id` when it is safe to log, and a new UUID otherwise; the answer returns it, so
//! an error a dashboard or a script saw can be found in the log.
//!
//! Reads a page or a health check repeats on a timer are written at debug while they succeed, so
//! the log at info holds what people and agents do rather than thousands of timer ticks a day:
//! the health checks (Docker's every 20 seconds), a Prometheus scrape of `/metrics`, the usage
//! charts (every second), a job being followed (every two seconds), the Activity page (every
//! five), the desktop previews (every ten) and the interface's own files, which name their
//! content. Event streams, screens and other
//! sockets are one line each when they open, and lists the event stream keeps current are read
//! once a minute, so neither floods the log.
use crate::transfers;
use axum::{
    extract::Request,
    http::{HeaderName, HeaderValue, Method, Uri},
    middleware::Next,
    response::Response,
};
use std::time::Instant;
use tracing::Instrument;

/// The header that carries a request's ID both ways.
pub(crate) const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");
/// The longest client ID that is kept.
const ID_LIMIT: usize = 128;

/// The middleware that gives a request its ID, its span and its line.
pub(crate) async fn log(mut request: Request, next: Next) -> Response {
    let id = request
        .headers()
        .get(&REQUEST_ID)
        .filter(|id| loggable(id))
        .cloned()
        .unwrap_or_else(|| {
            HeaderValue::try_from(uuid::Uuid::new_v4().to_string())
                .expect("a UUID is a header value")
        });
    // Handlers and a desktop service the request reaches see the ID that was logged.
    request.headers_mut().insert(REQUEST_ID, id.clone());
    let method = request.method().clone();
    let quiet = repeated(&method, request.uri().path());
    let span = tracing::info_span!(
        "request",
        %method,
        path = %path(request.uri()),
        request_id = %id.to_str().unwrap_or_default(),
    );
    let started = Instant::now();
    let mut response = next.run(request).instrument(span.clone()).await;
    let status = response.status();
    let latency_ms = started.elapsed().as_millis() as u64;
    span.in_scope(|| {
        let status = status.as_u16();
        if quiet && status < 400 {
            tracing::debug!(status, latency_ms, "answered");
        } else {
            tracing::info!(status, latency_ms, "answered");
        }
    });
    response.headers_mut().insert(REQUEST_ID, id);
    response
}

/// Whether a client's ID can be logged as it is: up to [`ID_LIMIT`] letters, digits and `-_.:`,
/// so it can neither run on into the next field nor forge a line.
fn loggable(id: &HeaderValue) -> bool {
    let id = id.as_bytes();
    !id.is_empty()
        && id.len() <= ID_LIMIT
        && id
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(byte))
}

/// A request's path as the log records it. Never the query, which may carry a file's path or a
/// desktop service's token; never a transfer link's token, which is the credential itself; and of
/// a desktop service reached through the manager only the port, since what follows belongs to the
/// service and may hold a credential of its own.
pub(crate) fn path(uri: &Uri) -> String {
    let path = uri.path();
    if path.starts_with(transfers::ROUTE) {
        return format!("{}<token>", transfers::ROUTE);
    }
    match service_base(path) {
        Some(base) if path.len() > base.len() + 1 => format!("{base}/<path>"),
        _ => path.to_owned(),
    }
}

/// `/api/containers/{id}/ports/{port}` of a path under it.
fn service_base(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/api/containers/")?;
    let (_, rest) = rest.split_once('/')?;
    let rest = rest.strip_prefix("ports/")?;
    let inner = rest.find('/').map_or(0, |port| rest.len() - port);
    Some(&path[..path.len() - inner])
}

/// The reads a page or a health check repeats on a timer.
fn repeated(method: &Method, path: &str) -> bool {
    if !matches!(*method, Method::GET | Method::HEAD) {
        return false;
    }
    if matches!(
        path,
        "/healthz" | "/readyz" | "/metrics" | "/api/usage" | "/api/activity"
    ) || path.starts_with("/assets/")
    {
        return true;
    }
    if let Some(job) = path.strip_prefix("/api/jobs/") {
        return !job.contains('/');
    }
    path.strip_prefix("/api/containers/")
        .and_then(|rest| rest.split_once('/'))
        .is_some_and(|(_, rest)| matches!(rest, "usage" | "screenshot"))
}

#[cfg(test)]
#[path = "request_log_tests.rs"]
mod tests;
