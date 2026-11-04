//! Relative pointer motion through XTest inside the desktop.
//!
//! RFB carries only absolute pointer positions, so motion sent over the VNC session stops at the
//! desktop's edge and reaches an application as a move to a position. Games and other programs
//! that grab the pointer read XInput2 raw motion instead, the deltas a mouse reports, which
//! XTest's relative motion carries even where the pointer itself can go no further. TigerVNC's
//! server reads no input devices of its own, so a uinput mouse would not reach it; xdotool's
//! `mousemove_relative` sends exactly that motion. It runs in one `docker exec` as the desktop
//! user per movement, its steps chained with xdotool's own pauses, and reports where the pointer
//! ended so later button changes over VNC happen there.
use super::Relative;
use crate::{App, model::Container, process::Overflow};
use anyhow::{Context, bail};
use std::time::Duration;

/// What a movement's exec may take beyond its own pauses, and what each movement adds to the
/// time a whole input request may take.
pub(super) const ALLOWANCE: Duration = Duration::from_secs(5);

/// xdotool's command line for a movement: each step's move and the pause after it, then where the
/// pointer is.
pub(super) fn command(motion: &Relative) -> Vec<String> {
    let mut args = vec!["xdotool".to_owned()];
    for (dx, dy) in motion.moves() {
        if (dx, dy) != (0, 0) {
            // `--` lets a negative offset through, and the next command follows the two numbers.
            args.extend(["mousemove_relative".into(), "--".into()]);
            args.extend([dx.to_string(), dy.to_string()]);
        }
        if motion.pause > 0 {
            args.extend([
                "sleep".into(),
                format!("{:.3}", motion.pause as f64 / 1000.0),
            ]);
        }
    }
    args.extend(["getmouselocation".into(), "--shell".into()]);
    args
}

/// The pointer's position in what `xdotool getmouselocation --shell` prints.
pub(super) fn location(text: &str) -> Option<(u16, u16)> {
    let value = |prefix: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(prefix))
            .and_then(|v| v.trim().parse::<i64>().ok())
            .map(|v| v.clamp(0, i64::from(u16::MAX)) as u16)
    };
    Some((value("X=")?, value("Y=")?))
}

/// Move the pointer of `container`, the desktop's Docker ID, and return where it ended.
pub(super) async fn move_pointer(
    app: &App,
    container: &str,
    motion: &Relative,
) -> anyhow::Result<(u16, u16)> {
    let mut args: Vec<String> = [
        "exec",
        "--user",
        "desktop",
        "--env",
        "DISPLAY=:0",
        container,
    ]
    .map(str::to_owned)
    .into();
    args.extend(command(motion));
    let pauses = Duration::from_millis(motion.pause * u64::from(motion.steps));
    let moving = crate::process::capture(
        app.command(&args),
        pauses + ALLOWANCE,
        64 * 1024,
        Overflow::Reject,
    );
    let output = crate::display::watched(app, container, moving).await?;
    if !output.status.success() {
        bail!(
            "xdotool could not move the pointer: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    location(&String::from_utf8_lossy(&output.stdout))
        .context("xdotool did not report where the pointer is")
}

/// The desktop's Docker ID, which the container record does not keep.
pub(super) async fn docker_id(app: &App, env: &Container) -> anyhow::Result<String> {
    let info = app.owned(env).await?;
    Ok(info["Id"]
        .as_str()
        .context("Missing container ID")?
        .to_owned())
}

#[cfg(test)]
#[path = "relative_tests.rs"]
mod tests;
