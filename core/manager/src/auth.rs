use crate::{
    App,
    error::{Error, Result},
};
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::{
    Json,
    extract::{ConnectInfo, FromRequestParts, Request, State, connect_info::Connected},
    http::{HeaderMap, StatusCode, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
    serve::IncomingStream,
};
use serde::Deserialize;
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::net::TcpListener;

mod exposure;
mod forwarded;
mod interfaces;
mod limiter;
pub(crate) use exposure::{exposure, warn as warn_exposure};
pub(crate) use forwarded::{HttpsProxy, spawn as follow_https_proxy};
pub(crate) use interfaces::Interfaces;
pub(crate) use limiter::{Limiter, Verdict};

/// Both ends of a connection: the address it came from, and the manager's own address it reached,
/// which tells the network it came on.
#[derive(Clone, Copy, Debug)]
pub struct Connection {
    pub peer: SocketAddr,
    pub local: Option<SocketAddr>,
}
impl Connected<IncomingStream<'_, TcpListener>> for Connection {
    fn connect_info(stream: IncomingStream<'_, TcpListener>) -> Self {
        Self {
            peer: *stream.remote_addr(),
            local: stream.io().local_addr().ok(),
        }
    }
}

/// The address a connection came from, when the listener records one. A request made without a
/// connection, as the tests do, has none, and such requests share one count of failed sign-ins.
pub struct Peer(pub Option<IpAddr>);
impl<S: Send + Sync> FromRequestParts<S> for Peer {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(
        parts: &mut Parts,
        _: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        Ok(Peer(
            parts
                .extensions
                .get::<ConnectInfo<Connection>>()
                .map(|info| info.0.peer.ip()),
        ))
    }
}

pub fn cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|p| p.trim().strip_prefix("thoric_session=").map(str::to_owned))
}
/// What a long transfer answers to once it has begun. A dashboard request's session cookie ends
/// it when the session signs out or expires. A one-time transfer link was checked when its
/// request arrived and belongs to no session, so nothing revokes it. No request extracts `Link`;
/// only the transfer route passes it.
#[derive(Clone, Debug, PartialEq)]
pub enum Session {
    Cookie(Option<String>),
    Link,
}
impl From<Option<String>> for Session {
    fn from(token: Option<String>) -> Self {
        Self::Cookie(token)
    }
}
impl<S: Send + Sync> FromRequestParts<S> for Session {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(
        parts: &mut Parts,
        _: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        Ok(Self::Cookie(cookie(&parts.headers)))
    }
}
pub fn origin_matches(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|h| h.to_str().ok()) else {
        return false;
    };
    let Some(host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) else {
        return false;
    };
    let Ok(uri) = origin.parse::<axum::http::Uri>() else {
        return false;
    };
    matches!(uri.scheme_str(), Some("http" | "https"))
        && uri.path() == "/"
        && uri.query().is_none()
        && uri
            .authority()
            .is_some_and(|a| a.as_str().eq_ignore_ascii_case(host))
}
/// The address a request counts against in a limiter: the connection's, or behind the bundled
/// HTTPS proxy the one it forwards.
pub(crate) fn client(app: &App, peer: Option<IpAddr>, headers: &HeaderMap) -> Option<IpAddr> {
    forwarded::client(peer, headers, &app.https_proxy)
}
impl App {
    pub fn host_allowed(&self, headers: &HeaderMap) -> bool {
        let Some(host) = headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.parse::<axum::http::uri::Authority>().ok())
        else {
            return false;
        };
        if host.as_str().contains('@') {
            return false;
        }
        let name = host
            .host()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase();
        if name == "localhost" || self.config.host_allowed(&name) {
            return true;
        }
        match name.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V4(ip)) => {
                ip.is_loopback() || ip.is_private() || ip.is_link_local()
            }
            Ok(std::net::IpAddr::V6(ip)) => {
                ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()
            }
            _ => false,
        }
    }
    /// Resolves once `session` no longer authorizes the work it started: at once for a request
    /// without a session cookie, never while sign-in is off or for a transfer link.
    pub async fn session_revoked(&self, session: impl Into<Session>) {
        let token = match session.into() {
            _ if !self.config.auth => None,
            Session::Link => None,
            Session::Cookie(None) => return,
            Session::Cookie(Some(token)) => Some(token),
        };
        let Some(token) = token else {
            std::future::pending::<()>().await;
            return;
        };
        loop {
            let notified = self.session_events.notified();
            tokio::pin!(notified);
            // Register before checking: logout cannot fall into a check/wait gap.
            notified.as_mut().enable();
            let expiry = self.sessions.lock().unwrap().get(&token).copied();
            let Some(expiry) = expiry.filter(|t| *t > Instant::now()) else {
                return;
            };
            tokio::select! { _=tokio::time::sleep_until(expiry.into())=>return, _=&mut notified=>{} }
        }
    }
    pub fn authenticated(&self, headers: &HeaderMap) -> bool {
        if !self.config.auth {
            return true;
        }
        let Some(token) = cookie(headers) else {
            return false;
        };
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|_, expiry| *expiry > Instant::now());
        sessions.contains_key(&token)
    }
}
/// Refuse a connection that reached the manager on a desktop's network ([`interfaces`]), before
/// anything else looks at the request. A request made without a connection, as the tests do,
/// names no network.
pub async fn check_network(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    let local = req
        .extensions()
        .get::<ConnectInfo<Connection>>()
        .and_then(|info| info.0.local);
    if let Some(local) = local {
        match app.interfaces.answers(&app, local.ip()).await {
            Ok(true) => {}
            Ok(false) => {
                return Error(
                    StatusCode::FORBIDDEN,
                    "The manager does not answer on a desktop's network".into(),
                )
                .into_response();
            }
            Err(error) => return Error::from(error).into_response(),
        }
    }
    next.run(req).await
}
pub async fn check_host(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    if !app.host_allowed(req.headers()) {
        return Error(
            StatusCode::FORBIDDEN,
            "Host not allowed. Add your hostname to ALLOWED_HOSTS.".into(),
        )
        .into_response();
    }
    next.run(req).await
}
pub async fn protect(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    if !app.authenticated(req.headers()) {
        return Error(StatusCode::UNAUTHORIZED, "Sign in required".into()).into_response();
    }
    if !matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    ) && (req
        .headers()
        .get("x-thoric-request")
        .is_none_or(|v| v != "1")
        || (req.headers().contains_key(header::ORIGIN) && !origin_matches(req.headers())))
    {
        return Error(StatusCode::FORBIDDEN, "Request origin rejected".into()).into_response();
    }
    next.run(req).await
}
/// Refuse a GET that starts work, such as a backup, an export or a folder download, when the
/// browser says another site sent it, as a page's `<img>` or link would. A GET needs no
/// `X-Thoric-Request`, and with sign-in off, or from another port of the same host, which is the
/// same site, no cookie the browser holds back either. The dashboard's own requests and downloads
/// are `same-origin`, an address opened by hand is `none`, and a client that is no browser, such as
/// curl with a transfer link, sends no `Sec-Fetch-Site` at all.
pub async fn same_origin(req: Request, next: Next) -> Response {
    let site = req
        .headers()
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok());
    if matches!(site, Some("cross-site" | "same-site")) {
        return Error(
            StatusCode::FORBIDDEN,
            "Another site cannot start this; open it from the dashboard".into(),
        )
        .into_response();
    }
    next.run(req).await
}
#[derive(Deserialize, schemars::JsonSchema)]
pub struct Login {
    pub username: String,
    pub password: String,
}
pub async fn login(
    State(app): State<Arc<App>>,
    Peer(peer): Peer,
    headers: HeaderMap,
    Json(body): Json<Login>,
) -> Result<Response> {
    if headers.get("x-thoric-request").is_none_or(|v| v != "1")
        || (headers.contains_key(header::ORIGIN) && !origin_matches(&headers))
    {
        return Err(Error(
            StatusCode::FORBIDDEN,
            "Request origin rejected".into(),
        ));
    }
    if !app.config.auth {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    if body.username.len() > 256 || body.password.len() > 4096 {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            "Credentials are too long".into(),
        ));
    }
    let Some(hash) = app.config.password_hash.clone() else {
        return Err(Error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Sign-in is on without an administrator password; set ADMIN_PASSWORD".into(),
        ));
    };
    // Failures count per address, so one guessing neighbour does not lock the administrator out
    // from another machine. Behind the HTTPS proxy the address is the one it forwards.
    let client = forwarded::client(peer, &headers, &app.https_proxy);
    let shown = client.map_or_else(|| "an unknown address".to_owned(), |ip| ip.to_string());
    if let Verdict::Refused { first } = app.login_attempts.begin(client, Instant::now()) {
        if first {
            tracing::warn!(
                "Sign-ins from {shown} are refused for up to a minute after {} failures",
                limiter::ATTEMPTS
            );
        }
        return Err(Error(
            StatusCode::TOO_MANY_REQUESTS,
            "Try again in one minute".into(),
        ));
    }
    let valid_user = body.username == app.config.username;
    let valid = tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash).is_ok_and(|h| {
            Argon2::default()
                .verify_password(body.password.as_bytes(), &h)
                .is_ok()
        })
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))?;
    if !valid || !valid_user {
        // Never the name or the password that was tried: either may be the real one mistyped.
        tracing::warn!("Failed sign-in from {shown}");
        return Err(Error(
            StatusCode::UNAUTHORIZED,
            "Invalid username or password".into(),
        ));
    }
    app.login_attempts.succeeded(client);
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    if let Some(old) = cookie(&headers) {
        app.sessions.lock().unwrap().remove(&old);
        app.session_events.notify_waiters();
    }
    app.sessions
        .lock()
        .unwrap()
        .insert(token.clone(), Instant::now() + Duration::from_secs(43200));
    let secure = if app.config.secure_cookie {
        "; Secure"
    } else {
        ""
    };
    Ok((
        [(
            header::SET_COOKIE,
            format!(
                "thoric_session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=43200{secure}"
            ),
        )],
        StatusCode::NO_CONTENT,
    )
        .into_response())
}
pub async fn logout(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if let Some(token) = cookie(&headers) {
        app.sessions.lock().unwrap().remove(&token);
        app.session_events.notify_waiters();
    }
    (
        [(
            header::SET_COOKIE,
            "thoric_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0",
        )],
        StatusCode::NO_CONTENT,
    )
        .into_response()
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
