//! The HTTP API's handlers by resource: a container's routes in `containers`, an image's in
//! `images`, and here the session, settings and system routes and the small ones of volumes and
//! jobs. Routes with a subject of their own, such as files, uploads or a volume's files, live
//! with it; `main.rs` is the route table.
mod containers;
mod images;
mod scale;

#[cfg(test)]
pub(crate) use containers::creation_request;
pub(crate) use containers::{
    Action, CreateContainer, Internet, Remove, action, container, containers, create,
    creation_finished, diagnostics, internet, logs, removal, remove, reset, retry, run_action,
    screenshot, update_settings,
};
pub(crate) use images::{
    BuildImage, RegisterImage, Snapshot, build, build_catalog, image_config, image_list,
    image_summary, images, pull_catalog, register_image, snapshot,
};
pub(crate) use scale::{
    MakeScalable, NewInstance, UpdateSet, create_instance, delete_instance, delete_set,
    list_instances, make_scalable, set, set_activity, sets, update_set,
};

use crate::{App, error::Result, jobs, model::*};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub async fn session(State(app): State<Arc<App>>, headers: axum::http::HeaderMap) -> Json<Value> {
    Json(json!({"auth_enabled":app.config.auth,"authenticated":app.authenticated(&headers)}))
}
pub async fn settings(State(app): State<Arc<App>>) -> Json<Value> {
    Json(
        json!({"version":env!("CARGO_PKG_VERSION"),"architecture":app.config.architecture,"base_image":app.config.base_image,"audio_enabled":app.config.audio,"microphone_enabled":app.config.microphone,"gpu_enabled":app.config.gpu_enabled(),"wsl_gpu":app.config.wsl_gpu,"nvidia_compute":app.config.cuda,"dri_gpu":app.config.dri_gpu,"gpu_adapter":app.config.adapter,"gpu":crate::gpu::report(&app.config),"upload_limit_bytes":app.config.upload_limit,"archive_limit_bytes":app.config.archive_limit,"templates_url":app.config.templates_url,"volume_filter":app.config.volume_filter,"root_mcp_enabled":app.config.root_mcp_enabled,"internet_enabled":app.config.internet,"sudo_enabled":app.config.sudo,"gamepad_enabled":app.config.gamepad,"vpn_enabled":app.config.vpn,"debugger_enabled":app.config.debugger,"realtime_enabled":app.config.realtime,"profiler_enabled":app.config.profiler,"kvm_enabled":crate::permissions::kvm_enabled(&app.config),"kvm_host":app.config.kvm_host,"proxy":app.config.proxy.as_ref().map(|proxy| json!({"url":proxy.url,"username":proxy.username,"bypass":proxy.bypass})),"defaults":app.config.feature_defaults()}),
    )
}
pub async fn system(State(app): State<Arc<App>>) -> Json<Value> {
    match app.docker(&["info", "--format", "{{json .}}"]).await {
        Ok(data) => {
            let info: Value = serde_json::from_slice(&data).unwrap_or_default();
            Json(
                json!({"connected":true,"version":info["ServerVersion"],"cpus":info["NCPU"],"memory_bytes":info["MemTotal"],"containers":info["Containers"],"manager_configured":app.config.manager.is_some()}),
            )
        }
        Err(e) => Json(json!({"connected":false,"error":format!("{e:#}")})),
    }
}
pub async fn volumes(State(app): State<Arc<App>>) -> Result<Json<Value>> {
    let filter = app.config.volume_filter.as_deref();
    let bytes = app.docker_args(&crate::volumes::list_args(filter)).await?;
    let mut values = String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect::<Vec<_>>();
    let manager = app.listing_manager().await?;
    crate::volumes::mark_manager_volumes(&mut values, manager.as_ref());
    crate::volumes::mark_owners(&mut values, &app.db.list::<Container>(CONTAINERS)?);
    Ok(Json(json!(values)))
}
#[derive(Deserialize, schemars::JsonSchema)]
pub struct Volume {
    pub name: String,
}
pub async fn create_volume(
    State(app): State<Arc<App>>,
    Json(body): Json<Volume>,
) -> Result<Json<Value>> {
    volume_name(&body.name)?;
    let _guard = app.hold(&format!("volume:{}", body.name)).await?;
    app.new_volume(&body.name).await?;
    Ok(Json(json!({"name":body.name})))
}
/// `GET /api/jobs`: the newest 100 jobs without their logs, of every kind unless
/// [`jobs::history::Listed`] names some or leaves the scripts out.
pub async fn jobs(
    State(app): State<Arc<App>>,
    Query(listed): Query<jobs::history::Listed>,
) -> Result<Json<Value>> {
    let jobs = app.db.jobs_listed(&listed.filter()?)?;
    Ok(Json(json!(jobs)))
}
/// `GET /api/jobs/{id}`: the job with its whole log, or as [`jobs::read::Read`] asks for it.
pub async fn job(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Query(read): Query<jobs::read::Read>,
) -> Result<Json<Value>> {
    Ok(Json(jobs::read::answer(&app, &id, &read, None).await?))
}
fn default_true() -> bool {
    true
}
#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;
