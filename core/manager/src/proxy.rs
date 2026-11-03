//! A desktop's own HTTP and WebSocket services, reached through the manager.
//!
//! A desktop publishes no ports; the manager reaches it on its private network, as it does for
//! VNC. `/api/containers/{id}/ports/{port}/...` forwards a request to that address and one of the
//! ports the container's settings list, so an agent can use a program inside, such as the MCP
//! server of an editor, a test runner or a debugger, without a job for every request.
//!
//! Whatever the desktop serves appears on the dashboard's origin, and a desktop browses sites
//! nobody vouches for. A response that ran script there with the dashboard's cookie could take
//! over the manager. So the proxy:
//! - accepts the MCP bearer token and never the dashboard cookie, so no browser opens it signed
//!   in, and it is off whenever MCP is;
//! - strips the token, the dashboard cookie and `Origin` before forwarding: the desktop never sees
//!   a credential of the manager, and its service sees a local client;
//! - answers with `Content-Security-Policy: sandbox` and `nosniff`, so a browser that renders a
//!   response anyway, as one can with `MCP_AUTH_MODE=none`, runs no script and gives it an opaque
//!   origin, and drops the response headers that act on the dashboard's origin: cookies and
//!   `Clear-Site-Data`.
use crate::{
    App, docker,
    error::{Error, Result},
    mcp,
};
use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Version, header},
    response::{IntoResponse, Response},
};
use hyper_util::rt::TokioIo;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::net::TcpStream;

/// A desktop service that does not take a connection in this time is not there. A request that
/// was taken may take as long as the service needs, and a stream stays open while both ends do.
const CONNECT_LIMIT: Duration = Duration::from_secs(10);
/// Headers about one connection, which a proxy does not pass on (RFC 9110, 7.6.1).
const HOP_BY_HOP: [&str; 9] = [
    "connection",
    "keep-alive",
    "proxy-connection",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// A request's place under `/api/containers/{id}/ports/{port}`, as the client sent the path.
struct Route<'a> {
    id: &'a str,
    port: &'a str,
    /// The prefix up to the port, which a redirect inside the service is kept under.
    base: &'a str,
    /// The path the service sees: what follows the port, or `/`.
    path: &'a str,
}

fn route(path: &str) -> Option<Route<'_>> {
    let rest = path.strip_prefix("/api/containers/")?;
    let (id, rest) = rest.split_once('/')?;
    let rest = rest.strip_prefix("ports/")?;
    let (port, inner) = match rest.find('/') {
        Some(at) => rest.split_at(at),
        None => (rest, ""),
    };
    let base = &path[..path.len() - inner.len()];
    Some(Route {
        id,
        port,
        base,
        path: if inner.is_empty() { "/" } else { inner },
    })
}

/// `/api/containers/{id}/ports/{port}/{*path}`, any method.
pub async fn proxy(State(app): State<Arc<App>>, request: Request) -> Response {
    if let Some(rejection) = mcp::reject(&app, &request) {
        return rejection;
    }
    // Owned: a request's body is not Sync, so no borrow of it may wait on Docker.
    let path = request.uri().path().to_owned();
    let target = async {
        let route = route(&path).ok_or_else(crate::error::missing)?;
        let port = route
            .port
            .parse::<u16>()
            .map_err(|_| Error(StatusCode::BAD_REQUEST, "Invalid port".into()))?;
        Ok::<_, Error>((open(&app, route.id, port).await?, port))
    };
    match target.await {
        Ok((address, port)) => {
            // A service of the desktop in use is the desktop in use, until its answer has been
            // sent or a WebSocket it keeps open closes.
            let using = route(&path).map(|route| app.lifecycle.open(route.id));
            forward(address, port, request, using).await
        }
        Err(error) => error.into_response(),
    }
}

/// The address of a port the container's settings open, while the container runs.
async fn open(app: &App, id: &str, port: u16) -> Result<SocketAddr> {
    let env = app.container(id)?;
    if !env.spec.ports.contains(&port) {
        return Err(Error(
            StatusCode::FORBIDDEN,
            format!("Port {port} is not proxied for this container; add it to its ports setting"),
        ));
    }
    let info = app.owned(&env).await?;
    if info["State"]["Running"] != true {
        return Err(Error(
            StatusCode::CONFLICT,
            "Start the container before reaching its ports".into(),
        ));
    }
    Ok(SocketAddr::from((docker::private_ip(&env, &info)?, port)))
}

/// Send `request` to the service at `address` and stream its answer back as it arrives. When the
/// service switches protocols, as a WebSocket handshake does, the two connections are joined.
/// `using` lasts until the answer has been sent or the joined connections close.
pub(crate) async fn forward(
    address: SocketAddr,
    port: u16,
    mut request: Request,
    mut using: Option<crate::lifecycle::InUse>,
) -> Response {
    let Some(route) = route(request.uri().path()) else {
        return crate::error::missing().into_response();
    };
    let base = route.base.to_owned();
    let mut target = route.path.to_owned();
    if let Some(query) = request.uri().query() {
        target = format!("{target}?{query}");
    }
    let mcp = route.path.ends_with("/mcp");
    let protocol = upgrade_of(request.headers());
    let client = protocol.is_some().then(|| hyper::upgrade::on(&mut request));
    let (parts, body) = request.into_parts();
    let head = parts.method == Method::HEAD;
    let mut headers = parts.headers;
    without_hop_by_hop(&mut headers);
    for name in [header::AUTHORIZATION, header::ORIGIN, header::EXPECT] {
        headers.remove(name);
    }
    without_session_cookie(&mut headers);
    // The service may check Host against DNS rebinding; it is reached as a local one.
    headers.insert(
        header::HOST,
        HeaderValue::from_str(&format!("localhost:{port}")).expect("a port is a valid host"),
    );
    if let Some(protocol) = protocol {
        headers.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
        headers.insert(header::UPGRADE, protocol);
    }
    let mut upstream = match hyper::Request::builder()
        .method(parts.method)
        .uri(target)
        .version(Version::HTTP_11)
        .body(body)
    {
        Ok(upstream) => upstream,
        Err(_) => return Error(StatusCode::BAD_REQUEST, "Invalid request".into()).into_response(),
    };
    *upstream.headers_mut() = headers;
    let mut response = match send(address, upstream).await {
        Ok(response) => response,
        Err(message) => {
            return Error(
                StatusCode::BAD_GATEWAY,
                format!("Port {port} of the desktop: {message}"),
            )
            .into_response();
        }
    };
    let switched = response.status() == StatusCode::SWITCHING_PROTOCOLS;
    if switched {
        let Some(client) = client else {
            return Error(
                StatusCode::BAD_GATEWAY,
                format!("Port {port} of the desktop switched protocols unasked"),
            )
            .into_response();
        };
        let service = hyper::upgrade::on(&mut response);
        let joined = using.take();
        tokio::spawn(async move {
            let _using = joined;
            match tokio::try_join!(client, service) {
                Ok((client, service)) => {
                    let _ = tokio::io::copy_bidirectional(
                        &mut TokioIo::new(client),
                        &mut TokioIo::new(service),
                    )
                    .await;
                }
                Err(error) => tracing::debug!("Port proxy upgrade: {error}"),
            }
        });
    }
    let (mut parts, body) = response.into_parts();
    if !switched {
        without_hop_by_hop(&mut parts.headers);
    }
    answered(&mut parts.headers, &base);
    let status = parts.status;
    let response = Response::from_parts(
        parts,
        if switched {
            Body::empty()
        } else {
            Body::new(body)
        },
    );
    // A Streamable HTTP MCP server is reached at a path ending in /mcp, which some Windows
    // security software mangles on loopback unless the reply is chunked; see mcp::chunked.
    let bodiless = head
        || status.is_informational()
        || status == StatusCode::NO_CONTENT
        || status == StatusCode::NOT_MODIFIED;
    let response = if mcp && !bodiless {
        mcp::chunked(response)
    } else {
        response
    };
    match using {
        Some(using) => crate::lifecycle::lasting(response, using),
        None => response,
    }
}

/// One request on a new connection to the service.
async fn send(
    address: SocketAddr,
    request: hyper::Request<Body>,
) -> std::result::Result<hyper::Response<hyper::body::Incoming>, String> {
    let stream = tokio::time::timeout(CONNECT_LIMIT, TcpStream::connect(address))
        .await
        .map_err(|_| "nothing answered within 10 seconds".to_owned())?
        .map_err(|error| format!("nothing answers ({error})"))?;
    let _ = stream.set_nodelay(true);
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|error| error.to_string())?;
    tokio::spawn(async move {
        if let Err(error) = connection.with_upgrades().await {
            tracing::debug!("Port proxy connection: {error}");
        }
    });
    sender
        .send_request(request)
        .await
        .map_err(|error| format!("no answer ({error})"))
}

/// The protocol a request asks to switch to, such as `websocket`.
fn upgrade_of(headers: &HeaderMap) -> Option<HeaderValue> {
    headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
        .then(|| headers.get(header::UPGRADE).cloned())
        .flatten()
}

fn without_hop_by_hop(headers: &mut HeaderMap) {
    let named = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect::<Vec<_>>();
    for name in named {
        headers.remove(name);
    }
    for name in HOP_BY_HOP {
        headers.remove(name);
    }
}

/// Keep the cookies a client sends for the service, but never the dashboard's session.
fn without_session_cookie(headers: &mut HeaderMap) {
    let kept = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .map(str::trim)
        .filter(|pair| !pair.is_empty() && !pair.starts_with("thoric_session="))
        .collect::<Vec<_>>()
        .join("; ");
    headers.remove(header::COOKIE);
    if let Ok(value) = HeaderValue::from_str(&kept)
        && !kept.is_empty()
    {
        headers.insert(header::COOKIE, value);
    }
}

/// A service's response headers as the dashboard's origin may carry them.
fn answered(headers: &mut HeaderMap, base: &str) {
    headers.remove(header::SET_COOKIE);
    headers.remove("clear-site-data");
    // A redirect to a path of the service stays under the proxy.
    if let Some(location) = headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .filter(|location| location.starts_with('/') && !location.starts_with("//"))
        .and_then(|location| HeaderValue::from_str(&format!("{base}{location}")).ok())
    {
        headers.insert(header::LOCATION, location);
    }
    headers.append(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
}

#[cfg(test)]
#[path = "proxy_tests.rs"]
mod tests;
