use crate::{
    App,
    error::{Error, Result},
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
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

mod relative;

/// Longest single wait or key hold in one request.
pub const MAX_WAIT_MS: u64 = 10_000;
/// Combined waits, holds and gesture delays allowed in one request.
pub const MAX_TOTAL_DELAY_MS: u64 = 30_000;
/// How long an input request waits for the container before answering that it is busy.
const LOCK_WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Default, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    #[default]
    Left,
    Middle,
    Right,
}
impl Button {
    const ALL: [Button; 3] = [Button::Left, Button::Middle, Button::Right];
    fn mask(self) -> u8 {
        match self {
            Self::Left => 1,
            Self::Middle => 2,
            Self::Right => 4,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Middle => "middle",
            Self::Right => "right",
        }
    }
}
fn one() -> u8 {
    1
}
fn drag_duration() -> u64 {
    300
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    /// Move the pointer to absolute desktop coordinates.
    Move { x: u16, y: u16 },
    /// Move the pointer by an offset from its current position, in steps over duration_ms, as
    /// relative motion through XTest in the desktop: applications that grab the pointer and read
    /// raw motion, such as first-person cameras, get the offset even at the desktop's edge, where
    /// the pointer itself stops. Without xdotool in the desktop it is emulated with absolute
    /// positions, and the result's relative_motion says so.
    MoveRelative {
        dx: i32,
        dy: i32,
        #[serde(default = "one")]
        steps: u8,
        #[serde(default)]
        duration_ms: u64,
    },
    /// Click up to three times at a position.
    Click {
        x: u16,
        y: u16,
        #[serde(default)]
        button: Button,
        #[serde(default = "one")]
        count: u8,
    },
    /// Press and hold a mouse button, optionally after moving to x,y. It stays held until
    /// button_up, release_all or the end of the request.
    ButtonDown {
        #[serde(default)]
        x: Option<u16>,
        #[serde(default)]
        y: Option<u16>,
        #[serde(default)]
        button: Button,
    },
    /// Release a held mouse button, optionally after moving to x,y.
    ButtonUp {
        #[serde(default)]
        x: Option<u16>,
        #[serde(default)]
        y: Option<u16>,
        #[serde(default)]
        button: Button,
    },
    /// Press at the start point, move over duration_ms and release at the end point.
    Drag {
        from_x: u16,
        from_y: u16,
        to_x: u16,
        to_y: u16,
        #[serde(default)]
        button: Button,
        #[serde(default = "drag_duration")]
        duration_ms: u64,
    },
    /// Turn the mouse wheel at a position.
    Scroll {
        x: u16,
        y: u16,
        direction: Direction,
        #[serde(default = "one")]
        steps: u8,
    },
    /// Press keys together, then release them in reverse order.
    KeyPress { keys: Vec<String> },
    /// Press keys and keep them held until key_up, release_all or the end of the request.
    KeyDown { keys: Vec<String> },
    /// Release keys held by key_down.
    KeyUp { keys: Vec<String> },
    /// Press keys together, keep them held for milliseconds, then release them.
    HoldKeys {
        keys: Vec<String>,
        milliseconds: u64,
    },
    /// Type text; a newline presses Enter and a tab presses Tab.
    TypeText { text: String },
    /// Pause before the next action.
    Wait { milliseconds: u64 },
    /// Release every key and mouse button still held by this request.
    ReleaseAll,
}
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub actions: Vec<Action>,
}

pub(crate) fn keysym(key: &str) -> anyhow::Result<u32> {
    if key.chars().count() == 1 {
        let c = key.chars().next().unwrap();
        return match c {
            '\n' | '\r' => Ok(0xff0d),
            '\t' => Ok(0xff09),
            c if !c.is_control() => Ok(if (c as u32) < 256 {
                c as u32
            } else {
                0x01000000 | c as u32
            }),
            _ => Err(anyhow::anyhow!("Unsupported control character")),
        };
    }
    Ok(match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => 0xff0d,
        "tab" => 0xff09,
        "backspace" => 0xff08,
        "escape" | "esc" => 0xff1b,
        "delete" => 0xffff,
        "insert" => 0xff63,
        "home" => 0xff50,
        "end" => 0xff57,
        "pageup" => 0xff55,
        "pagedown" => 0xff56,
        "left" => 0xff51,
        "up" => 0xff52,
        "right" => 0xff53,
        "down" => 0xff54,
        "shift" => 0xffe1,
        "control" | "ctrl" => 0xffe3,
        "alt" => 0xffe9,
        "super" | "meta" | "win" => 0xffeb,
        "space" => 0x20,
        "capslock" => 0xffe5,
        "menu" => 0xff67,
        "print" | "printscreen" => 0xff61,
        "pause" => 0xff13,
        name if name.starts_with('f') => {
            let n: u32 = name[1..].parse().context("Unknown key name")?;
            if !(1..=12).contains(&n) {
                bail!("Function keys must be F1 to F12");
            }
            0xffbd + n
        }
        _ => bail!("Unknown key name: {key}"),
    })
}
fn pointer(mask: u8, x: u16, y: u16) -> Vec<u8> {
    let mut bytes = vec![5, mask];
    bytes.extend(x.to_be_bytes());
    bytes.extend(y.to_be_bytes());
    bytes
}
fn key(down: bool, keysym: u32) -> Vec<u8> {
    let mut bytes = vec![4, u8::from(down), 0, 0];
    bytes.extend(keysym.to_be_bytes());
    bytes
}
/// One step of a planned request, in the order it is sent.
#[derive(Debug, PartialEq)]
pub(crate) enum Step {
    /// RFB key messages.
    Keys(Vec<u8>),
    /// A pointer event at a position with `mask`'s buttons held, then a pause in milliseconds.
    Pointer {
        mask: u8,
        x: u16,
        y: u16,
        pause: u64,
    },
    /// A pointer event that only changes the buttons held, wherever the pointer is: `expected` is
    /// where the plan puts it, which relative motion may have left elsewhere.
    Buttons {
        mask: u8,
        expected: (u16, u16),
    },
    /// A pause in milliseconds.
    Wait(u64),
    Relative(Relative),
}
/// The motion of a `move_relative`: `steps` moves that add up to `dx`, `dy`, each followed by a
/// pause of `pause` milliseconds.
#[derive(Debug, PartialEq)]
pub(crate) struct Relative {
    pub dx: i32,
    pub dy: i32,
    pub steps: u8,
    pub pause: u64,
    /// The buttons held while it moves, which its emulation keeps pressed.
    pub mask: u8,
    /// Where the plan expects it to start.
    pub from: (u16, u16),
}
impl Relative {
    /// Where the motion is after `step` of its steps, from where it started.
    fn offset(&self, step: i64) -> (i64, i64) {
        let n = i64::from(self.steps);
        (i64::from(self.dx) * step / n, i64::from(self.dy) * step / n)
    }
    /// Each step's own move.
    pub fn moves(&self) -> Vec<(i32, i32)> {
        (1..=i64::from(self.steps))
            .map(|step| {
                let ((x, y), (px, py)) = (self.offset(step), self.offset(step - 1));
                ((x - px) as i32, (y - py) as i32)
            })
            .collect()
    }
    /// The absolute positions that emulate it from `start`, clamped to the desktop.
    pub fn emulated(&self, start: (u16, u16), width: u16, height: u16) -> Vec<(u16, u16)> {
        (1..=i64::from(self.steps))
            .map(|step| {
                let (x, y) = self.offset(step);
                (
                    (i64::from(start.0) + x).clamp(0, i64::from(width) - 1) as u16,
                    (i64::from(start.1) + y).clamp(0, i64::from(height) - 1) as u16,
                )
            })
            .collect()
    }
}
pub struct Planned {
    pub(crate) steps: Vec<Step>,
    pub released_keys: Vec<String>,
    pub released_buttons: Vec<&'static str>,
    /// Where the plan leaves the pointer; relative motion through XTest may leave it elsewhere.
    pub pointer: Option<(u16, u16)>,
    pub delay_ms: u64,
}
struct Planner {
    width: u16,
    height: u16,
    steps: Vec<Step>,
    delay: u64,
    chars: usize,
    pos: Option<(u16, u16)>,
    mask: u8,
    held: Vec<(String, u32)>,
}
impl Planner {
    fn check(&self, x: u16, y: u16) -> anyhow::Result<()> {
        if x >= self.width || y >= self.height {
            bail!(
                "Coordinates ({x}, {y}) are outside the {}x{} desktop",
                self.width,
                self.height
            );
        }
        Ok(())
    }
    fn pointer(&mut self, mask: u8, x: u16, y: u16, pause: u64) {
        self.steps.push(Step::Pointer { mask, x, y, pause });
        self.pos = Some((x, y));
        self.delay += pause;
    }
    /// Send the buttons now held, at x,y or wherever the pointer is.
    fn buttons(&mut self, x: Option<u16>, y: Option<u16>) -> anyhow::Result<()> {
        match (x, y) {
            (Some(x), Some(y)) => {
                self.check(x, y)?;
                self.pointer(self.mask, x, y, 0);
            }
            (None, None) => {
                let expected = self
                    .pos
                    .context("Provide x and y, or move the pointer earlier in the request")?;
                self.steps.push(Step::Buttons {
                    mask: self.mask,
                    expected,
                });
            }
            _ => bail!("Provide both x and y"),
        }
        Ok(())
    }
    fn keys(keys: &[String]) -> anyhow::Result<Vec<u32>> {
        if keys.is_empty() || keys.len() > 8 {
            bail!("A key action needs 1 to 8 keys");
        }
        let symbols = keys
            .iter()
            .map(|k| keysym(k))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let unique: std::collections::HashSet<_> = symbols.iter().collect();
        if unique.len() != symbols.len() {
            bail!("Do not repeat the same key in a chord");
        }
        Ok(symbols)
    }
    fn wait(&mut self, milliseconds: u64) -> anyhow::Result<()> {
        if milliseconds > MAX_WAIT_MS {
            bail!("A single wait or hold is limited to {MAX_WAIT_MS} milliseconds");
        }
        self.steps.push(Step::Wait(milliseconds));
        self.delay += milliseconds;
        Ok(())
    }
    fn release_all(&mut self) -> (Vec<String>, Vec<&'static str>) {
        let mut bytes = Vec::new();
        let mut keys = Vec::new();
        for (name, symbol) in self.held.drain(..).rev() {
            bytes.extend(key(false, symbol));
            keys.push(name);
        }
        if !bytes.is_empty() {
            self.steps.push(Step::Keys(bytes));
        }
        let mut buttons = Vec::new();
        if self.mask != 0 {
            buttons.extend(
                Button::ALL
                    .iter()
                    .filter(|b| self.mask & b.mask() != 0)
                    .map(|b| b.name()),
            );
            self.mask = 0;
            self.steps.push(Step::Buttons {
                mask: 0,
                expected: self.pos.unwrap_or_default(),
            });
        }
        (keys, buttons)
    }
}
pub(crate) fn plan(
    actions: &[Action],
    width: u16,
    height: u16,
    start: Option<(u16, u16)>,
) -> anyhow::Result<Planned> {
    if actions.is_empty() || actions.len() > 100 {
        bail!("Send 1 to 100 desktop actions");
    }
    let mut p = Planner {
        width,
        height,
        steps: Vec::new(),
        delay: 0,
        chars: 0,
        pos: start.filter(|(x, y)| *x < width && *y < height),
        mask: 0,
        held: Vec::new(),
    };
    let (mut released_keys, mut released_buttons) = (Vec::new(), Vec::new());
    for action in actions {
        match action {
            Action::Move { x, y } => {
                p.check(*x, *y)?;
                p.pointer(p.mask, *x, *y, 0);
            }
            Action::MoveRelative {
                dx,
                dy,
                steps,
                duration_ms,
            } => {
                let (x0, y0) = p.pos.context(
                    "Relative movement needs a known pointer position; move or click with coordinates first",
                )?;
                if !(1..=50).contains(steps) {
                    bail!("Relative movement steps must be 1 to 50");
                }
                if *duration_ms > 2000 {
                    bail!("Relative movement duration must be at most 2000 milliseconds");
                }
                let motion = Relative {
                    dx: *dx,
                    dy: *dy,
                    steps: *steps,
                    pause: duration_ms / u64::from(*steps),
                    mask: p.mask,
                    from: (x0, y0),
                };
                // Where the pointer itself stops; relative motion goes on past the edge.
                p.pos = motion.emulated((x0, y0), width, height).last().copied();
                p.delay += motion.pause * u64::from(*steps);
                p.steps.push(Step::Relative(motion));
            }
            Action::Click {
                x,
                y,
                button,
                count,
            } => {
                p.check(*x, *y)?;
                if !(1..=3).contains(count) {
                    bail!("Click count must be 1 to 3");
                }
                for _ in 0..*count {
                    p.pointer(p.mask | button.mask(), *x, *y, 40);
                    p.pointer(p.mask, *x, *y, 80);
                }
            }
            Action::ButtonDown { x, y, button } => {
                p.mask |= button.mask();
                p.buttons(*x, *y)?;
            }
            Action::ButtonUp { x, y, button } => {
                p.mask &= !button.mask();
                p.buttons(*x, *y)?;
            }
            Action::Drag {
                from_x,
                from_y,
                to_x,
                to_y,
                button,
                duration_ms,
            } => {
                p.check(*from_x, *from_y)?;
                p.check(*to_x, *to_y)?;
                if !(100..=2000).contains(duration_ms) {
                    bail!("Drag duration must be 100 to 2000 milliseconds");
                }
                let pressed = p.mask | button.mask();
                p.pointer(pressed, *from_x, *from_y, 0);
                for n in 1..=20 {
                    let interpolate = |a: u16, b: u16| {
                        (i32::from(a) + (i32::from(b) - i32::from(a)) * n / 20) as u16
                    };
                    p.pointer(
                        pressed,
                        interpolate(*from_x, *to_x),
                        interpolate(*from_y, *to_y),
                        duration_ms / 20,
                    );
                }
                p.pointer(p.mask, *to_x, *to_y, 0);
            }
            Action::Scroll {
                x,
                y,
                direction,
                steps,
            } => {
                p.check(*x, *y)?;
                if !(1..=50).contains(steps) {
                    bail!("Scroll steps must be 1 to 50");
                }
                let wheel = match direction {
                    Direction::Up => 8,
                    Direction::Down => 16,
                    Direction::Left => 32,
                    Direction::Right => 64,
                };
                for _ in 0..*steps {
                    p.pointer(p.mask | wheel, *x, *y, 10);
                    p.pointer(p.mask, *x, *y, 10);
                }
            }
            Action::KeyPress { keys } => {
                let symbols = Planner::keys(keys)?;
                let mut bytes = Vec::new();
                for symbol in &symbols {
                    bytes.extend(key(true, *symbol));
                }
                for symbol in symbols.iter().rev() {
                    bytes.extend(key(false, *symbol));
                }
                p.steps.push(Step::Keys(bytes));
            }
            Action::KeyDown { keys } => {
                let symbols = Planner::keys(keys)?;
                let mut bytes = Vec::new();
                for (name, symbol) in keys.iter().zip(symbols) {
                    if p.held.iter().any(|(_, held)| *held == symbol) {
                        continue;
                    }
                    bytes.extend(key(true, symbol));
                    p.held.push((name.clone(), symbol));
                }
                p.steps.push(Step::Keys(bytes));
            }
            Action::KeyUp { keys } => {
                let symbols = Planner::keys(keys)?;
                let mut bytes = Vec::new();
                for symbol in symbols {
                    bytes.extend(key(false, symbol));
                    p.held.retain(|(_, held)| *held != symbol);
                }
                p.steps.push(Step::Keys(bytes));
            }
            Action::HoldKeys { keys, milliseconds } => {
                if *milliseconds == 0 {
                    bail!("Hold keys for at least 1 millisecond");
                }
                let symbols = Planner::keys(keys)?;
                let mut down = Vec::new();
                let mut up = Vec::new();
                for symbol in &symbols {
                    down.extend(key(true, *symbol));
                }
                for symbol in symbols.iter().rev() {
                    up.extend(key(false, *symbol));
                }
                p.steps.push(Step::Keys(down));
                p.wait(*milliseconds)?;
                p.steps.push(Step::Keys(up));
            }
            Action::TypeText { text } => {
                p.chars += text.chars().count();
                if p.chars > 10000 {
                    bail!("Type at most 10000 characters per request");
                }
                let mut bytes = Vec::new();
                for c in text.chars() {
                    let symbol = keysym(&c.to_string())?;
                    bytes.extend(key(true, symbol));
                    bytes.extend(key(false, symbol));
                }
                p.steps.push(Step::Keys(bytes));
            }
            Action::Wait { milliseconds } => p.wait(*milliseconds)?,
            Action::ReleaseAll => {
                let (keys, buttons) = p.release_all();
                released_keys.extend(keys);
                released_buttons.extend(buttons);
            }
        }
        if p.delay > MAX_TOTAL_DELAY_MS {
            bail!(
                "Desktop actions may wait at most {} seconds in total",
                MAX_TOTAL_DELAY_MS / 1000
            );
        }
    }
    // Nothing stays pressed after the connection closes; report what the request left held.
    let (keys, buttons) = p.release_all();
    released_keys.extend(keys);
    released_buttons.extend(buttons);
    Ok(Planned {
        steps: p.steps,
        released_keys,
        released_buttons,
        pointer: p.pos,
        delay_ms: p.delay,
    })
}
/// True when the first positional action relies on the pointer's current location.
pub(crate) fn needs_start_position(actions: &[Action]) -> bool {
    for action in actions {
        match action {
            Action::Move { .. }
            | Action::Click { .. }
            | Action::Drag { .. }
            | Action::Scroll { .. }
            | Action::ButtonDown {
                x: Some(_),
                y: Some(_),
                ..
            }
            | Action::ButtonUp {
                x: Some(_),
                y: Some(_),
                ..
            } => return false,
            Action::MoveRelative { .. } | Action::ButtonDown { .. } | Action::ButtonUp { .. } => {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// A shared connection to a desktop's VNC server past its ServerInit, before any encoding is
/// asked for: the screen's size and how many bytes each pixel the server sends takes.
pub(crate) struct Connection {
    pub stream: TcpStream,
    pub width: u16,
    pub height: u16,
    pub pixel_bytes: usize,
}

/// Open a shared RFB 3.8 connection without authentication, which the desktop's VNC server
/// allows on its private network only, so the browser's viewer stays connected.
pub(crate) async fn handshake(address: &str) -> anyhow::Result<Connection> {
    let mut stream = TcpStream::connect(address).await?;
    stream.set_nodelay(true)?;
    let mut version = [0u8; 12];
    stream.read_exact(&mut version).await?;
    if &version != b"RFB 003.008\n" {
        bail!("Desktop input requires RFB 3.8");
    }
    stream.write_all(b"RFB 003.008\n").await?;
    let count = stream.read_u8().await?;
    if count == 0 {
        bail!("VNC rejected the connection");
    }
    let mut types = vec![0; count as usize];
    stream.read_exact(&mut types).await?;
    if !types.contains(&1) {
        bail!("Managed VNC must support its private no-auth connection");
    }
    stream.write_all(&[1]).await?;
    if stream.read_u32().await? != 0 {
        bail!("VNC handshake failed");
    }
    stream.write_all(&[1]).await?; // Shared session; keep the browser's viewer connected.
    let (width, height) = (stream.read_u16().await?, stream.read_u16().await?);
    let mut pixel_format = [0; 16];
    stream.read_exact(&mut pixel_format).await?;
    let name_len = stream.read_u32().await? as usize;
    if name_len > 65536 {
        bail!("VNC desktop name is too long");
    }
    let mut name = vec![0; name_len];
    stream.read_exact(&mut name).await?;
    Ok(Connection {
        stream,
        width,
        height,
        pixel_bytes: usize::from(pixel_format[0] / 8),
    })
}
async fn connect(address: &str) -> anyhow::Result<(TcpStream, u16, u16)> {
    let Connection {
        mut stream,
        width,
        height,
        ..
    } = handshake(address).await?;
    // Negotiate Fence to acknowledge that preceding input has been processed.
    let mut encodings = vec![2, 0, 0, 1];
    encodings.extend((-312i32).to_be_bytes());
    stream.write_all(&encodings).await?;
    Ok((stream, width, height))
}
async fn fence(stream: &mut TcpStream) -> anyhow::Result<()> {
    let marker = b"thoric";
    let mut request = vec![248, 0, 0, 0];
    request.extend(0x80000001u32.to_be_bytes());
    request.push(marker.len() as u8);
    request.extend(marker);
    stream.write_all(&request).await?;
    stream.flush().await?;
    loop {
        match stream.read_u8().await? {
            248 => {
                let mut padding = [0; 3];
                stream.read_exact(&mut padding).await?;
                let flags = stream.read_u32().await?;
                let length = stream.read_u8().await?;
                let mut data = vec![0; length as usize];
                stream.read_exact(&mut data).await?;
                if flags & 0x80000000 == 0 && data == marker {
                    return Ok(());
                }
                if flags & 0x80000000 != 0 {
                    let mut response = vec![248, 0, 0, 0];
                    response.extend((flags & 3).to_be_bytes());
                    response.push(length);
                    response.extend(data);
                    stream.write_all(&response).await?;
                }
            }
            2 => {} // Bell.
            3 => {
                let mut padding = [0; 3];
                stream.read_exact(&mut padding).await?;
                let length = stream.read_u32().await? as usize;
                if length > 65536 {
                    bail!("VNC clipboard message exceeds the input limit");
                }
                let mut text = vec![0; length];
                stream.read_exact(&mut text).await?;
            }
            message => bail!("Unexpected VNC message during input acknowledgement: {message}"),
        }
    }
}
async fn pointer_position(app: &App, env: &crate::model::Container) -> anyhow::Result<(u16, u16)> {
    let container = relative::docker_id(app, env).await?;
    let output = app
        .docker(&[
            "exec",
            "--user",
            "desktop",
            "--env",
            "DISPLAY=:0",
            &container,
            "xdotool",
            "getmouselocation",
            "--shell",
        ])
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Relative pointer actions need the current pointer position, which requires xdotool in the desktop image; move or click with coordinates first: {e:#}"
            )
        })?;
    relative::location(&String::from_utf8_lossy(&output))
        .context("Could not read the pointer position")
}
/// How a request's steps went while they were sent.
#[derive(Default)]
struct Sending {
    /// Where relative motion through XTest left the pointer, which later button changes use
    /// instead of where the plan expects it; none while the plan's positions hold.
    here: Option<(u16, u16)>,
    /// The desktop's Docker ID, read for its first relative motion.
    container: Option<String>,
    /// Why relative motion is emulated, once XTest could not move the pointer.
    emulated: Option<String>,
    moved_relatively: bool,
}
async fn pause(milliseconds: u64) {
    if milliseconds > 0 {
        tokio::time::sleep(Duration::from_millis(milliseconds)).await;
    }
}
/// Send a plan's steps: RFB messages over the VNC connection and relative motion through XTest,
/// in order, falling back to emulating the motion with positions when XTest cannot be used.
async fn send(
    app: &App,
    env: &crate::model::Container,
    stream: &mut TcpStream,
    planned: &Planned,
    (width, height): (u16, u16),
) -> anyhow::Result<Sending> {
    let mut sending = Sending::default();
    for step in &planned.steps {
        match step {
            Step::Keys(bytes) => stream.write_all(bytes).await?,
            Step::Pointer {
                mask,
                x,
                y,
                pause: wait,
            } => {
                stream.write_all(&pointer(*mask, *x, *y)).await?;
                sending.here = None;
                pause(*wait).await;
            }
            Step::Buttons { mask, expected } => {
                let (x, y) = sending.here.unwrap_or(*expected);
                stream.write_all(&pointer(*mask, x, y)).await?;
            }
            Step::Wait(wait) => pause(*wait).await,
            Step::Relative(motion) => {
                sending.moved_relatively = true;
                if sending.emulated.is_none() {
                    // The input sent so far is processed before the motion begins.
                    fence(stream).await?;
                    let moved = async {
                        if sending.container.is_none() {
                            sending.container = Some(relative::docker_id(app, env).await?);
                        }
                        let container = sending.container.as_deref().unwrap_or_default();
                        relative::move_pointer(app, container, motion).await
                    }
                    .await;
                    match moved {
                        Ok(at) => {
                            sending.here = Some(at);
                            continue;
                        }
                        Err(error) => sending.emulated = Some(format!("{error:#}")),
                    }
                }
                let start = sending.here.unwrap_or(motion.from);
                for (x, y) in motion.emulated(start, width, height) {
                    stream.write_all(&pointer(motion.mask, x, y)).await?;
                    sending.here = Some((x, y));
                    pause(motion.pause).await;
                }
            }
        }
    }
    Ok(sending)
}
/// Send actions through a shared VNC connection; used by the HTTP route, MCP and act-and-observe.
pub async fn perform(app: &Arc<App>, id: &str, actions: &[Action]) -> Result<Value> {
    // Input shares the container with a script, an upload or an application, and waits only for
    // work that changes or replaces it, such as a reset or a snapshot. Waiting for that with one
    // of the four permits taken would leave the other containers without input, so the wait is
    // bounded and comes first. Two input requests to one desktop still take turns, so the keys
    // one holds never reach the other's typing.
    let container = format!("container:{id}");
    let _guards = tokio::time::timeout(LOCK_WAIT, async {
        let shared = app.resource_lock(&container).share_owned().await;
        let turn = app.resource_lock(&format!("input:{id}")).lock_owned().await;
        (shared, turn)
    })
    .await
    .map_err(|_| crate::jobs::busy(&container))?;
    let _permit = app.desktop_inputs.try_acquire().map_err(|_| {
        Error(
            StatusCode::TOO_MANY_REQUESTS,
            "Four desktop input requests are already active".into(),
        )
    })?;
    let env = app.container(id)?;
    let address = app.address(&env).await?;
    let start = if needs_start_position(actions) {
        Some(pointer_position(app, &env).await?)
    } else {
        None
    };
    // Each relative movement is a `docker exec` of its own, which takes time beyond its pauses.
    let movements = actions
        .iter()
        .filter(|action| matches!(action, Action::MoveRelative { .. }))
        .count() as u32;
    let limit =
        Duration::from_millis(MAX_TOTAL_DELAY_MS + 15_000) + relative::ALLOWANCE * movements;
    let result = tokio::time::timeout(limit, async {
        let (mut stream, width, height) = connect(&address).await?;
        let planned = plan(actions, width, height, start)?;
        let sent = send(app, &env, &mut stream, &planned, (width, height)).await?;
        fence(&mut stream).await?;
        stream.shutdown().await?;
        let mut result = json!({
            "container_id": id,
            "applied_actions": actions.len(),
            "width": width,
            "height": height,
            "pointer": sent.here.or(planned.pointer).map(|(x, y)| json!({"x": x, "y": y})),
            "released": {"keys": planned.released_keys, "buttons": planned.released_buttons},
            "delay_ms": planned.delay_ms,
        });
        if sent.moved_relatively {
            result["relative_motion"] = match sent.emulated {
                None => json!({"method": "xtest"}),
                Some(reason) => json!({"method": "emulated", "reason": reason}),
            };
        }
        anyhow::Ok(result)
    })
    .await
    .context("VNC input timed out; inspect the desktop before retrying")??;
    Ok(result)
}
pub async fn input(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<Input>,
) -> Result<Json<Value>> {
    Ok(Json(perform(&app, &id, &body.actions).await?))
}

#[cfg(test)]
#[path = "desktop_input_tests.rs"]
mod tests;
