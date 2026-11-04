use crate::{
    App,
    auth::origin_matches,
    error::{Error, Result},
};
use axum::{
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use std::{process::Stdio, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

pub async fn vnc(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    if !origin_matches(&headers) {
        return Err(Error(
            StatusCode::FORBIDDEN,
            "WebSocket origin rejected".into(),
        ));
    }
    let env = app.container(&id)?;
    let address = app.address(&env).await?;
    let stream = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(address))
        .await
        .map_err(|e| anyhow::anyhow!(e))?
        .map_err(|e| anyhow::anyhow!("VNC not ready: {e}"))?;
    stream.set_nodelay(true).map_err(|e| anyhow::anyhow!(e))?;
    let token = crate::auth::cookie(&headers);
    Ok(ws
        .protocols(["binary"])
        .max_message_size(4 * 1024 * 1024)
        .on_upgrade(move |socket| async move {
            // An open viewer uses the desktop, which its idle stop leaves running.
            let _using = app.lifecycle.open(&env.id);
            let fit = crate::resolution::Fit::new(app.clone(), &env.id);
            tokio::select! { _=app.session_revoked(token)=>{}, _=bridge(socket,stream,fit)=>{} }
        }))
}
/// Relay RFB between the viewer and the desktop's VNC server as binary frames. A text frame is
/// the viewer's own message to the manager, whether it fits the screen to its window, which
/// `fit` keeps until the socket ends.
async fn bridge(socket: WebSocket, stream: TcpStream, mut fit: crate::resolution::Fit) {
    let (mut sender, mut receiver) = socket.split();
    let (mut read, mut write) = stream.into_split();
    let from_desktop = async {
        let mut buf = vec![0; 65536];
        loop {
            let n = read.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            sender
                .send(Message::Binary(buf[..n].to_vec().into()))
                .await?;
        }
        anyhow::Ok(())
    };
    let to_desktop = async {
        while let Some(message) = receiver.next().await {
            match message? {
                Message::Binary(bytes) => write.write_all(&bytes).await?,
                Message::Close(_) => break,
                Message::Text(text) => fit.message(text.as_str())?,
                _ => {}
            }
        }
        anyhow::Ok(())
    };
    tokio::select! { result=from_desktop=>{if let Err(e)=result {tracing::debug!("VNC: {e}");}},result=to_desktop=>{if let Err(e)=result {tracing::debug!("VNC: {e}");}} }
}
pub async fn audio(
    State(app): State<Arc<App>>,
    Path((id, direction)): Path<(String, String)>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    if !origin_matches(&headers) {
        return Err(Error(
            StatusCode::FORBIDDEN,
            "WebSocket origin rejected".into(),
        ));
    }
    let env = app.container(&id)?;
    let info = app.owned(&env).await?;
    if info["State"]["Running"] != true {
        return Err(anyhow::anyhow!("Container is not running").into());
    }
    if !env.spec.audio || !app.config.audio {
        return Err(Error(StatusCode::BAD_REQUEST, "Audio is disabled".into()));
    }
    let capture = match direction.as_str() {
        "output" => true,
        "input" => false,
        _ => {
            return Err(Error(
                StatusCode::NOT_FOUND,
                "Audio channel not found".into(),
            ));
        }
    };
    if !capture && (!env.spec.microphone || !app.config.microphone) {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            "Microphone is disabled".into(),
        ));
    }
    let mut args = vec!["exec".into(), "-i".into()];
    args.extend([
        "--user".into(),
        "desktop".into(),
        "--env".into(),
        "PULSE_SERVER=unix:/run/desktop/pulse/native".into(),
        info["Id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing container ID"))?
            .to_owned(),
    ]);
    if capture {
        args.extend(
            [
                "python3",
                "-u",
                "-c",
                include_str!("../helpers/audio_output.py"),
            ]
            .map(str::to_owned),
        );
    } else {
        args.extend(
            [
                "pacat",
                "--playback",
                "--device=browser-mic-sink",
                "--format=float32le",
                "--rate=48000",
                "--channels=1",
                "--latency-msec=40",
            ]
            .map(str::to_owned),
        );
    }
    let child = app
        .command(&args)
        .stdin(Stdio::piped())
        .stdout(if capture {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!(e))?;
    let token = crate::auth::cookie(&headers);
    Ok(ws
        .max_message_size(65536)
        .on_upgrade(move |socket| async move {
            // Sound or a microphone that is connected uses the desktop.
            let _using = app.lifecycle.open(&env.id);
            pcm(socket, child, capture, app, token).await
        }))
}
async fn pcm(
    socket: WebSocket,
    mut child: tokio::process::Child,
    capture: bool,
    app: Arc<App>,
    token: Option<String>,
) {
    let (mut sender, mut receiver) = socket.split();
    if capture {
        let mut stdout = child.stdout.take().unwrap();
        let stream = async {
            let mut buf = vec![0; 8192];
            loop {
                let n = stdout.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                tokio::time::timeout(
                    Duration::from_secs(5),
                    sender.send(Message::Binary(buf[..n].to_vec().into())),
                )
                .await??;
            }
            anyhow::Ok(())
        };
        let closed = async {
            while let Some(message) = receiver.next().await {
                if matches!(message, Ok(Message::Close(_)) | Err(_)) {
                    break;
                }
            }
        };
        tokio::select! { _=stream=>{},_=closed=>{},_=app.session_revoked(token)=>{} }
    } else {
        let mut stdin = child.stdin.take().unwrap();
        let input = async {
            while let Some(message) = receiver.next().await {
                match message? {
                    Message::Binary(bytes) if bytes.len() % 4 == 0 => {
                        stdin.write_all(&bytes).await?
                    }
                    Message::Close(_) => break,
                    Message::Ping(_) | Message::Pong(_) => {}
                    _ => anyhow::bail!("Invalid PCM data"),
                }
            }
            anyhow::Ok(())
        };
        tokio::select! { _=input=>{},_=child.wait()=>{},_=app.session_revoked(token)=>{} }
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}
