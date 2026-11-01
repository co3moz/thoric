//! A desktop's resolution, which its settings name and its VNC server changes while the desktop
//! runs. Xtigervnc takes an RFB `SetDesktopSize` from any client at every desktop image level, as
//! the dashboard's **Resize to window** shows, so a settings change of the resolution alone needs
//! no new container. Docker keeps the size the container was created with in
//! `DESKTOP_RESOLUTION`, which the desktop starts at, so a desktop that started is given its
//! settings' size again: by the job that started it, through Docker's `start` event when Docker
//! restarted it alone, and by a look at every running desktop whenever Docker's events begin to
//! flow, for the starts no event told the manager of. A desktop whose screen no longer has the
//! size it was created with, such as one a viewer resized to their window, is left as it is.
use crate::{
    App,
    desktop_input::{Connection, handshake},
    model::{Container, LABEL},
};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

/// How long opening the VNC connection and changing the screen may take together.
const TIMEOUT: Duration = Duration::from_secs(15);
/// The RFB pseudo-encoding through which a server reports its screens and a client changes them.
const EXTENDED_DESKTOP_SIZE: i32 = -308;
/// How many messages from the server are read for the answer it owes.
const MESSAGES: usize = 64;

/// A `WIDTHxHEIGHT` resolution as a width and a height.
pub(crate) fn size(resolution: &str) -> Result<(u16, u16)> {
    let (width, height) = resolution
        .split_once('x')
        .context("Resolution must use WIDTHxHEIGHT")?;
    Ok((width.parse()?, height.parse()?))
}

/// The resolution Docker created the container with, from `docker inspect`.
pub(crate) fn created_with(info: &Value) -> Option<(u16, u16)> {
    info["Config"]["Env"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find_map(|entry| entry.strip_prefix("DESKTOP_RESOLUTION="))
        .and_then(|value| size(value).ok())
}

/// Whether a container's settings name another resolution than the one Docker created it with.
fn differs(env: &Container, info: &Value) -> bool {
    created_with(info)
        .is_some_and(|created| size(&env.spec.resolution).is_ok_and(|wanted| wanted != created))
}

/// What an `ExtendedDesktopSize` rectangle says: why the screens changed (1 answers this client's
/// own request), the status of that request, and each screen's ID and flags.
#[derive(Debug, PartialEq)]
struct Layout {
    reason: u16,
    status: u16,
    screens: Vec<(u32, u32)>,
}

/// The screen of a desktop's VNC server, over a shared connection.
pub(crate) struct Screen(Connection);

impl Screen {
    pub(crate) async fn open(address: &str) -> Result<Self> {
        Ok(Self(handshake(address).await?))
    }

    pub(crate) fn size(&self) -> (u16, u16) {
        (self.0.width, self.0.height)
    }

    /// Change the screen's size: ask for the server's screens, send `SetDesktopSize` with the
    /// first of them covering the whole new size, and read the server's answer to this client.
    pub(crate) async fn resize(self, (width, height): (u16, u16)) -> Result<()> {
        let Connection {
            mut stream,
            pixel_bytes,
            ..
        } = self.0;
        let mut encodings = vec![2, 0, 0, 1];
        encodings.extend(EXTENDED_DESKTOP_SIZE.to_be_bytes());
        stream.write_all(&encodings).await?;
        // A request that is not incremental has the server report its screens.
        stream.write_all(&update_request(false)).await?;
        let current = layout(&mut stream, pixel_bytes, |_| true).await?;
        let &(id, flags) = current
            .screens
            .first()
            .context("The VNC server reported no screen")?;
        let mut message = vec![251, 0];
        message.extend(width.to_be_bytes());
        message.extend(height.to_be_bytes());
        message.extend([1, 0]);
        message.extend(id.to_be_bytes());
        message.extend([0; 4]);
        message.extend(width.to_be_bytes());
        message.extend(height.to_be_bytes());
        message.extend(flags.to_be_bytes());
        stream.write_all(&message).await?;
        // The answer travels in a framebuffer update, which a request lets the server send.
        stream.write_all(&update_request(true)).await?;
        let answer = layout(&mut stream, pixel_bytes, |layout| layout.reason == 1).await?;
        match answer.status {
            0 => Ok(()),
            1 => bail!("the VNC server does not allow its size to change"),
            2 => bail!("the VNC server lacks the resources for {width}x{height}"),
            3 => bail!("the VNC server refused {width}x{height} as a screen layout"),
            status => bail!("the VNC server refused {width}x{height} with status {status}"),
        }
    }
}

/// A request for the top left pixel only, whose content nothing here reads.
fn update_request(incremental: bool) -> [u8; 10] {
    [3, u8::from(incremental), 0, 0, 0, 0, 0, 1, 0, 1]
}

/// Read the server's messages until one carries a screen layout that `wanted` accepts.
async fn layout<S: AsyncRead + Unpin>(
    stream: &mut S,
    pixel_bytes: usize,
    wanted: impl Fn(&Layout) -> bool,
) -> Result<Layout> {
    for _ in 0..MESSAGES {
        if let Some(found) = layouts(stream, pixel_bytes)
            .await?
            .into_iter()
            .find(&wanted)
        {
            return Ok(found);
        }
    }
    bail!("The VNC server did not report its screens")
}

/// Read one message from the server and the screen layouts it carries.
async fn layouts<S: AsyncRead + Unpin>(stream: &mut S, pixel_bytes: usize) -> Result<Vec<Layout>> {
    let mut found = Vec::new();
    match stream.read_u8().await? {
        // FramebufferUpdate: rectangles, of pixels without an encoding asked for or of screens.
        0 => {
            skip(stream, 1).await?;
            for _ in 0..stream.read_u16().await? {
                let reason = stream.read_u16().await?;
                let status = stream.read_u16().await?;
                let width = u64::from(stream.read_u16().await?);
                let height = u64::from(stream.read_u16().await?);
                match stream.read_i32().await? {
                    0 => skip(stream, width * height * pixel_bytes as u64).await?,
                    EXTENDED_DESKTOP_SIZE => {
                        let count = stream.read_u8().await?;
                        skip(stream, 3).await?;
                        let mut screens = Vec::new();
                        for _ in 0..count {
                            let id = stream.read_u32().await?;
                            // Where the screen is and how large; the new size says it all.
                            skip(stream, 8).await?;
                            screens.push((id, stream.read_u32().await?));
                        }
                        found.push(Layout {
                            reason,
                            status,
                            screens,
                        });
                    }
                    encoding => bail!("The VNC server sent encoding {encoding} unasked"),
                }
            }
        }
        // SetColourMapEntries: six bytes a colour.
        1 => {
            skip(stream, 3).await?;
            let colours = stream.read_u16().await?;
            skip(stream, u64::from(colours) * 6).await?;
        }
        // Bell.
        2 => {}
        // ServerCutText: the clipboard, which this connection has no use for.
        3 => {
            skip(stream, 3).await?;
            let length = stream.read_u32().await?;
            skip(stream, u64::from(length)).await?;
        }
        message => bail!("Unexpected VNC message {message} while changing the resolution"),
    }
    Ok(found)
}

/// Read past `bytes` bytes without keeping them.
async fn skip<S: AsyncRead + Unpin>(stream: &mut S, bytes: u64) -> Result<()> {
    let read = tokio::io::copy(&mut (&mut *stream).take(bytes), &mut tokio::io::sink()).await?;
    if read < bytes {
        bail!("The VNC connection closed");
    }
    Ok(())
}

/// The address of a desktop's VNC server, on its private network.
fn vnc_address(env: &Container, info: &Value) -> Result<String> {
    Ok(format!("{}:5900", crate::docker::private_ip(env, info)?))
}

/// Change the screen at `address` to `wanted` while it still has the size it was `created` with,
/// and say whether it changed.
async fn settle(address: &str, created: (u16, u16), wanted: (u16, u16)) -> Result<bool> {
    let screen = Screen::open(address).await?;
    if screen.size() != created {
        return Ok(false);
    }
    screen.resize(wanted).await?;
    Ok(true)
}

impl App {
    /// Give the running desktop `resolution`, as a settings change of it does.
    pub(crate) async fn change_resolution(
        &self,
        env: &Container,
        info: &Value,
        resolution: &str,
    ) -> Result<()> {
        let wanted = size(resolution)?;
        let address = vnc_address(env, info)?;
        tokio::time::timeout(TIMEOUT, async {
            let screen = Screen::open(&address).await?;
            if screen.size() != wanted {
                screen.resize(wanted).await?;
            }
            anyhow::Ok(())
        })
        .await
        .context("The desktop's VNC server did not answer in time")?
    }

    /// Give a desktop that started the resolution its settings name, when they name another than
    /// the one Docker created the container with and its screen still has that one. Returns the
    /// resolution it set.
    pub(crate) async fn keep_resolution(
        &self,
        env: &Container,
        info: &Value,
    ) -> Result<Option<String>> {
        let Some(created) = created_with(info) else {
            return Ok(None);
        };
        if !differs(env, info) || info["State"]["Running"] != true {
            return Ok(None);
        }
        let address = vnc_address(env, info)?;
        let wanted = size(&env.spec.resolution)?;
        let changed = tokio::time::timeout(TIMEOUT, settle(&address, created, wanted))
            .await
            .context("The desktop's VNC server did not answer in time")??;
        Ok(changed.then(|| env.spec.resolution.clone()))
    }
}

/// Give a desktop Docker started the resolution its settings name once it is ready, in the
/// background: one that Docker restarted alone, or one a job started, which the job does too.
pub(crate) fn after_start(app: Arc<App>, id: String) {
    tokio::spawn(async move {
        if let Err(error) = settle_started(&app, &id).await {
            tracing::warn!(
                "Container {id} kept the resolution it was created with, not the one its settings name: {error:#}"
            );
        }
    });
}

/// Look at every running desktop, since Docker may have started one while its events did not
/// reach the manager, such as before the manager itself started.
pub(crate) fn after_gap(app: Arc<App>) {
    tokio::spawn(async move {
        let label = format!("label={LABEL}=true");
        let format = "{{.Label \"thoric.environment\"}}";
        let listed = app
            .docker(&["ps", "--filter", &label, "--format", format])
            .await;
        match listed {
            Ok(output) => {
                for id in String::from_utf8_lossy(&output).lines() {
                    if !id.is_empty() {
                        after_start(app.clone(), id.to_owned());
                    }
                }
            }
            Err(error) => tracing::debug!("Running desktops: {error:#}"),
        }
    });
}

async fn settle_started(app: &App, id: &str) -> Result<()> {
    let patience = &crate::desktop_ready::PATIENCE;
    let began = std::time::Instant::now();
    loop {
        // A container that is gone, stopped or created with its settings' size needs nothing.
        let Ok(env) = app.container(id) else {
            return Ok(());
        };
        let Some(info) = app.optional_inspect("container", &env.container).await? else {
            return Ok(());
        };
        if !differs(&env, &info) || info["State"]["Running"] != true {
            return Ok(());
        }
        let docker_id = info["Id"].as_str().context("Missing container ID")?;
        if info["State"]["Restarting"] != true && app.desktop_is_ready(docker_id).await {
            // A job that holds the container alone gives the desktop its size itself.
            let Ok(_shared) = app.share(&format!("container:{id}")).await else {
                return Ok(());
            };
            let Ok(env) = app.container(id) else {
                return Ok(());
            };
            let info = app.owned(&env).await?;
            if let Some(resolution) = app.keep_resolution(&env, &info).await? {
                tracing::info!(
                    "Gave \"{}\" the resolution {resolution} its settings name",
                    env.spec.name
                );
            }
            return Ok(());
        }
        if began.elapsed() >= patience.limit {
            tracing::debug!("Container {id} was not ready for its resolution");
            return Ok(());
        }
        tokio::time::sleep(patience.interval).await;
    }
}

#[cfg(test)]
#[path = "resolution_tests.rs"]
mod tests;
