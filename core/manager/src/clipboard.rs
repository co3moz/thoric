//! The desktop's clipboard, read and set as text by `helpers/clipboard.py` through GTK 3 as the
//! desktop user, so an agent can hand an application a long text in one paste instead of typing
//! it key by key. RFB's cut text is Latin-1, and its Extended Clipboard keeps a selection only
//! while the connection that set it stays open, which a request's does not; a set leaves an owner
//! process in the desktop instead, which serves the text until another program takes the
//! selection.
use crate::{
    App,
    desktop_input::{self, Action},
    error::{Error, Result},
    process,
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
use std::{sync::Arc, time::Duration};

const HELPER: &str = include_str!("../helpers/clipboard.py");
/// The most text read or set at once, in bytes of UTF-8. A set's text arrives in a JSON request
/// within [`crate::mcp::BODY_LIMIT`], as every request does, where a newline, a quote or a
/// backslash takes two bytes, so even a text made only of those fits.
pub const LIMIT: usize = 512 * 1024;
/// The helper's answer, JSON in ASCII: at most six bytes for each byte of the text.
const OUTPUT_LIMIT: usize = 6 * LIMIT + 64 * 1024;
/// The helper's exit codes for a failure that is not the request's, which it exits 1 for: a
/// selection's owner, or the owner it starts, that did not answer in time, and a display or owner
/// that failed.
const TIMED_OUT: i32 = 4;
const FAILED: i32 = 5;
/// How long the helper may run: its own wait of ten seconds for a selection's owner, and GTK's
/// start in a busy desktop.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The selection a read takes its text from.
#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Selection {
    #[default]
    Clipboard,
    Primary,
}
/// The selections a set gives the text to.
#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Selections {
    #[default]
    Clipboard,
    Primary,
    Both,
}
/// The shortcut that pastes the clipboard into the focused application: Ctrl+V in text fields,
/// editors and browsers, Ctrl+Shift+V in terminals such as xfce4-terminal and gnome-terminal.
#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum PasteKeys {
    #[default]
    #[serde(rename = "ctrl+v")]
    CtrlV,
    #[serde(rename = "ctrl+shift+v")]
    CtrlShiftV,
}

#[derive(Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetClipboard {
    /// "clipboard" is what Ctrl+C copies and Ctrl+V pastes; "primary" is the text last selected,
    /// which a middle click pastes.
    #[serde(default)]
    pub selection: Selection,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetClipboard {
    /// The text, at most 512 KiB of UTF-8 without NUL characters; any language, emoji included.
    pub text: String,
    /// "clipboard", "primary" (what a middle click pastes) or "both".
    #[serde(default)]
    pub selection: Selections,
    /// Then press paste_keys through the desktop's VNC session, so the focused application pastes
    /// the text. Focus the field or window first; needs selection clipboard or both.
    #[serde(default)]
    pub paste: bool,
    /// The shortcut paste presses: "ctrl+v" (default) or "ctrl+shift+v", which terminals take.
    #[serde(default)]
    pub paste_keys: PasteKeys,
}

impl Selection {
    fn id(self) -> &'static str {
        match self {
            Self::Clipboard => "clipboard",
            Self::Primary => "primary",
        }
    }
}
impl Selections {
    fn id(self) -> &'static str {
        match self {
            Self::Clipboard => "clipboard",
            Self::Primary => "primary",
            Self::Both => "both",
        }
    }
}
impl PasteKeys {
    fn id(self) -> &'static str {
        match self {
            Self::CtrlV => "ctrl+v",
            Self::CtrlShiftV => "ctrl+shift+v",
        }
    }
    /// The chord as desktop_input presses it. With Shift held the letter is the capital one, as
    /// a keyboard sends it, so the VNC server does not release Shift to type a small v.
    fn keys(self) -> Vec<String> {
        let keys: &[&str] = match self {
            Self::CtrlV => &["Control", "v"],
            Self::CtrlShiftV => &["Control", "Shift", "V"],
        };
        keys.iter().map(|key| (*key).to_owned()).collect()
    }
}
impl SetClipboard {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.text.len() > LIMIT {
            bail!(
                "Clipboard text is limited to {LIMIT} bytes of UTF-8; this text has {}",
                self.text.len()
            );
        }
        if self.text.contains('\0') {
            bail!("Clipboard text cannot contain NUL characters");
        }
        if self.paste && self.selection == Selections::Primary {
            bail!(
                "paste presses the shortcut that pastes the clipboard; set selection to clipboard or both"
            );
        }
        Ok(())
    }
}

/// The Docker ID of the running desktop.
async fn desktop(app: &App, id: &str) -> Result<String> {
    let env = app.container(id)?;
    let info = app.owned(&env).await?;
    if info["State"]["Running"] != true {
        return Err(Error(
            StatusCode::CONFLICT,
            "Start the container before using its clipboard".into(),
        ));
    }
    Ok(info["Id"]
        .as_str()
        .context("Missing container ID")?
        .to_owned())
}
/// Run the helper as the desktop user on the desktop's display, with the text on stdin.
async fn run(app: &App, id: &str, arguments: &[&str], input: Option<Vec<u8>>) -> Result<Value> {
    let _guard = app.share(&format!("container:{id}")).await?;
    let container = desktop(app, id).await?;
    let mut args = vec!["exec"];
    if input.is_some() {
        args.push("--interactive");
    }
    args.extend([
        "--user",
        "desktop",
        "--env",
        "DISPLAY=:0",
        &container,
        "python3",
        "-c",
        HELPER,
    ]);
    args.extend(arguments);
    let args = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let output = process::capture_input(
        app.command(&args),
        TIMEOUT,
        OUTPUT_LIMIT,
        process::Overflow::Reject,
        input,
    )
    .await?;
    if !output.status.success() {
        return Err(failure(&output));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| anyhow::anyhow!(e).into())
}
/// A failed helper's answer: what it printed, with the status its exit code names.
fn failure(output: &std::process::Output) -> Error {
    let Error(status, message) = Error::helper(output);
    let status = match output.status.code() {
        Some(TIMED_OUT) => StatusCode::GATEWAY_TIMEOUT,
        Some(FAILED) => StatusCode::BAD_GATEWAY,
        _ => status,
    };
    Error(status, message)
}

/// The text of the desktop's clipboard or PRIMARY selection, cut at [`LIMIT`] bytes.
pub async fn get_clipboard(app: &App, id: &str, options: &GetClipboard) -> Result<Value> {
    let limit = LIMIT.to_string();
    let mut result = run(app, id, &["get", options.selection.id(), &limit], None).await?;
    result["container_id"] = json!(id);
    Ok(result)
}
/// Give the desktop's selections the text, then paste it when asked.
pub async fn set_clipboard(app: &Arc<App>, id: &str, body: &SetClipboard) -> Result<Value> {
    body.validate()?;
    let limit = LIMIT.to_string();
    let input = body.text.as_bytes().to_vec();
    let mut result = run(app, id, &["set", body.selection.id(), &limit], Some(input)).await?;
    result["container_id"] = json!(id);
    if body.paste {
        // The share above has ended: input shares the container itself, and a second share while
        // a reset waits for the first would wait behind that reset.
        let keys = body.paste_keys;
        desktop_input::perform(app, id, &[Action::KeyPress { keys: keys.keys() }])
            .await
            .map_err(|Error(status, message)| {
                Error(
                    status,
                    format!(
                        "The clipboard holds the text, but pressing {} failed: {message}",
                        keys.id()
                    ),
                )
            })?;
        result["pasted"] = json!({"keys": keys.id()});
    }
    Ok(result)
}

pub async fn get(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Query(options): Query<GetClipboard>,
) -> Result<Json<Value>> {
    Ok(Json(get_clipboard(&app, &id, &options).await?))
}
pub async fn set(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<SetClipboard>,
) -> Result<Json<Value>> {
    Ok(Json(set_clipboard(&app, &id, &body).await?))
}

#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod tests;
