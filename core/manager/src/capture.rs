use crate::{
    App,
    desktop_input::{self, Action},
    error::{self, Error, Result},
    files, process, uploads,
};
use anyhow::{Context, bail};
use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::StatusCode,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

mod pixels;
pub(crate) use pixels::Gray;

/// Screenshot options shared by the HTTP route, the MCP tool and act-and-observe.
#[derive(Default, Deserialize, JsonSchema)]
pub struct Capture {
    /// Scale the image down to this width when the capture is wider (16 to 7680). Omit for full
    /// desktop resolution, where pixel coordinates match desktop_input.
    #[serde(default)]
    pub max_width: Option<u32>,
    /// Crop to "x,y,width,height" in desktop pixels before scaling.
    #[serde(default)]
    pub region: Option<String>,
    /// Capture one window by its X11 ID from list_windows instead of the whole desktop.
    #[serde(default)]
    pub window_id: Option<String>,
}
impl Capture {
    pub fn region(&self) -> anyhow::Result<Option<[u32; 4]>> {
        let Some(region) = &self.region else {
            return Ok(None);
        };
        let parts = region
            .split(',')
            .map(|part| part.trim().parse::<u32>())
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("region must be x,y,width,height in desktop pixels")?;
        let [x, y, width, height] = parts[..] else {
            bail!("region must be x,y,width,height in desktop pixels");
        };
        if width == 0 || height == 0 || [x, y, width, height].iter().any(|v| *v > 7680) {
            bail!("region must describe a non-empty area within the desktop");
        }
        Ok(Some([x, y, width, height]))
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(width) = self.max_width
            && !(16..=7680).contains(&width)
        {
            bail!("max_width must be 16 to 7680");
        }
        self.region()?;
        if let Some(window) = &self.window_id {
            crate::windows::window_id(window)?;
        }
        Ok(())
    }
}
pub struct Screenshot {
    pub jpeg: Vec<u8>,
    pub width: u16,
    pub height: u16,
    pub captured_at: u64,
    pub placement: Placement,
}
impl Screenshot {
    pub fn summary(&self) -> Value {
        let Placement {
            scale,
            origin,
            desktop,
        } = self.placement;
        json!({
            "width": self.width,
            "height": self.height,
            "captured_at": self.captured_at,
            "bytes": self.jpeg.len(),
            "scale": scale,
            "origin_x": origin.map(|(x, _)| x),
            "origin_y": origin.map(|(_, y)| y),
            "desktop_width": desktop.map(|(width, _)| width),
            "desktop_height": desktop.map(|(_, height)| height),
        })
    }
}

/// Where a screenshot's pixels are on the desktop, which desktop_input takes coordinates of: the
/// desktop pixel under image pixel (x, y) is origin + (x, y) / scale. A part is `None` when the
/// desktop image could not tell it, such as a window's place without xwininfo.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Placement {
    /// Image pixels per desktop pixel: 1 at full resolution, less once max_width scaled it down.
    pub scale: Option<f64>,
    /// The desktop pixel at the image's top left.
    pub origin: Option<(i64, i64)>,
    /// The whole desktop's width and height.
    pub desktop: Option<(i64, i64)>,
}

/// A rectangle as xwininfo reports it: its absolute left and top, its width and its height.
type Rect = [i64; 4];

/// Runs the image's desktop-screenshot and, once it has succeeded, prints on stderr what xwininfo
/// tells of the desktop and of the window it captured, so the result can say where its pixels
/// are. Without xwininfo the screenshot is taken all the same.
const SHOT: &str = r#"geometry=$({ xwininfo -root; [[ -z $3 ]] || xwininfo -id "$3"; } 2>/dev/null)
/usr/local/bin/desktop-screenshot "$@" || exit
printf '%s\n' "$geometry" >&2"#;

/// The rectangles in xwininfo's output, the desktop's first and then the captured window's.
fn rectangles(text: &str) -> Vec<Rect> {
    let mut found = Vec::new();
    let (mut x, mut y, mut width) = (None, None, None);
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let Ok(value) = value.trim().parse::<i64>() else {
            continue;
        };
        match key {
            "Absolute upper-left X" => x = Some(value),
            "Absolute upper-left Y" => y = Some(value),
            "Width" => width = Some(value),
            "Height" => {
                if let (Some(x), Some(y), Some(width)) = (x.take(), y.take(), width.take()) {
                    found.push([x, y, width, value]);
                }
            }
            _ => {}
        }
    }
    found
}

/// Where an image `width` pixels wide, taken with `options`, sits on the desktop. desktop-screenshot
/// crops the window, or the whole desktop, to the region, which ImageMagick clips to it, and then
/// scales the result down to max_width only when it is wider.
fn place(options: &Capture, rects: &[Rect], width: u16) -> Placement {
    let desktop = rects.first().map(|&[_, _, width, height]| (width, height));
    let window = options.window_id.is_some();
    let base = if window {
        rects.get(1).copied()
    } else {
        rects
            .first()
            .map(|&[_, _, width, height]| [0, 0, width, height])
    };
    let region = options.region().ok().flatten().map(|r| r.map(i64::from));
    let (origin, source) = match (region, base) {
        (Some([x, y, w, _]), Some([left, top, base_width, _])) => {
            (Some((left + x, top + y)), Some(w.min(base_width - x)))
        }
        (Some([x, y, w, _]), None) if !window => (Some((x, y)), Some(w)),
        (None, Some([left, top, base_width, _])) => (Some((left, top)), Some(base_width)),
        (None, None) if !window => (Some((0, 0)), None),
        _ => (None, None),
    };
    let width = i64::from(width);
    let scale = match source {
        Some(source) if source > 0 => Some((width as f64 / source as f64 * 1e6).round() / 1e6),
        // Not scaled down: the image is narrower than max_width, or there was none.
        _ if options.max_width.is_none_or(|max| width < i64::from(max)) => Some(1.0),
        _ => None,
    };
    Placement {
        scale,
        origin,
        desktop,
    }
}
/// Read the frame size from a JPEG start-of-frame segment.
pub fn jpeg_dimensions(bytes: &[u8]) -> Option<(u16, u16)> {
    if bytes.len() < 4 || bytes[0] != 0xff || bytes[1] != 0xd8 {
        return None;
    }
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xff {
            return None;
        }
        let marker = bytes[i + 1];
        if marker == 0xff {
            i += 1;
            continue;
        }
        if marker == 0x01 || (0xd0..=0xd8).contains(&marker) {
            i += 2;
            continue;
        }
        if marker == 0xd9 || marker == 0xda {
            return None;
        }
        let length = usize::from(u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]));
        if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            if i + 9 > bytes.len() {
                return None;
            }
            let height = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]);
            let width = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]);
            return Some((width, height));
        }
        i += 2 + length;
    }
    None
}
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
/// How long a screenshot waits for one of the two that may be taken at once, so desktops that do
/// not answer cannot hold up every other container's screenshots.
const PERMIT_WAIT: Duration = Duration::from_secs(5);
/// How long one capture may take. A desktop answers in about a second, so a longer one is stuck,
/// and Docker's general limit would keep its permit for two minutes.
const CAPTURE_LIMIT: Duration = Duration::from_secs(20);

pub async fn capture(app: &App, id: &str, options: &Capture) -> Result<Screenshot> {
    options.validate()?;
    let _permit = tokio::time::timeout(PERMIT_WAIT, app.screenshots.acquire())
        .await
        .map_err(|_| {
            Error(
                StatusCode::TOO_MANY_REQUESTS,
                "Two screenshots are already being taken; retry in a moment".into(),
            )
        })?
        .map_err(|e| anyhow::anyhow!(e))?;
    let env = app.container(id)?;
    let info = app.owned(&env).await?;
    let region = options
        .region()?
        .map(|[x, y, width, height]| format!("{x},{y},{width},{height}"))
        .unwrap_or_default();
    let container = info["Id"].as_str().context("Missing container ID")?;
    let args = [
        "exec",
        "--user",
        "desktop",
        "--env",
        "DISPLAY=:0",
        container,
        "bash",
        "-c",
        SHOT,
        "desktop-screenshot",
        &options.max_width.unwrap_or(0).to_string(),
        &region,
        options.window_id.as_deref().unwrap_or(""),
    ]
    .map(str::to_owned);
    let shot = process::capture(
        app.command(&args),
        CAPTURE_LIMIT,
        4 * 1024 * 1024,
        process::Overflow::Reject,
    );
    let output = crate::display::watched(app, container, shot).await?;
    if !output.status.success() {
        return Err(error::docker_failure(&output.stderr, output.status.code()).into());
    }
    let bytes = output.stdout;
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return Err(anyhow::anyhow!("Screenshot not ready").into());
    }
    let (width, height) =
        jpeg_dimensions(&bytes).context("Screenshot dimensions could not be read")?;
    let rects = rectangles(&String::from_utf8_lossy(&output.stderr));
    Ok(Screenshot {
        jpeg: bytes,
        width,
        height,
        captured_at: now_ms(),
        placement: place(options, &rects, width),
    })
}
/// A screenshot saved inside the desktop, so the user can collect it through the dashboard.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveCapture {
    /// Absolute file path in the desktop home, a writable volume or a compose folder, written as the desktop user.
    pub path: String,
    /// Replace an existing file at path.
    #[serde(default)]
    pub replace: bool,
    #[serde(default)]
    pub capture: Capture,
}
impl SaveCapture {
    /// Split path into the directory and file name that the upload destination rules check.
    pub fn target(&self) -> anyhow::Result<(String, String)> {
        let target = self
            .path
            .strip_prefix('/')
            .and_then(|_| self.path.rsplit_once('/'))
            .filter(|(directory, name)| !directory.is_empty() && !name.is_empty());
        let Some((directory, name)) = target else {
            bail!(
                "path must be an absolute file path in the desktop home, a writable volume or a compose folder"
            );
        };
        Ok((directory.to_owned(), name.to_owned()))
    }
}
/// Capture a screenshot and store it inside the desktop under the upload destination rules.
/// The result describes the file by its path, size and dashboard download_url, so an agent can
/// hand the image to a person without relaying it through the conversation or the host.
pub async fn save_screenshot(app: &Arc<App>, id: &str, body: &SaveCapture) -> Result<Value> {
    let (destination, name) = body.target()?;
    let shot = capture(app, id, &body.capture).await?;
    let request = uploads::Upload {
        destination,
        name,
        size: shot.jpeg.len() as u64,
        replace: body.replace,
    };
    let saved = uploads::save_file(
        app,
        id.to_owned(),
        request,
        Body::from(shot.jpeg),
        std::future::pending(),
    )
    .await?
    .0;
    let path = saved["path"].as_str().unwrap_or_default().to_owned();
    let download_url = files::download_url(id, &path);
    Ok(json!({
        "path": path,
        "bytes": saved["bytes"],
        "width": shot.width,
        "height": shot.height,
        "captured_at": shot.captured_at,
        "download_url": download_url,
    }))
}
pub async fn save(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<SaveCapture>,
) -> Result<Json<Value>> {
    Ok(Json(save_screenshot(&app, &id, &body).await?))
}
/// Desktop input followed by a screenshot of the result.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Observe {
    pub actions: Vec<Action>,
    /// Milliseconds to wait after the input before capturing (0 to 10000).
    #[serde(default = "settle")]
    pub settle_ms: u64,
    #[serde(default)]
    pub capture: Capture,
}
fn settle() -> u64 {
    500
}
pub async fn act_and_observe(
    app: &Arc<App>,
    id: &str,
    body: &Observe,
) -> Result<(Value, Screenshot)> {
    if body.settle_ms > 10_000 {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            "settle_ms must be at most 10000".into(),
        ));
    }
    body.capture.validate()?;
    let input = desktop_input::perform(app, id, &body.actions).await?;
    if body.settle_ms > 0 {
        tokio::time::sleep(Duration::from_millis(body.settle_ms)).await;
    }
    let shot = capture(app, id, &body.capture).await?;
    Ok((input, shot))
}
pub async fn act(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<Observe>,
) -> Result<Json<Value>> {
    let (input, shot) = act_and_observe(&app, &id, &body).await?;
    let mut screenshot = shot.summary();
    screenshot["jpeg_base64"] = json!(STANDARD.encode(&shot.jpeg));
    Ok(Json(json!({"input": input, "screenshot": screenshot})))
}
#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
