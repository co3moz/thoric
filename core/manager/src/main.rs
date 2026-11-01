mod activity;
mod api;
mod apps;
mod archives;
mod audio;
mod auth;
mod backup;
mod capture;
mod catalog;
mod compose;
mod config;
mod db;
mod dependencies;
mod desktop_input;
mod desktop_limits;
mod desktop_mcp;
mod desktop_ready;
mod docker;
mod error;
mod events;
mod exec;
mod files;
mod folder_archive;
mod gamepad;
mod gpu;
mod groups;
mod health;
mod human;
mod icons;
mod identity;
mod image_removal;
mod jobs;
mod layers;
mod mcp;
mod media;
mod model;
mod mount_parents;
mod ocr;
mod openapi;
mod outbound;
mod package_requests;
mod process;
mod proxy;
mod pulls;
mod reconfigure;
mod recordings;
mod releases;
mod reported;
mod request_log;
mod resolution;
mod script_progress;
mod stages;
mod terminal;
mod transfers;
mod uploads;
mod usage;
mod volumes;
mod waits;
mod windows;

use axum::{
    Router,
    http::{HeaderValue, header},
    middleware,
    routing::{any, delete, get, post, put},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};
use tower_http::{services::ServeDir, trace::TraceLayer};

pub struct App {
    config: config::Config,
    db: db::Db,
    sessions: Mutex<HashMap<String, Instant>>,
    /// Failed sign-ins of the last minute, by the address they came from.
    login_attempts: auth::Limiter,
    /// The addresses of the bundled HTTPS proxy, whose forwarded address a sign-in counts by.
    https_proxy: auth::HttpsProxy,
    /// The manager's own addresses, of which those on a desktop's network are not answered.
    interfaces: auth::Interfaces,
    operations: jobs::ResourceLock,
    resource_locks: Mutex<HashMap<String, std::sync::Weak<tokio::sync::RwLock<()>>>>,
    session_events: tokio::sync::Notify,
    screenshots: tokio::sync::Semaphore,
    uploads: tokio::sync::Semaphore,
    desktop_inputs: tokio::sync::Semaphore,
    /// Image and volume archive exports and imports, which each hold a Docker save, load or tar.
    archives: Arc<tokio::sync::Semaphore>,
    /// Resource usage of the running containers over the last 30 minutes, in memory only.
    usage: usage::Usage,
    /// One-time transfer links issued over MCP and not used yet.
    transfers: transfers::Links,
    /// The layers and history of the listed images, read once per image ID.
    layers: layers::Cache,
    /// Desktop MCP servers, their connections and the services of the MCP endpoints.
    desktop_mcp: desktop_mcp::DesktopMcp,
    /// The newest release GitHub named, for the dashboard's update.
    releases: releases::Releases,
    /// What the signed-in dashboards hear as it happens, such as a job that ended.
    events: events::Hub,
    /// The jobs that have not ended, which a cancel reaches.
    cancels: jobs::cancel::Cancels,
    /// Agents' requests for a person, in memory only.
    human: human::Requests,
    /// The proxies inside the desktops that use one.
    outbound: outbound::Outbound,
}
/// The interface's content security policy. The templates page reads its list, the templates
/// and their icons from another host, which is the one exception to `'self'`. No page needs a
/// `<base>`, a plugin or a form that posts elsewhere, so an injected one does nothing.
fn content_security_policy(templates: Option<&str>) -> HeaderValue {
    let other = templates
        .map(|origin| format!(" {origin}"))
        .unwrap_or_default();
    HeaderValue::from_str(&format!("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:{other}; connect-src 'self'{other}; worker-src 'self' blob:; frame-ancestors 'self'; base-uri 'none'; form-action 'self'; object-src 'none'"))
        .unwrap_or_else(|_| content_security_policy(None))
}
fn router(app: Arc<App>) -> Router {
    let csp = content_security_policy(app.config.templates_origin());
    // A GET that starts work, such as a backup, takes `auth::same_origin` too.
    let protected = Router::new()
        .route("/api/logout", post(auth::logout))
        .route("/api/settings", get(api::settings))
        .route(
            "/api/configuration",
            get(config::changes::configuration).post(config::changes::change),
        )
        .route("/api/system", get(api::system))
        .route("/api/update", get(releases::status).post(releases::start))
        .route("/api/update/check", post(releases::check))
        .route(
            "/api/backup",
            get(backup::export).layer(middleware::from_fn(auth::same_origin)),
        )
        .route("/api/backup/check", post(backup::check))
        .route(
            "/api/backup/restore",
            post(backup::restore).layer(tower_http::limit::RequestBodyLimitLayer::new(
                usize::try_from(app.config.archive_limit).unwrap_or(usize::MAX),
            )),
        )
        .route("/api/usage", get(usage::all))
        .route("/api/containers", get(api::containers).post(api::create))
        .route(
            "/api/containers/{id}",
            get(api::container).delete(api::remove),
        )
        .route("/api/containers/{id}/action", post(api::action))
        .route("/api/containers/{id}/retry", post(api::retry))
        .route("/api/containers/{id}/internet", post(api::internet))
        .route("/api/containers/{id}/settings", post(api::update_settings))
        .route("/api/containers/{id}/snapshot", post(api::snapshot))
        .route("/api/containers/{id}/exec", post(exec::execute))
        .route("/api/containers/{id}/files/read", post(files::read))
        .route("/api/containers/{id}/files/write", post(files::write))
        .route("/api/containers/{id}/files/list", post(files::list))
        .route(
            "/api/containers/{id}/files/mkdir",
            post(files::make_directory),
        )
        .route("/api/containers/{id}/files/move", post(files::move_))
        .route("/api/containers/{id}/files/delete", post(files::delete))
        .route(
            "/api/containers/{id}/files/download",
            get(files::download).layer(middleware::from_fn(auth::same_origin)),
        )
        .route("/api/containers/{id}/audio-capture", post(audio::capture))
        .route("/api/containers/{id}/audio-inject", post(audio::inject))
        .route(
            "/api/containers/{id}/terminals",
            get(terminal::list).post(terminal::open),
        )
        .route(
            "/api/containers/{id}/terminals/{terminal}/input",
            post(terminal::input),
        )
        .route(
            "/api/containers/{id}/terminals/{terminal}/output",
            get(terminal::output),
        )
        .route(
            "/api/containers/{id}/terminals/{terminal}/close",
            post(terminal::close),
        )
        .route("/api/containers/{id}/ui", get(windows::inspect))
        .route("/api/containers/{id}/gamepad", post(gamepad::input))
        .route("/api/containers/{id}/input", post(desktop_input::input))
        .route("/api/containers/{id}/act-and-observe", post(capture::act))
        .route("/api/containers/{id}/screenshots", post(capture::save))
        .route("/api/containers/{id}/find-text", post(ocr::find))
        .route(
            "/api/containers/{id}/applications",
            get(apps::list).post(apps::launch),
        )
        .route(
            "/api/containers/{id}/applications/{application}",
            get(apps::get),
        )
        .route(
            "/api/containers/{id}/applications/{application}/logs",
            get(apps::logs),
        )
        .route(
            "/api/containers/{id}/applications/{application}/stop",
            post(apps::stop),
        )
        .route(
            "/api/containers/{id}/applications/{application}/diagnostics",
            get(apps::diagnostics),
        )
        .route(
            "/api/containers/{id}/applications/{application}/metrics",
            get(apps::metrics),
        )
        .route("/api/containers/{id}/wait", post(waits::wait))
        .route(
            "/api/containers/{id}/recordings",
            get(recordings::list).post(recordings::start),
        )
        .route(
            "/api/containers/{id}/recordings/{recording}/stop",
            post(recordings::stop),
        )
        .route("/api/containers/{id}/artifacts", post(recordings::export))
        .route("/api/containers/{id}/reset", post(api::reset))
        .route("/api/containers/{id}/windows", get(windows::list))
        .route(
            "/api/containers/{id}/windows/{window}/focus",
            post(windows::focus),
        )
        .route(
            "/api/containers/{id}/windows/{window}/geometry",
            post(windows::geometry),
        )
        .route("/api/containers/{id}/logs", get(api::logs))
        .route("/api/containers/{id}/diagnostics", get(api::diagnostics))
        .route("/api/containers/{id}/usage", get(usage::container))
        .route("/api/containers/{id}/screenshot", get(api::screenshot))
        .route(
            "/api/containers/{id}/files",
            post(uploads::upload).layer(tower_http::limit::RequestBodyLimitLayer::new(
                app.config.upload_limit as usize,
            )),
        )
        .route("/api/containers/{id}/vnc", get(media::vnc))
        .route("/api/containers/{id}/audio/{direction}", get(media::audio))
        .route("/api/images", get(api::images).post(api::register_image))
        .route("/api/images/build", post(api::build))
        .route(
            "/api/images/import",
            post(archives::import).layer(tower_http::limit::RequestBodyLimitLayer::new(
                usize::try_from(app.config.archive_limit).unwrap_or(usize::MAX),
            )),
        )
        .route("/api/images/{id}", delete(image_removal::remove))
        .route("/api/images/{id}/pull", post(api::pull_catalog))
        .route("/api/images/{id}/build", post(api::build_catalog))
        .route(
            "/api/images/{id}/archive",
            get(archives::export).layer(middleware::from_fn(auth::same_origin)),
        )
        .route("/api/images/{id}/config", get(api::image_config))
        .route("/api/images/{id}/layers", get(layers::layers))
        .route(
            "/api/images/{id}/icon",
            get(icons::get).put(icons::set).delete(icons::remove),
        )
        .route(
            "/api/images/{id}/compose",
            get(compose::get).put(compose::set).delete(compose::remove),
        )
        .route(
            "/api/images/{id}/compose/base",
            post(compose::inherit::add_base),
        )
        .route("/api/volumes", get(api::volumes).post(api::create_volume))
        .route("/api/volumes/{name}", delete(volumes::remove))
        .route(
            "/api/volumes/import",
            post(volumes::import)
                .layer(tower_http::limit::RequestBodyLimitLayer::new(
                    usize::try_from(app.config.archive_limit).unwrap_or(usize::MAX),
                ))
                // This path wins over `{name}`, and a volume may be called `import` too.
                .delete(|app| volumes::remove(app, axum::extract::Path("import".into()))),
        )
        .route(
            "/api/volumes/{name}/archive",
            get(volumes::export).layer(middleware::from_fn(auth::same_origin)),
        )
        .route("/api/volumes/{name}/copy", post(volumes::copy))
        .route(
            "/api/volumes/{name}/extract",
            post(volumes::extract).layer(tower_http::limit::RequestBodyLimitLayer::new(
                usize::try_from(app.config.archive_limit).unwrap_or(usize::MAX),
            )),
        )
        .route("/api/volumes/{name}/files/list", post(volumes::list))
        .route("/api/volumes/{name}/files/read", post(volumes::read))
        .route("/api/volumes/{name}/files/write", post(volumes::write))
        .route(
            "/api/volumes/{name}/files/mkdir",
            post(volumes::make_directory),
        )
        .route("/api/volumes/{name}/files/move", post(volumes::move_))
        .route("/api/volumes/{name}/files/delete", post(volumes::delete))
        .route(
            "/api/volumes/{name}/files/download",
            get(volumes::download).layer(middleware::from_fn(auth::same_origin)),
        )
        .route(
            "/api/volumes/{name}/files",
            post(volumes::upload).layer(tower_http::limit::RequestBodyLimitLayer::new(
                app.config.upload_limit as usize,
            )),
        )
        .route("/api/mcp/servers", get(desktop_mcp::all_servers))
        .route("/api/mcp/scopes", get(desktop_mcp::scopes_list))
        .route("/api/mcp/inspect", get(desktop_mcp::inspect))
        .route(
            "/api/mcp/endpoints",
            get(desktop_mcp::endpoints).post(desktop_mcp::create_endpoint),
        )
        .route(
            "/api/mcp/endpoints/{name}",
            put(desktop_mcp::update_endpoint).delete(desktop_mcp::delete_endpoint),
        )
        .route(
            "/api/mcp/endpoints/{name}/token",
            post(desktop_mcp::token).delete(desktop_mcp::revoke_token),
        )
        .route(
            "/api/mcp/endpoints/{name}/tools",
            get(desktop_mcp::endpoint_tools),
        )
        .route(
            "/api/containers/{id}/mcp/servers",
            get(desktop_mcp::container_servers).post(desktop_mcp::register_server),
        )
        .route(
            "/api/containers/{id}/mcp/servers/{name}",
            delete(desktop_mcp::unregister_server),
        )
        .route(
            "/api/containers/{id}/mcp/servers/{name}/refresh",
            post(desktop_mcp::refresh_server),
        )
        .route("/api/jobs", get(api::jobs))
        .route("/api/jobs/{id}", get(api::job))
        .route("/api/jobs/{id}/retry", post(jobs::retry::retry))
        .route("/api/jobs/{id}/cancel", post(jobs::cancel::cancel))
        .route("/api/jobs/{id}/request", get(jobs::retry::request))
        .route("/api/events", get(events::stream))
        .route("/api/human-requests", get(human::list))
        .route("/api/human-requests/{id}/answer", post(human::answer))
        .route("/api/activity", get(activity::list))
        .route("/api/dependencies", get(dependencies::list))
        .route("/api/openapi.json", get(openapi::serve))
        .route_layer(middleware::from_fn_with_state(app.clone(), auth::protect));
    Router::new()
        .merge(protected)
        .merge(mcp::router(app.clone()))
        .route("/api/session", get(api::session))
        .route("/api/login", post(auth::login))
        // A transfer link authorizes itself, so it stays outside protect().
        .route(
            "/api/transfers/{token}",
            any(transfers::transfer).layer(tower_http::limit::RequestBodyLimitLayer::new(
                usize::try_from(app.config.archive_limit.max(app.config.upload_limit))
                    .unwrap_or(usize::MAX),
            )),
        )
        // An MCP endpoint takes its own token or the administrator's, never the dashboard's cookie.
        .route("/mcp/{name}", any(desktop_mcp::endpoint))
        // The MCP token opens a desktop's proxied ports, never the dashboard's cookie.
        .route("/api/containers/{id}/ports/{port}", any(proxy::proxy))
        .route("/api/containers/{id}/ports/{port}/", any(proxy::proxy))
        .route(
            "/api/containers/{id}/ports/{port}/{*path}",
            any(proxy::proxy),
        )
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(health::ready))
        .fallback_service(ServeDir::new(&app.config.web).append_index_html_on_directories(true))
        .layer(axum::extract::DefaultBodyLimit::max(1_100_000))
        .layer(middleware::from_fn_with_state(
            app.clone(),
            auth::check_host,
        ))
        .layer(middleware::from_fn_with_state(
            app.clone(),
            auth::check_network,
        ))
        .layer(middleware::from_fn(
            move |req: axum::extract::Request, next: middleware::Next| {
                let csp = csp.clone();
                async move {
                    let path = req.uri().path();
                    let api = path.starts_with("/api/") || path.starts_with("/mcp");
                    let proxied = path.starts_with("/api/containers/") && path.contains("/ports/");
                    // The interface build names every file under /assets after its content, so
                    // a found one never changes. Everything else, index.html above all, is
                    // revalidated on each load: after an upgrade a cached page would ask for
                    // assets that no longer exist.
                    let assets = path.starts_with("/assets/");
                    let mut res = next.run(req).await;
                    let found = res.status().is_success();
                    let h = res.headers_mut();
                    h.insert(
                        header::X_CONTENT_TYPE_OPTIONS,
                        HeaderValue::from_static("nosniff"),
                    );
                    h.insert(
                        header::REFERRER_POLICY,
                        HeaderValue::from_static("same-origin"),
                    );
                    // Appended: a proxied desktop service keeps its sandbox policy alongside this one.
                    h.append(header::CONTENT_SECURITY_POLICY, csp);
                    // An image icon names its version in the URL and says itself how long it
                    // keeps; what a desktop service says about caching is not trusted.
                    if proxied {
                        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                    } else if api {
                        h.entry(header::CACHE_CONTROL)
                            .or_insert(HeaderValue::from_static("no-store"));
                    } else if assets && found {
                        h.insert(
                            header::CACHE_CONTROL,
                            HeaderValue::from_static("public, max-age=31536000, immutable"),
                        );
                    } else {
                        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
                    }
                    res
                }
            },
        ))
        // The request's own span, which request_log made, also covers its body, and a server
        // error or a body that fails gets a line of its own.
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|_: &axum::extract::Request| tracing::Span::current())
                .on_request(())
                .on_response(()),
        )
        .layer(middleware::from_fn(request_log::log))
        .with_state(app)
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "thoric=info,tower_http=info".into()),
        )
        .init();
    let mut config = config::Config::load()?;
    for notice in &config.notices {
        tracing::warn!("{notice}");
    }
    // One look at the daemon serves the GPU routes and the desktops' log options.
    let docker_info = desktop_limits::docker_info(&config.docker_bin).await;
    config.gpu_host = gpu::Host::detect(&config, docker_info.as_ref());
    config.log_driver = desktop_limits::log_driver(docker_info.as_ref());
    gpu::log_routes(&config);
    desktop_limits::log_logging(&config);
    if config.mcp_token.is_none() && config.root_mcp_enabled {
        tracing::warn!(
            "MCP_AUTH_MODE=none: /mcp and the MCP endpoints accept requests without a token from any allowed host"
        );
    }
    tokio::fs::create_dir_all(&config.data).await?;
    uploads::cleanup_temporary_files(&config.data).await?;
    let db = db::Db::open(&config.data.join(db::FILE))?;
    // Finished jobs beyond the newest are history nobody reads, kept out of memory and the file.
    db.prune_jobs(jobs::KEPT_JOBS)?;
    // Interrupted work is explicit. Never silently retry a user script or remove a container.
    // An update or a settings change the WSL distribution runs replaces the manager on its way,
    // and goes on.
    let mut interrupted_builds = Vec::new();
    let mut updates = Vec::new();
    let mut settings_changes = Vec::new();
    for mut job in db.list::<model::Job>("job")? {
        if releases::resumable(&config.update_dir, &job) {
            updates.push(job);
        } else if config::changes::resumable(&config.update_dir, &job) {
            settings_changes.push(job);
        } else if job.status == "queued" || job.status == "running" {
            job.status = "interrupted".into();
            job.finished_at = Some(model::now());
            job.finish_stages(model::now());
            job.progress = None;
            db.put_job_logged(
                &job,
                "\nService restarted. The Docker operation may have continued; check resources before retrying.\n",
                false,
            )?;
            if job.kind == "build" {
                interrupted_builds.push(job.id);
            }
        }
    }
    compose::mark_interrupted(&db)?;
    let listen = config.listen.clone();
    let app = Arc::new(App {
        config,
        db,
        sessions: Mutex::new(HashMap::new()),
        login_attempts: Default::default(),
        https_proxy: Default::default(),
        interfaces: Default::default(),
        operations: Default::default(),
        resource_locks: Mutex::new(HashMap::new()),
        session_events: tokio::sync::Notify::new(),
        screenshots: tokio::sync::Semaphore::new(2),
        uploads: tokio::sync::Semaphore::new(2),
        desktop_inputs: tokio::sync::Semaphore::new(4),
        archives: Arc::new(tokio::sync::Semaphore::new(2)),
        usage: usage::Usage::default(),
        transfers: transfers::Links::default(),
        layers: layers::Cache::default(),
        desktop_mcp: desktop_mcp::DesktopMcp::default(),
        releases: releases::Releases::default(),
        events: events::Hub::default(),
        cancels: Default::default(),
        human: human::Requests::default(),
        outbound: outbound::Outbound::default(),
    });
    usage::spawn(app.clone());
    events::docker::spawn(app.clone());
    desktop_mcp::spawn(app.clone());
    releases::spawn(app.clone());
    outbound::spawn(app.clone());
    auth::follow_https_proxy(app.clone());
    auth::warn_exposure(app.clone());
    for job in updates {
        releases::resume(app.clone(), job);
    }
    for job in settings_changes {
        config::changes::resume(app.clone(), job);
    }
    let reconcile = app.clone();
    let cleanup = app.clone();
    tokio::spawn(async move {
        match cleanup.drop_orphan_tags().await {
            Ok(dropped) if !dropped.is_empty() => {
                tracing::info!(
                    "Removed image tags without a record: {}",
                    dropped.join(", ")
                );
            }
            Ok(_) => {}
            Err(error) => tracing::debug!("Orphan image tags: {error:#}"),
        }
        // A build that ended cleaned up after itself; only one the restart cut short did not.
        for id in interrupted_builds {
            if let Err(e) = cleanup.cleanup_build(&id).await {
                tracing::debug!("Interrupted build cleanup {id}: {e:#}");
            }
        }
    });
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(20));
        let mut reported = reported::Reported::default();
        loop {
            tick.tick().await;
            let result = reconcile_networks(&reconcile, &mut reported).await;
            reported.outcome(RECONCILIATION, &result);
        }
    });
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    tracing::info!("Thoric listening: {listen}");
    let closing = app.clone();
    let stop = async move {
        stop_requested().await;
        tracing::info!("Stopping: open requests have {} seconds", DRAIN.as_secs());
        // Event streams and waits for a person never end on their own, and shutdown waits for
        // every response.
        closing.events.close();
    };
    serve(listener, router(app), stop, DRAIN).await?;
    Ok(())
}

/// How long the open requests may take to finish once the manager is asked to stop. `docker
/// stop`, which `thoric update` and a settings change run too, kills it ten seconds after its
/// SIGTERM, and a long upload or an MCP stream would otherwise keep it until then.
const DRAIN: std::time::Duration = std::time::Duration::from_secs(5);

/// Resolves once the manager is asked to stop: Ctrl+C, or the SIGTERM that `docker stop` sends,
/// which tini passes on.
async fn stop_requested() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
                return;
            }
            Err(error) => tracing::warn!("SIGTERM cannot stop the manager gracefully: {error}"),
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

/// Serve `router` until `stop` resolves, then let the open requests finish for at most `drain`.
/// WebSocket and proxied connections, which the server does not wait for, end with the process.
async fn serve(
    listener: tokio::net::TcpListener,
    router: Router,
    stop: impl std::future::Future<Output = ()> + Send + 'static,
    drain: std::time::Duration,
) -> std::io::Result<()> {
    let (stopped, stopping) = tokio::sync::oneshot::channel();
    let server = axum::serve(
        listener,
        router.into_make_service_with_connect_info::<auth::Connection>(),
    )
    .with_graceful_shutdown(async move {
        stop.await;
        let _ = stopped.send(());
    });
    tokio::select! {
        served = std::future::IntoFuture::into_future(server) => served,
        () = async {
            match stopping.await {
                Ok(()) => tokio::time::sleep(drain).await,
                Err(_) => std::future::pending().await,
            }
        } => {
            tracing::warn!(
                "Requests still open {} seconds after the stop were cut off",
                drain.as_secs()
            );
            Ok(())
        }
    }
}
/// What the network reconciliation as a whole reports its failures as.
const RECONCILIATION: &str = "Docker reconciliation";
fn container_reconciliation(id: &str) -> String {
    format!("Container {id} network reconciliation")
}
fn container_pids_limit(id: &str) -> String {
    format!("Container {id} pids limit")
}
async fn reconcile_networks(app: &App, reported: &mut reported::Reported) -> anyhow::Result<()> {
    let Some(manager) = &app.config.manager else {
        return Ok(());
    };
    let m = app.inspect("container", manager).await?;
    if m["Config"]["Labels"]["thoric.manager"] != "true" {
        anyhow::bail!("Missing manager label");
    }
    let containers = app.db.list::<model::Container>(model::CONTAINERS)?;
    // A deleted container's failure goes with it.
    reported.retain(|subject| {
        subject == RECONCILIATION
            || containers.iter().any(|env| {
                subject == container_reconciliation(&env.id)
                    || subject == container_pids_limit(&env.id)
            })
    });
    for mut env in containers {
        let lock = app.resource_lock(&format!("container:{}", env.id));
        let Ok(_guard) = lock.try_lock() else {
            continue;
        };
        // Reload after taking the lock; a queued mutation may have removed/updated this record.
        env = match app.container(&env.id) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let result = async {
            let info = app.owned(&env).await?;
            // Also a desktop created before the installation's pids limit, or before it changed.
            // Its networks are checked either way, and its failure is reported on its own.
            let pids = app.keep_pids_limit(&info).await;
            reported.outcome(&container_pids_limit(&env.id), &pids);
            let network = app.inspect("network", &env.private_network).await?;
            App::check_network(&env, &network, true)?;
            let egress = app.inspect("network", &env.egress_network).await?;
            App::check_network(&env, &egress, false)?;
            if m["NetworkSettings"]["Networks"]
                .get(&env.private_network)
                .is_none()
            {
                app.docker(&[
                    "network",
                    "connect",
                    network["Id"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing network ID"))?,
                    m["Id"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing manager ID"))?,
                ])
                .await?;
            }
            let actual = info["NetworkSettings"]["Networks"]
                .get(&env.egress_network)
                .is_some();
            if env.spec.internet != actual {
                env.spec.internet = actual;
                app.db.put(model::CONTAINERS, &env.id, &env)?;
            }
            // Also a group network removed by hand while the manager was down.
            app.join_groups(&env, &info).await?;
            anyhow::Ok(())
        }
        .await;
        reported.outcome(&container_reconciliation(&env.id), &result);
    }
    Ok(())
}

#[cfg(test)]
mod test_support;
#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
