//! Whether a desktop's X display answers, and which programs are connected to it when it does not.
//!
//! A program can hold an X server with a server grab: ImageMagick's import does while it waits for
//! a window to be clicked, which it falls back to when the window it was given is gone. The server
//! then serves that program alone, and every other program that connects waits without an error:
//! the health check's xdpyinfo, xdotool and wmctrl behind the window tools, the screenshot's xwd and
//! the clipboard's GTK. Docker's limit would end such a command after minutes with an error that
//! blames Docker. Instead, a command on the display that has not finished after [`PATIENCE`] has
//! the display looked at meanwhile ([`watched`]): one connection setup within [`PROBE`] seconds in
//! `display_check.py`, as the desktop user. A display that does not answer ends the command with a
//! conflict that names the programs connected to it, newest first, the applications the manager
//! launched by their ID, and says how to let it go; a display that answers lets the command go on.
use crate::{
    App,
    error::Fault,
    process::{self, Overflow},
};
use anyhow::{Context, bail};
use axum::http::StatusCode;
use serde::Deserialize;
use std::{future::Future, time::Duration};

const HELPER: &str = include_str!("../helpers/display_check.py");
/// How long a command on the display runs before the display is looked at. Window tools answer in
/// a fraction of a second and a screenshot of a large desktop in about one.
pub(crate) const PATIENCE: Duration = Duration::from_secs(3);
/// The seconds a connection setup has to be answered in, as the health check gives xdpyinfo.
const PROBE: &str = "3";
/// What looking at the display may take, the probe and the listing of its connections included.
const LOOK_LIMIT: Duration = Duration::from_secs(15);
/// How many of the programs that do not wait for the display a refusal names.
const NAMED: usize = 3;

/// What `display_check.py` found.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "display", rename_all = "lowercase")]
enum Display {
    Answers,
    Held {
        #[serde(default)]
        clients: Vec<Client>,
    },
    Gone,
}

/// A program connected to the display.
#[derive(Debug, Deserialize, PartialEq)]
struct Client {
    pid: u32,
    command: String,
    age_seconds: u64,
    /// Whether the display keeps it waiting: every connection it holds has bytes the server has
    /// not read. The program that holds the display is among the others.
    waiting: bool,
    #[serde(default)]
    application_id: Option<String>,
}

/// Look at the display of the desktop whose Docker ID is `container`.
async fn look(app: &App, container: &str) -> anyhow::Result<Display> {
    let args = [
        "exec", "--user", "desktop", container, "python3", "-I", "-c", HELPER, PROBE,
    ]
    .map(str::to_owned);
    let output =
        process::capture(app.command(&args), LOOK_LIMIT, 256 * 1024, Overflow::Reject).await?;
    if !output.status.success() {
        bail!(
            "The display check failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout).context("The display check answered something else")
}

/// The error a command on a display that does not answer ends with, or `None` when it answers.
fn refusal(display: &Display) -> Option<anyhow::Error> {
    let message = match display {
        Display::Answers => return None,
        Display::Gone => {
            "The desktop's display is not running: nothing answers on :0. Restart the container."
                .to_owned()
        }
        Display::Held { clients } => held(clients),
    };
    Some(Fault::error(StatusCode::CONFLICT, message))
}

/// What to say of a display a program holds, with the programs that may be holding it.
fn held(clients: &[Client]) -> String {
    let mut text = String::from(
        "The desktop's display is not answering: a program holds the X server, and window tools, screenshots, the clipboard and the health check wait until it lets go.",
    );
    let named = clients
        .iter()
        .filter(|client| !client.waiting)
        .take(NAMED)
        .map(describe)
        .collect::<Vec<_>>();
    if !named.is_empty() {
        text.push_str(" Connected to it, newest first: ");
        text.push_str(&named.join("; "));
        text.push('.');
    }
    let waiting = clients.iter().filter(|client| client.waiting).count();
    if waiting > 0 {
        text.push_str(&format!(
            " {waiting} other {} for it.",
            if waiting == 1 {
                "program waits"
            } else {
                "programs wait"
            }
        ));
    }
    text.push_str(" End the program that holds it, with stop_application when it was launched as an application and kill_process with its pid otherwise. A program that waits for a window to be picked with the mouse, such as ImageMagick's import given a window ID that no longer exists, also lets go after one click on the desktop.");
    text
}

fn describe(client: &Client) -> String {
    let application = client
        .application_id
        .as_deref()
        .map(|id| format!("application {id}, "))
        .unwrap_or_default();
    format!(
        "{} (pid {}, {application}started {} ago)",
        client.command,
        client.pid,
        age(client.age_seconds)
    )
}

fn age(seconds: u64) -> String {
    match seconds {
        0..90 => format!("{seconds} s"),
        90..5400 => format!("{} min", (seconds + 30) / 60),
        _ => format!("{} h", (seconds + 1800) / 3600),
    }
}

/// Run `work`, a command on the display of the desktop whose Docker ID is `container`. Once it has
/// taken [`PATIENCE`], the display is looked at while it goes on, and a display that does not
/// answer ends it with the refusal that says why; one that answers, or a look that fails, leaves
/// it to finish as it would have.
pub(crate) async fn watched<T, E>(
    app: &App,
    container: &str,
    work: impl Future<Output = Result<T, E>>,
) -> Result<T, E>
where
    E: From<anyhow::Error>,
{
    // On the heap, so the futures of the tools that wait on it keep their size.
    let mut work = Box::pin(work);
    if let Ok(done) = tokio::time::timeout(PATIENCE, &mut work).await {
        return done;
    }
    let mut looking = Box::pin(look(app, container));
    tokio::select! {
        biased;
        done = &mut work => done,
        seen = &mut looking => match seen {
            Ok(display) => match refusal(&display) {
                Some(refused) => Err(refused.into()),
                None => work.await,
            },
            Err(error) => {
                tracing::debug!("Could not look at the display of {container}: {error:#}");
                work.await
            }
        },
    }
}

#[cfg(test)]
#[path = "display_tests.rs"]
mod tests;
