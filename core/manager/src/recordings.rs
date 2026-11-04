use crate::{
    App,
    capture::Capture,
    error::{Error, Result, missing},
    model::{Application, Recording, now},
    process, uploads,
};
use anyhow::{Context, bail};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

const RECORDER: &str = include_str!("../helpers/recorder.py");
const CONTROL: &str = include_str!("../helpers/app_control.py");
const EXPORTER: &str = include_str!("../helpers/export_artifacts.py");
const VIDEO: &str = include_str!("../helpers/recording_video.py");
pub const ROOT: &str = "/tmp/thoric-recordings";
/// How long the videos of one stop or export may take to encode. Past it, the recordings left go
/// into an MJPEG AVI, which only copies their frames, so the helper ends soon after.
const ENCODE_SECONDS: u64 = 90;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StartRecording {
    /// Frames per second (1 to 5).
    #[serde(default = "fps")]
    pub fps: u8,
    /// Stop automatically after this many seconds (1 to 600).
    #[serde(default = "max_seconds")]
    pub max_seconds: u64,
    /// Scale frames down to this width (16 to 7680); default 960.
    #[serde(default = "max_width")]
    pub max_width: u32,
    /// Record only this region, given as x,y,width,height in desktop pixels.
    #[serde(default)]
    pub region: Option<String>,
    /// Display name for the recording.
    #[serde(default)]
    pub name: Option<String>,
}
fn fps() -> u8 {
    2
}
fn max_seconds() -> u64 {
    60
}
fn max_width() -> u32 {
    960
}
impl StartRecording {
    pub fn validate(&self) -> anyhow::Result<()> {
        if !(1..=5).contains(&self.fps) {
            bail!("fps must be 1 to 5");
        }
        if !(1..=600).contains(&self.max_seconds) {
            bail!("max_seconds must be 1 to 600");
        }
        if let Some(name) = &self.name
            && (name.trim().is_empty() || name.chars().count() > 100)
        {
            bail!("Recording name must be 1 to 100 characters");
        }
        Capture {
            max_width: Some(self.max_width),
            region: self.region.clone(),
            window_id: None,
        }
        .validate()
    }
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Export {
    /// Directory inside the desktop: in its home, a writable volume or a compose folder.
    pub destination: String,
    /// Archive file name, typically ending in .tar.gz.
    pub name: String,
    /// Recording IDs whose frames are included.
    #[serde(default)]
    pub recording_ids: Vec<String>,
    /// Application IDs whose logs are included.
    #[serde(default)]
    pub application_ids: Vec<String>,
    /// Replace an existing archive with the same name.
    #[serde(default)]
    pub replace: bool,
}
impl Export {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.recording_ids.is_empty() && self.application_ids.is_empty() {
            bail!("Provide at least one recording_id or application_id");
        }
        if self.recording_ids.len() + self.application_ids.len() > 100 {
            bail!("At most 100 artifacts can be exported at once");
        }
        Ok(())
    }
}
fn record(app: &App, id: &str, recording: &str) -> Result<Recording> {
    let record: Recording = app.db.get("recording", recording)?;
    if record.container_id != id {
        return Err(missing());
    }
    Ok(record)
}
async fn running(app: &App, id: &str) -> Result<(crate::model::Container, Value)> {
    let env = app.container(id)?;
    let info = app.owned(&env).await?;
    if info["State"]["Running"] != true {
        return Err(Error(
            StatusCode::CONFLICT,
            "Start the container before recording".into(),
        ));
    }
    Ok((env, info))
}
async fn control(app: &App, container: &str, arguments: &[&str]) -> anyhow::Result<Value> {
    let mut args = vec![
        "exec",
        "--user",
        "1001:1001",
        container,
        "python3",
        "-c",
        CONTROL,
    ];
    args.extend(arguments);
    let output = app.docker(&args).await?;
    serde_json::from_slice(&output).context("Recording control returned invalid data")
}
/// Makes the video of every recording in `directories` that finished without one, as the desktop
/// user, and returns their states in the same order.
async fn videos(app: &App, container: &str, directories: &[String]) -> anyhow::Result<Vec<Value>> {
    let mut args = [
        "exec",
        "--user",
        "1001:1001",
        container,
        "python3",
        "-c",
        VIDEO,
        &ENCODE_SECONDS.to_string(),
    ]
    .map(str::to_owned)
    .to_vec();
    args.extend(directories.iter().cloned());
    let output = process::capture(
        app.command(&args),
        Duration::from_secs(ENCODE_SECONDS + 60),
        4 * 1024 * 1024,
        process::Overflow::Reject,
    )
    .await?;
    if !output.status.success() {
        bail!(
            "Recording video: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout).context("Recording video returned invalid data")
}
fn describe(record: &Recording, status: Value) -> Value {
    let mut value = json!({
        "recording_id": record.id,
        "container_id": record.container_id,
        "name": record.name,
        "fps": record.fps,
        "created_at": record.created_at,
        "directory": format!("{ROOT}/{}", record.id),
    });
    if let Some(fields) = status.as_object() {
        for (key, field) in fields {
            value[key] = field.clone();
        }
    }
    if let Some(path) = value["video"]["path"].as_str().map(str::to_owned) {
        value["video"]["download_url"] =
            json!(crate::files::download_url(&record.container_id, &path));
    }
    value
}
pub async fn start_recording(app: &Arc<App>, id: &str, body: &StartRecording) -> Result<Value> {
    body.validate()?;
    let _guard = app.share(&format!("container:{id}")).await?;
    let (_, info) = running(app, id).await?;
    let record = Recording {
        id: uuid::Uuid::new_v4().simple().to_string(),
        container_id: id.into(),
        name: body.name.clone().unwrap_or_else(|| "Recording".into()),
        fps: body.fps,
        created_at: now(),
    };
    let directory = format!("{ROOT}/{}", record.id);
    let container = info["Id"].as_str().context("Missing container ID")?;
    let region = body
        .region
        .as_deref()
        .map(|r| r.split(',').map(str::trim).collect::<Vec<_>>().join(","))
        .unwrap_or_default();
    app.docker(&[
        "exec",
        "--detach",
        "--user",
        "desktop",
        "--env",
        "DISPLAY=:0",
        container,
        "python3",
        "-c",
        RECORDER,
        &directory,
        &body.fps.to_string(),
        &body.max_seconds.to_string(),
        &body.max_width.to_string(),
        &region,
        // Frames are read with the manager's reader, as screenshots are.
        crate::capture::READER,
    ])
    .await?;
    app.db.put("recording", &record.id, &record)?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let status = control(app, container, &["recording-status", &directory])
        .await
        .ok()
        .and_then(|v| v.as_array()?.first().cloned())
        .unwrap_or_else(|| json!({"state": "starting"}));
    Ok(describe(&record, status))
}
pub async fn stop_recording(app: &Arc<App>, id: &str, recording: &str) -> Result<Value> {
    let record = record(app, id, recording)?;
    let (_, info) = running(app, id).await?;
    let container = info["Id"].as_str().context("Missing container ID")?;
    let directory = format!("{ROOT}/{}", record.id);
    let mut status = control(app, container, &["recording-stop", &directory, "15"]).await?;
    // Also a recording that ended at its limit gets its video here, once.
    if status["state"] == "finished" && status["video"].is_null() {
        match videos(app, container, std::slice::from_ref(&directory)).await {
            Ok(mut items) if !items.is_empty() => status = items.swap_remove(0),
            Ok(_) => {}
            Err(error) => status["video"] = json!({"error": format!("{error:#}")}),
        }
    }
    Ok(describe(&record, status))
}
pub async fn list_recordings(app: &App, id: &str) -> Result<Value> {
    let env = app.container(id)?;
    let records = app
        .db
        .list::<Recording>("recording")?
        .into_iter()
        .filter(|record| record.container_id == id)
        .take(100)
        .collect::<Vec<_>>();
    let info = app.owned(&env).await?;
    let statuses = if info["State"]["Running"] == true && !records.is_empty() {
        let directories = records
            .iter()
            .map(|record| format!("{ROOT}/{}", record.id))
            .collect::<Vec<_>>();
        let mut arguments = vec!["recording-status"];
        arguments.extend(directories.iter().map(String::as_str));
        let container = info["Id"].as_str().context("Missing container ID")?;
        control(app, container, &arguments)
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let items = records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            describe(
                record,
                statuses.get(index).cloned().unwrap_or_else(
                    || json!({"state": "unknown", "error": "Container is not running"}),
                ),
            )
        })
        .collect::<Vec<_>>();
    Ok(json!({"items": items}))
}
pub async fn export_artifacts(app: &Arc<App>, id: &str, body: &Export) -> Result<Value> {
    body.validate()?;
    let _guard = app.share(&format!("container:{id}")).await?;
    let (env, info) = running(app, id).await?;
    uploads::validate_target(&env, &body.destination, &body.name)?;
    let mut recordings = serde_json::Map::new();
    for recording in &body.recording_ids {
        let record = record(app, id, recording)?;
        recordings.insert(record.id.clone(), json!(format!("{ROOT}/{}", record.id)));
    }
    let mut applications = serde_json::Map::new();
    for application in &body.application_ids {
        let record: Application = app.db.get("application", application)?;
        if record.container_id != id {
            return Err(missing());
        }
        applications.insert(
            record.id.clone(),
            json!(format!("{}/{}", apps_root(), record.id)),
        );
    }
    let container = info["Id"].as_str().context("Missing container ID")?;
    // A recording that ended at its limit and was never stopped gets its video before it is
    // packaged; one that cannot is exported with its frames as before.
    let directories = recordings
        .values()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if !directories.is_empty()
        && let Err(error) = videos(app, container, &directories).await
    {
        tracing::warn!("Recordings of {id} are exported without a new video: {error:#}");
    }
    let manifest = json!({"recordings": recordings, "applications": applications}).to_string();
    let args = [
        "exec",
        "--user",
        "1001:1001",
        container,
        "python3",
        "-c",
        EXPORTER,
        &body.destination,
        &body.name,
        if body.replace { "true" } else { "false" },
        &manifest,
    ]
    .map(str::to_owned);
    let output = process::capture(
        app.command(&args),
        Duration::from_secs(600),
        65536,
        process::Overflow::Tail,
    )
    .await?;
    if !output.status.success() {
        return Err(Error::helper(&output));
    }
    let mut value: Value =
        serde_json::from_slice(&output.stdout).map_err(|e| anyhow::anyhow!(e))?;
    let path = value["path"].as_str().unwrap_or_default().to_owned();
    value["download_url"] = json!(crate::files::download_url(id, &path));
    Ok(value)
}
fn apps_root() -> &'static str {
    "/tmp/thoric-apps"
}
pub async fn start(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<StartRecording>,
) -> Result<Json<Value>> {
    Ok(Json(start_recording(&app, &id, &body).await?))
}
pub async fn list(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<Json<Value>> {
    Ok(Json(list_recordings(&app, &id).await?))
}
pub async fn stop(
    State(app): State<Arc<App>>,
    Path((id, recording)): Path<(String, String)>,
) -> Result<Json<Value>> {
    Ok(Json(stop_recording(&app, &id, &recording).await?))
}
pub async fn export(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<Export>,
) -> Result<Json<Value>> {
    Ok(Json(export_artifacts(&app, &id, &body).await?))
}

#[cfg(test)]
#[path = "recordings_tests.rs"]
mod tests;
