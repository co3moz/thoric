use crate::{
    App,
    error::{Error, Result},
};
use anyhow::{Context, bail};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub fn window_id(value: &str) -> anyhow::Result<()> {
    let hex = value.strip_prefix("0x").is_some_and(|digits| {
        (1..=8).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_hexdigit())
    });
    let decimal = (1..=10).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit());
    if !hex && !decimal {
        bail!("window_id must be an X11 window ID such as 0x3a00007");
    }
    Ok(())
}
#[derive(Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Geometry {
    /// New left edge in desktop pixels; requires y.
    #[serde(default)]
    pub x: Option<u32>,
    /// New top edge in desktop pixels; requires x.
    #[serde(default)]
    pub y: Option<u32>,
    /// New client width in pixels; requires height.
    #[serde(default)]
    pub width: Option<u32>,
    /// New client height in pixels; requires width.
    #[serde(default)]
    pub height: Option<u32>,
}
impl Geometry {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.x.is_some() != self.y.is_some() || self.width.is_some() != self.height.is_some() {
            bail!("Provide x with y, and width with height");
        }
        if self.x.is_none() && self.width.is_none() {
            bail!("Provide a position, a size or both");
        }
        if [self.x, self.y].iter().flatten().any(|v| *v > 16384)
            || [self.width, self.height]
                .iter()
                .flatten()
                .any(|v| *v == 0 || *v > 16384)
        {
            bail!("Window geometry must stay within 16384 pixels");
        }
        Ok(())
    }
}
async fn x11(app: &App, id: &str, command: &[&str]) -> Result<Vec<u8>> {
    let env = app.container(id)?;
    let info = app.owned(&env).await?;
    if info["State"]["Running"] != true {
        return Err(Error(
            StatusCode::CONFLICT,
            "Start the container before using its windows".into(),
        ));
    }
    let container = info["Id"].as_str().context("Missing container ID")?;
    let mut args = vec![
        "exec",
        "--user",
        "desktop",
        "--env",
        "DISPLAY=:0",
        container,
    ];
    args.extend(command);
    let output = crate::display::watched(app, container, app.docker(&args)).await;
    output.map_err(|e| {
        if format!("{e:#}").contains("executable file not found") {
            anyhow::anyhow!(
                "Window tools need xdotool and wmctrl in the desktop image; update its catalog image or the installation"
            )
            .into()
        } else {
            e.into()
        }
    })
}
pub async fn list_windows(app: &App, id: &str) -> Result<Value> {
    let output = x11(
        app,
        id,
        &[
            "bash",
            "-c",
            "wmctrl -lpG; printf '\\n--active\\n'; xdotool getactivewindow 2>/dev/null || true",
        ],
    )
    .await?;
    let text = String::from_utf8_lossy(&output);
    let (list, active) = text.split_once("\n--active\n").unwrap_or((&text, ""));
    let active = active.trim().parse::<u64>().ok();
    let items = list
        .lines()
        .filter_map(|line| {
            let mut rest = line;
            let mut fields = Vec::with_capacity(8);
            for _ in 0..8 {
                let trimmed = rest.trim_start();
                let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
                fields.push(&trimmed[..end]);
                rest = &trimmed[end..];
            }
            let numeric = u64::from_str_radix(fields[0].trim_start_matches("0x"), 16).ok()?;
            let number = |index: usize| fields[index].parse::<i64>().ok();
            Some(json!({
                "window_id": fields[0],
                "desktop": number(1)?,
                "pid": number(2)?,
                "x": number(3)?,
                "y": number(4)?,
                "width": number(5)?,
                "height": number(6)?,
                "title": rest.trim(),
                "active": active == Some(numeric),
            }))
        })
        .collect::<Vec<_>>();
    Ok(json!({"items": items, "active_window_id": active.map(|id| format!("{id:#x}"))}))
}
pub async fn focus_window(app: &App, id: &str, window: &str) -> Result<Value> {
    window_id(window)?;
    x11(
        app,
        id,
        &[
            "timeout",
            "5",
            "xdotool",
            "windowactivate",
            "--sync",
            window,
        ],
    )
    .await?;
    Ok(json!({"window_id": window, "active": true}))
}
pub async fn set_window_geometry(
    app: &App,
    id: &str,
    window: &str,
    geometry: &Geometry,
) -> Result<Value> {
    window_id(window)?;
    geometry.validate()?;
    if let (Some(x), Some(y)) = (geometry.x, geometry.y) {
        x11(
            app,
            id,
            &[
                "timeout",
                "5",
                "xdotool",
                "windowmove",
                "--sync",
                window,
                &x.to_string(),
                &y.to_string(),
            ],
        )
        .await?;
    }
    if let (Some(width), Some(height)) = (geometry.width, geometry.height) {
        x11(
            app,
            id,
            &[
                "timeout",
                "5",
                "xdotool",
                "windowsize",
                "--sync",
                window,
                &width.to_string(),
                &height.to_string(),
            ],
        )
        .await?;
    }
    let output = x11(
        app,
        id,
        &["xdotool", "getwindowgeometry", "--shell", window],
    )
    .await?;
    let text = String::from_utf8_lossy(&output);
    let value = |prefix: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(prefix))
            .and_then(|v| v.trim().parse::<i64>().ok())
    };
    Ok(json!({
        "window_id": window,
        "x": value("X="),
        "y": value("Y="),
        "width": value("WIDTH="),
        "height": value("HEIGHT="),
    }))
}
pub async fn list(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<Json<Value>> {
    Ok(Json(list_windows(&app, &id).await?))
}
pub async fn focus(
    State(app): State<Arc<App>>,
    Path((id, window)): Path<(String, String)>,
) -> Result<Json<Value>> {
    Ok(Json(focus_window(&app, &id, &window).await?))
}
pub async fn geometry(
    State(app): State<Arc<App>>,
    Path((id, window)): Path<(String, String)>,
    Json(body): Json<Geometry>,
) -> Result<Json<Value>> {
    Ok(Json(set_window_geometry(&app, &id, &window, &body).await?))
}

const INSPECT: &str = include_str!("../helpers/inspect_ui.py");
#[derive(Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Inspect {
    /// Only the application with this process ID (from get_application).
    #[serde(default)]
    pub pid: Option<u64>,
    /// Only applications whose accessible name contains this text.
    #[serde(default)]
    pub name: Option<String>,
    /// Tree depth to descend (1 to 20).
    #[serde(default = "inspect_depth")]
    pub max_depth: u8,
    /// Maximum nodes to return (1 to 2000).
    #[serde(default = "inspect_nodes")]
    pub max_nodes: u32,
}
fn inspect_depth() -> u8 {
    6
}
fn inspect_nodes() -> u32 {
    500
}
impl Inspect {
    pub fn validate(&self) -> anyhow::Result<()> {
        if !(1..=20).contains(&self.max_depth) {
            bail!("max_depth must be 1 to 20");
        }
        if !(1..=2000).contains(&self.max_nodes) {
            bail!("max_nodes must be 1 to 2000");
        }
        if let Some(name) = &self.name
            && (name.is_empty() || name.len() > 200)
        {
            bail!("name must be 1 to 200 bytes");
        }
        Ok(())
    }
}
/// Read the AT-SPI accessibility tree of applications that expose one.
pub async fn inspect_ui(app: &App, id: &str, options: &Inspect) -> Result<Value> {
    options.validate()?;
    let pid = options.pid.map(|p| p.to_string()).unwrap_or_default();
    let output = x11(
        app,
        id,
        &[
            "python3",
            "-c",
            INSPECT,
            &pid,
            options.name.as_deref().unwrap_or(""),
            &options.max_depth.to_string(),
            &options.max_nodes.to_string(),
        ],
    )
    .await?;
    serde_json::from_slice(&output).map_err(|e| anyhow::anyhow!(e).into())
}
pub async fn inspect(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Query(options): Query<Inspect>,
) -> Result<Json<Value>> {
    Ok(Json(inspect_ui(&app, &id, &options).await?))
}
