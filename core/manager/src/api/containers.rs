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
            let mut item = json!(record);
            item["state"] = json!(if ids.is_err() {
                "unavailable"
            } else {
                docker
                    .and_then(|d| d["State"].as_str())
                    .unwrap_or("missing")
            });
            item["health"] = json!(health);
            item["status"] = json!(docker.map(|d| &d["Status"]));
            // Whether the members of its groups are kept from its screen.
            item["vnc_private"] = json!(docker.map(crate::groups::vnc_private));
            item
        })
        .collect::<Vec<_>>();
    Ok(Json(json!(items)))
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
    } = creation_request(&app.config, body)?;
    let password =
        crate::outbound::PasswordChange::from_value(proxy_password.map(Value::String).as_ref())?;
    password.settle(None, &mut spec.proxy)?;
    spec.validate()?;
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
        move |app, job_id| async move {
            let env = app.container(&id).map_err(|e| anyhow::anyhow!(e.1))?;
            let info = app.owned(&env).await?;
            if body.action != "stop" {
                app.before_start(&env, &info).await?;
            }
            app.docker(&[
                &body.action,
                info["Id"].as_str().context("Missing container ID")?,
            ])
            .await
            .map_err(|error| app.device_error(&env, error))?;
            // A started desktop is waited for beside the work that only uses it.
            if body.action != "stop" {
                crate::jobs::share_from_here();
                app.desktop_ready(&env, &job_id).await?;
            }
            Ok(json!({"container_id":id}))
        },
    )
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
