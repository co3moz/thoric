//! A container's routes: the list, creation with the installation's defaults, start, stop and
//! restart, internet access, settings changes, deletion, retry and reset, and its logs,
//! screenshot and diagnostics. Most start a job under the container's lock.
use super::default_true;
use crate::{
    App,
    error::{Error, Result, missing},
    jobs::{launch, launch_named},
    model::*,
};
use anyhow::Context;
use axum::{
    Json,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

pub async fn containers(State(app): State<Arc<App>>) -> Result<Json<Value>> {
    let containers: Vec<Container> = app.db.list(CONTAINERS)?;
    let ids = app
        .docker(&[
            "ps",
            "-a",
            "--filter",
            &format!("label={LABEL}=true"),
            "--format",
            "{{json .}}",
        ])
        .await;
    let mut states = std::collections::HashMap::new();
    if let Ok(ref data) = ids {
        for line in String::from_utf8_lossy(data).lines() {
            if let Ok(value) = serde_json::from_str::<Value>(line)
                && let Some(name) = value["Names"].as_str()
            {
                states.insert(name.to_string(), value);
            }
        }
    }
    let unhealthy = states
        .iter()
        .filter(|(_, docker)| {
            docker["Status"]
                .as_str()
                .is_some_and(|status| status.contains("(unhealthy)"))
        })
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    let details = health_details(&app, &unhealthy).await;
    let sets = crate::scale::all(&app).unwrap_or_default();
    // Each record with Docker's view of it beside its own fields.
    let items = containers
        .into_iter()
        .map(|record| {
            let docker = states.get(&record.container);
            let status = docker
                .and_then(|d| d["Status"].as_str())
                .unwrap_or_default();
            let health = if status.contains("(unhealthy)") {
                "unhealthy"
            } else if status.contains("(healthy)") {
                "healthy"
            } else if status.contains("(health: starting)") {
                "starting"
            } else {
                "unknown"
            };
            let state = if ids.is_err() {
                "unavailable"
            } else {
                docker
                    .and_then(|d| d["State"].as_str())
                    .unwrap_or("missing")
            };
            // Whether something uses it, and when its idle stop comes if nothing does from now on.
            let (in_use, idle_stop_at) =
                crate::lifecycle::use_of(&app, &record, state == "running");
            let mut item = json!(record);
            item["state"] = json!(state);
            item["in_use"] = json!(in_use);
            item["idle_stop_at"] = json!(idle_stop_at);
            item["health"] = json!(health);
            item["health_detail"] = json!(details.get(&record.container));
            item["status"] = json!(docker.map(|d| &d["Status"]));
            // Whether the members of its groups are kept from its screen.
            item["vnc_private"] = json!(docker.map(crate::groups::vnc_private));
            // The scale set it is an instance of, with its number.
            item["scale"] = crate::scale::membership(&sets, &record.id);
            item
        })
        .collect::<Vec<_>>();
    Ok(Json(json!(items)))
}

/// How much of a health check's output the list carries.
const DETAIL_LIMIT: usize = 300;

/// Why each of the named containers is unhealthy, by name: the first line of its newest health
/// check's output, which a Thoric desktop's check begins with a word that says what failed, such
/// as `display-held:`, and Docker gives as `Health check exceeded timeout (4s)` for one that did
/// not finish. Docker's list leaves the output out, so it is read only while one is unhealthy, and
/// a failure to read it leaves the list without it.
async fn health_details(app: &App, names: &[&str]) -> std::collections::HashMap<String, String> {
    if names.is_empty() {
        return Default::default();
    }
    let mut args = vec!["inspect", "--format", "{{.Name}}\t{{json .State.Health}}"];
    args.extend(names);
    match app.docker(&args).await {
        Ok(output) => read_health_details(&String::from_utf8_lossy(&output)),
        Err(error) => {
            tracing::debug!("Could not read the health checks of {names:?}: {error:#}");
            Default::default()
        }
    }
}

/// [`health_details`] from what `docker inspect` printed, a name and the health state a line.
fn read_health_details(text: &str) -> std::collections::HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (name, health) = line.split_once('\t')?;
            let health = serde_json::from_str::<Value>(health).ok()?;
            let output = health["Log"].as_array()?.last()?["Output"].as_str()?;
            let first = output
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())?;
            let mut end = first.len().min(DETAIL_LIMIT);
            while !first.is_char_boundary(end) {
                end -= 1;
            }
            Some((
                name.trim_start_matches('/').to_owned(),
                first[..end].to_owned(),
            ))
        })
        .collect()
}

pub async fn container(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<Json<Value>> {
    app.container(&id)?;
    let Json(items) = containers(State(app)).await?;
    let item = items
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == id))
        .cloned()
        .ok_or_else(missing)?;
    Ok(Json(item))
}

/// A new container's settings, and values for the compose parameters of its image.
#[derive(Deserialize, schemars::JsonSchema)]
pub struct CreateContainer {
    #[serde(flatten)]
    pub spec: ContainerSpec,
    #[serde(default)]
    pub parameters: std::collections::BTreeMap<String, String>,
    /// Give the container an MCP endpoint of its own once it is created.
    #[serde(default = "default_true")]
    pub mcp_endpoint: bool,
    /// The password of the container's own proxy, kept apart from its settings.
    #[serde(default)]
    pub proxy_password: Option<String>,
    /// Make the container scalable: instance 1 of a set that creates more of it on demand, from
    /// the same request (see `scale`).
    #[serde(default)]
    pub scaling: Option<crate::scale::Scaling>,
}
/// A create request with the installation's defaults for the features it leaves out. Only a
/// request gets them: the serde defaults of [`ContainerSpec`] fill the fields that records saved
/// before a field existed lack, and keep their meaning.
pub(crate) fn creation_request(
    config: &crate::config::Config,
    mut body: Value,
) -> Result<CreateContainer> {
    let defaults = config.feature_defaults();
    if let Value::Object(request) = &mut body {
        for (feature, default) in [
            ("gpu", defaults.gpu),
            ("internet", defaults.internet),
            ("sudo", defaults.sudo),
            ("audio", defaults.audio),
            ("gamepad", defaults.gamepad),
            ("vpn", defaults.vpn),
            ("debugger", defaults.debugger),
            ("realtime", defaults.realtime),
            ("profiler", defaults.profiler),
            ("kvm", defaults.kvm),
        ] {
            request.entry(feature).or_insert(json!(default));
        }
        // The microphone travels over the audio connection.
        let audio = request.get("audio") == Some(&Value::Bool(true));
        request.entry("microphone").or_insert(json!(
            audio && config.defaults.microphone && config.microphone
        ));
        // A container follows the installation's proxy unless it asks otherwise.
        request.entry("proxy").or_insert(if config.proxy.is_some() {
            json!({"mode": "installation"})
        } else {
            Value::Null
        });
        // Its idle stop is the installation's unless it names one, and an expiry in minutes
        // counts from now.
        let lifecycle = crate::lifecycle::for_creation(
            request.get("lifecycle"),
            defaults.idle_stop_minutes,
            now(),
        )?;
        request.insert("lifecycle".into(), json!(lifecycle));
    }
    serde_json::from_value(body).map_err(|e| {
        Error(
            StatusCode::BAD_REQUEST,
            format!("Invalid container request: {e}"),
        )
    })
}
pub async fn create(State(app): State<Arc<App>>, Json(body): Json<Value>) -> Result<Json<Value>> {
    let CreateContainer {
        mut spec,
        parameters,
        mcp_endpoint,
        proxy_password,
        scaling,
    } = creation_request(&app.config, body)?;
    let password =
        crate::outbound::PasswordChange::from_value(proxy_password.map(Value::String).as_ref())?;
    password.settle(None, &mut spec.proxy)?;
    spec.validate()?;
    if let Some(scaling) = scaling {
        return create_scalable(app, spec, parameters, password, &scaling, mcp_endpoint).await;
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    // A compose found by the name the request uses is checked now; the job checks again once
    // Docker has resolved the image.
    let records: Vec<ImageRecord> = app.db.list("image")?;
    if let Some(record) = crate::compose::compose_image(&records, &spec.image, &spec.image)
        && let Some(compose) = &record.compose
    {
        let values = compose.resolve(&parameters, &id)?;
        crate::compose::check_mounts(&spec.mounts, &compose.mounts(&values))?;
    }
    let name = spec.name.clone();
    // The password is kept before the job, so the desktop's proxy has it as the desktop starts.
    app.save_proxy_password(&id, spec.proxy.as_ref(), &password)?;
    launch_named(
        app,
        "create",
        format!("container:{id}"),
        Some(name),
        move |app, job_id| async move {
            let created = app.create_container(&id, spec, parameters, &job_id).await;
            creation_finished(&app, &id, created, mcp_endpoint, &job_id).await
        },
    )
}
/// Create a scalable container: its set, with the request as the template and an MCP endpoint
/// whose tools take `instance` unless `mcp_endpoint` is false, and its first instance, whose job
/// the answer names. The instances the set keeps beyond the first follow once it stands.
async fn create_scalable(
    app: Arc<App>,
    spec: ContainerSpec,
    parameters: std::collections::BTreeMap<String, String>,
    password: crate::outbound::PasswordChange,
    scaling: &crate::scale::Scaling,
    mcp_endpoint: bool,
) -> Result<Json<Value>> {
    scaling.validate()?;
    app.installation_allows(&spec)?;
    let name = scaling
        .set
        .clone()
        .unwrap_or_else(|| crate::scale::name_of(&spec.name));
    crate::scale::set_name(&name)?;
    let password = match password {
        crate::outbound::PasswordChange::Set(password) => Some(password),
        _ => None,
    };
    let endpoint = crate::scale::instances::make_set(
        &app,
        &name,
        &spec,
        parameters,
        password,
        scaling,
        mcp_endpoint,
    )
    .await?;
    let mut answer =
        crate::scale::instances::grow(app, &name, Default::default(), String::new()).await?;
    answer["mcp_endpoint"] = json!(endpoint);
    Ok(Json(answer))
}
/// What a creation job ends with once the container's creation has returned: its own MCP endpoint
/// whenever the container's record was saved, also when its desktop was not ready or its setup
/// failed, so the endpoint works once the container is repaired.
pub(crate) async fn creation_finished(
    app: &App,
    id: &str,
    created: anyhow::Result<Value>,
    mcp_endpoint: bool,
    job_id: &str,
) -> anyhow::Result<Value> {
    // A creation refused before its record was saved leaves no container to keep it for.
    let Ok(container) = app.container(id) else {
        app.forget_proxy_password(id)?;
        return created;
    };
    let mut endpoint = None;
    if mcp_endpoint {
        // The container is made either way; an endpoint can still be created later.
        match crate::desktop_mcp::automatic_endpoint(app, &container).await {
            Ok(name) => {
                app.append_log(
                    job_id,
                    &format!("Created the MCP endpoint /mcp/{name} for this container."),
                )?;
                endpoint = Some(name);
            }
            Err(error) => app.append_log(
                job_id,
                &format!("Could not create an MCP endpoint: {error:#}"),
            )?,
        }
    }
    let mut result = created?;
    if let Some(name) = endpoint {
        result["mcp_endpoint"] = json!(name);
    }
    Ok(result)
}
#[derive(Deserialize, schemars::JsonSchema)]
#[schemars(rename = "ContainerAction")]
pub struct Action {
    pub action: String,
}
pub async fn action(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<Action>,
) -> Result<Json<Value>> {
    if !["start", "stop", "restart"].contains(&body.action.as_str()) {
        return Err(Error(StatusCode::BAD_REQUEST, "Invalid action".into()));
    }
    app.container(&id)?;
    launch(
        app,
        &body.action.clone(),
        format!("container:{id}"),
        move |app, job_id| async move { run_action(&app, &id, &body.action, &job_id).await },
    )
}
/// Start, stop or restart the container `id` in a job that holds it: what `action` runs, and
/// the lifecycle's stops too.
pub(crate) async fn run_action(
    app: &App,
    id: &str,
    action: &str,
    job_id: &str,
) -> anyhow::Result<Value> {
    let env = app.container(id).map_err(|e| anyhow::anyhow!(e.1))?;
    let info = app.owned(&env).await?;
    if action != "stop" {
        app.before_start(&env, &info).await?;
    }
    app.docker(&[action, info["Id"].as_str().context("Missing container ID")?])
        .await
        .map_err(|error| app.device_error(&env, error))?;
    // A started desktop is waited for beside the work that only uses it.
    if action != "stop" {
        crate::jobs::share_from_here();
        app.desktop_ready(&env, job_id).await?;
    }
    Ok(json!({"container_id":id}))
}
#[derive(Deserialize, schemars::JsonSchema)]
pub struct Internet {
    pub enabled: bool,
}
pub async fn internet(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<Internet>,
) -> Result<Json<Value>> {
    app.container(&id)?;
    // Turning it off stays possible on a container that has it.
    if body.enabled && !app.config.internet {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            "Internet access is disabled for this installation (INTERNET_ENABLED=false)".into(),
        ));
    }
    launch(
        app,
        "internet",
        format!("container:{id}"),
        move |app, _| async move {
            let mut env = app.container(&id).map_err(|e| anyhow::anyhow!(e.1))?;
            app.set_internet(&mut env, body.enabled).await?;
            Ok(json!({"container_id":id,"internet":body.enabled}))
        },
    )
}
/// Change a container's settings. `dry_run` reports which changes apply in place and which
/// restart the desktop, without a job.
pub async fn update_settings(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    let env = app.container(&id)?;
    let Value::Object(mut request) = body else {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            "Settings must be a JSON object".into(),
        ));
    };
    let dry_run = match request.remove("dry_run") {
        None => false,
        Some(Value::Bool(dry_run)) => dry_run,
        Some(_) => {
            return Err(Error(
                StatusCode::BAD_REQUEST,
                "dry_run must be true or false".into(),
            ));
        }
    };
    let (_, changes) = app.settings_preview(&env, &request)?;
    if dry_run {
        return Ok(Json(
            json!({"container_id": id, "live": changes.live, "restart": changes.restart}),
        ));
    }
    launch(
        app,
        "settings",
        format!("container:{id}"),
        move |app, job_id| async move { app.update_container(&id, request, &job_id).await },
    )
}
/// How a container is deleted: its named volumes stay unless `delete_volumes` asks for those
/// created for it.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Remove {
    #[serde(default)]
    pub delete_volumes: bool,
}
pub async fn remove(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    axum::extract::Query(options): axum::extract::Query<Remove>,
) -> Result<Json<Value>> {
    app.container(&id)?;
    launch(
        app,
        "delete",
        format!("container:{id}"),
        move |app, job_id| async move {
            let env = app.container(&id).map_err(|e| anyhow::anyhow!(e.1))?;
            let removed = app
                .remove_container(&env, &job_id, options.delete_volumes)
                .await?;
            Ok(removal(
                &id,
                &env.spec.name,
                options.delete_volumes,
                removed,
            ))
        },
    )
}
/// A delete job's result. The volumes it went through are listed only when it was asked to.
pub(crate) fn removal(
    id: &str,
    name: &str,
    asked: bool,
    removed: crate::volumes::RemovedVolumes,
) -> Value {
    let mut result = json!({"deleted":id,"name":name,"volumes_preserved":removed.deleted.is_empty(),"images_preserved":true});
    if asked {
        result["volumes_deleted"] = json!(removed.deleted);
        result["volumes_kept"] = json!(removed.kept);
    }
    result
}
pub async fn logs(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<Json<Value>> {
    let env = app.container(&id)?;
    let info = app.owned(&env).await?;
    let args = [
        "logs",
        "--tail",
        "300",
        info["Id"].as_str().context("Missing container ID")?,
    ]
    .map(str::to_owned);
    let output = crate::process::capture(
        app.command(&args),
        Duration::from_secs(15),
        262_144,
        crate::process::Overflow::Tail,
    )
    .await?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(String::from_utf8_lossy(&output.stderr).into_owned()).into());
    }
    Ok(Json(
        json!({"log":format!("{}{}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr))}),
    ))
}
pub async fn retry(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<Json<Value>> {
    app.container(&id)?;
    launch(
        app,
        "retry",
        format!("container:{id}"),
        move |app, job_id| async move { app.retry_container(&id, &job_id).await },
    )
}
pub async fn reset(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<Json<Value>> {
    app.container(&id)?;
    launch(
        app,
        "reset",
        format!("container:{id}"),
        move |app, job_id| async move { app.reset_container(&id, &job_id).await },
    )
}
pub async fn screenshot(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    axum::extract::Query(options): axum::extract::Query<crate::capture::Capture>,
) -> Result<Response> {
    let shot = crate::capture::capture(&app, &id, &options).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg".to_string()),
            (header::CACHE_CONTROL, "no-store".to_string()),
            (
                header::HeaderName::from_static("x-image-width"),
                shot.width.to_string(),
            ),
            (
                header::HeaderName::from_static("x-image-height"),
                shot.height.to_string(),
            ),
            (
                header::HeaderName::from_static("x-captured-at"),
                shot.captured_at.to_string(),
            ),
        ],
        shot.jpeg,
    )
        .into_response())
}
pub async fn diagnostics(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let env = app.container(&id)?;
    let info = app.owned(&env).await?;
    let bytes = app
        .docker(&[
            "exec",
            "--user",
            "desktop",
            info["Id"].as_str().context("Missing container ID")?,
            "/usr/local/bin/desktop-diagnostics",
        ])
        .await?;
    Ok(Json(
        json!({"output":String::from_utf8_lossy(&bytes),"gpu_requested":env.spec.gpu}),
    ))
}
#[cfg(test)]
#[path = "containers_tests.rs"]
mod tests;
