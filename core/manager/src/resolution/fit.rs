//! The viewers that fit a desktop's screen to their window, the dashboard's **Resize to window**.
//! A viewer says so on its own VNC socket, in a text frame beside the binary frames of RFB that
//! the manager relays as they are: `{"fit":true}` when the toggle is on and `{"fit":false}` when it
//! turns off, which `core/web/src/lib/desktop-fit.ts` writes and both sides hold to
//! `core/web/tests/desktop-fit.json`. The screen takes that viewer's size from its own RFB
//! `SetDesktopSize`; once no viewer fits it any more, because the last one turned the toggle off or
//! its socket closed, the desktop takes the resolution its settings name again. A closed socket
//! leaves the screen as it is for [`GRACE`] first, so a reload, whose new socket fits the screen
//! again, does not resize the desktop twice.
use crate::App;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};
use tokio::runtime::Handle;

/// How long a closed socket leaves the screen at its viewer's size, for a reload to fit it again.
const GRACE: Duration = Duration::from_secs(5);
/// How long the return waits for a job that holds the container alone, such as a snapshot.
const LOCK_WAIT: Duration = Duration::from_secs(600);
/// The longest text frame a viewer sends; anything longer is not one of its messages.
const MESSAGE_LIMIT: usize = 64;

/// How many viewers fit each desktop's screen to their window, in memory only. Its lock is held
/// only to count, never across an await.
#[derive(Default)]
pub(crate) struct Fitters(Mutex<HashMap<String, usize>>);

impl Fitters {
    fn counts(&self) -> MutexGuard<'_, HashMap<String, usize>> {
        // Nothing is half written while it is held.
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether a viewer fits the container's screen now.
    pub(crate) fn fitted(&self, id: &str) -> bool {
        self.counts().contains_key(id)
    }

    fn add(&self, id: &str) {
        *self.counts().entry(id.to_owned()).or_default() += 1;
    }

    /// One viewer fits the screen no more; says whether it was the last.
    fn remove(&self, id: &str) -> bool {
        let mut counts = self.counts();
        let Some(count) = counts.get_mut(id) else {
            return false;
        };
        *count = count.saturating_sub(1);
        if *count > 0 {
            return false;
        }
        counts.remove(id);
        true
    }
}

/// What a viewer says in a text frame of its VNC socket.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Message {
    fit: bool,
}

/// Whether one VNC socket fits its desktop's screen to its viewer's window. Dropped with the
/// socket, it gives the desktop its settings' resolution again when it was the last that did.
pub(crate) struct Fit {
    app: Arc<App>,
    id: String,
    on: bool,
}

impl Fit {
    /// A socket of the container `id`, which fits nothing until its viewer says so.
    pub(crate) fn new(app: Arc<App>, id: &str) -> Self {
        Self {
            app,
            id: id.to_owned(),
            on: false,
        }
    }

    /// Take a text frame of the socket, which ends it when it is not a viewer's message.
    pub(crate) fn message(&mut self, text: &str) -> Result<()> {
        if text.len() > MESSAGE_LIMIT {
            bail!("A VNC text frame carries a short JSON message");
        }
        let message: Message =
            serde_json::from_str(text).context("A VNC text frame says {\"fit\":true|false}")?;
        self.set(message.fit);
        Ok(())
    }

    fn set(&mut self, on: bool) {
        if on == self.on {
            return;
        }
        self.on = on;
        if on {
            self.app.fitters.add(&self.id);
        } else if self.app.fitters.remove(&self.id) {
            // The toggle went off in front of its viewer, who waits for the screen.
            spawn_return(self.app.clone(), self.id.clone(), Duration::ZERO);
        }
    }
}

impl Drop for Fit {
    fn drop(&mut self) {
        if self.on && self.app.fitters.remove(&self.id) {
            spawn_return(self.app.clone(), self.id.clone(), GRACE);
        }
    }
}

/// What became of a return to the settings' resolution.
#[derive(Debug, PartialEq)]
enum Returned {
    /// A viewer fits the screen again, so it keeps that viewer's size.
    Fitted,
    /// The container is gone, stopped, not ready or not Thoric's; it starts at its settings' size.
    Skipped,
    /// The screen had that size already.
    Kept,
    /// The screen took the resolution its settings name.
    Changed(String),
}

fn spawn_return(app: Arc<App>, id: String, wait: Duration) {
    // A socket dropped with the runtime, as the manager stops, leaves the screen as it is.
    let Ok(runtime) = Handle::try_current() else {
        return;
    };
    runtime.spawn(async move {
        match return_after(&app, &id, wait).await {
            Ok(Returned::Changed(resolution)) => tracing::info!(
                "Gave container {id} the resolution {resolution} its settings name, since no viewer fits it to their window any more"
            ),
            Ok(_) => {}
            Err(error) => tracing::warn!(
                "Container {id} kept the size a viewer fitted it to, not the resolution its settings name: {error:#}"
            ),
        }
    });
}

/// After `wait`, give the desktop the resolution its settings name, unless a viewer fits its
/// screen again by then.
async fn return_after(app: &App, id: &str, wait: Duration) -> Result<Returned> {
    tokio::time::sleep(wait).await;
    if app.fitters.fitted(id) {
        return Ok(Returned::Fitted);
    }
    // A job that holds the container alone, such as a settings change, may replace it meanwhile.
    let Ok(_shared) = app.share_for(&format!("container:{id}"), LOCK_WAIT).await else {
        bail!("another operation held the container for {LOCK_WAIT:?}");
    };
    if app.fitters.fitted(id) {
        return Ok(Returned::Fitted);
    }
    let Ok(env) = app.container(id) else {
        return Ok(Returned::Skipped);
    };
    let Some(info) = app.optional_inspect("container", &env.container).await? else {
        return Ok(Returned::Skipped);
    };
    let docker_id = info["Id"].as_str().unwrap_or_default();
    let running = info["State"]["Running"] == true && info["State"]["Restarting"] != true;
    if !running || !app.desktop_is_ready(docker_id).await {
        return Ok(Returned::Skipped);
    }
    let info = app.owned(&env).await?;
    Ok(
        if app
            .change_resolution(&env, &info, &env.spec.resolution)
            .await?
        {
            Returned::Changed(env.spec.resolution)
        } else {
            Returned::Kept
        },
    )
}

#[cfg(test)]
#[path = "fit_tests.rs"]
mod tests;
