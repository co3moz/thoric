use crate::{
    App, auth,
    error::{Error, Result},
    folder_archive::{self, Target},
    process,
};
use anyhow::{Context, bail};
use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{process::Stdio, sync::Arc, time::Duration};
use tokio::io::AsyncReadExt;

mod search;
pub use search::{SearchFiles, search, search_files};

pub const FILE_LIMIT: usize = 512 * 1024;
#[derive(Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    #[default]
    Utf8,
    Base64,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadFile {
    pub path: String,
    #[serde(default)]
    pub encoding: Encoding,
    #[serde(default)]
    pub offset: u64,
    #[serde(default = "default_limit")]
    pub limit: usize,
}
fn default_limit() -> usize {
    65536
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteFile {
    pub path: String,
    pub data: String,
    #[serde(default)]
    pub encoding: Encoding,
    #[serde(default)]
    pub replace: bool,
}
pub(crate) fn split_path(path: &str) -> anyhow::Result<(&str, &str)> {
    if !path.starts_with('/')
        || path.len() > 4096
        || path.contains(['\0', '\n', '\r', '\\'])
        || path[1..]
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
    {
        bail!("Use a canonical absolute file path without dot segments or symlinks");
    }
    let (directory, name) = path.rsplit_once('/').context("Invalid file path")?;
    Ok((if directory.is_empty() { "/" } else { directory }, name))
}
pub async fn read(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<ReadFile>,
) -> Result<Json<Value>> {
    split_path(&body.path)?;
    if body.limit == 0 || body.limit > FILE_LIMIT || body.offset > i64::MAX as u64 {
        return Err(anyhow::anyhow!(
            "Read limit must be 1 to 524288 bytes; offset must fit a signed 64-bit integer"
        )
        .into());
    }
    let _guard = app.share(&format!("container:{id}")).await?;
    let env = app.container(&id)?;
    let container = app.owned(&env).await?;
    if container["State"]["Running"] != true {
        return Err(anyhow::anyhow!("Start the container before reading files").into());
    }
    let bytes = app
        .docker(&[
            "exec",
            "--user",
            "1001:1001",
            container["Id"].as_str().context("Missing container ID")?,
            "python3",
            "-c",
            include_str!("../helpers/read_file.py"),
            &body.path,
            &body.offset.to_string(),
            &body.limit.to_string(),
        ])
        .await?;
    let mut result: Value = serde_json::from_slice(&bytes).map_err(|e| anyhow::anyhow!(e))?;
    let encoded = result["data_base64"]
        .as_str()
        .context("Missing file data")?;
    let (data, encoding) = match body.encoding {
        Encoding::Base64 => (encoded.to_string(), "base64"),
        Encoding::Utf8 => {
            let bytes = STANDARD.decode(encoded).map_err(|e| anyhow::anyhow!(e))?;
            (String::from_utf8(bytes).context("File slice is not valid UTF-8; use encoding=base64 for binary files or split characters")?, "utf8")
        }
    };
    result.as_object_mut().unwrap().remove("data_base64");
    result["data"] = json!(data);
    result["encoding"] = json!(encoding);
    result["path"] = json!(body.path);
    result["offset"] = json!(body.offset);
    Ok(Json(result))
}
pub async fn write(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<WriteFile>,
) -> Result<Json<Value>> {
    let (directory, name) = split_path(&body.path)?;
    let data = match body.encoding {
        Encoding::Utf8 => body.data.as_bytes().to_vec(),
        Encoding::Base64 => {
            if body.data.len() > FILE_LIMIT.div_ceil(3) * 4 {
                return Err(anyhow::anyhow!("File writes are limited to 512 KiB").into());
            }
            STANDARD
                .decode(&body.data)
                .map_err(|_| anyhow::anyhow!("data must be valid standard base64"))?
        }
    };
    if data.len() > FILE_LIMIT {
        return Err(anyhow::anyhow!("File writes are limited to 512 KiB").into());
    }
    let size = data.len();
    let _guard = app.share(&format!("container:{id}")).await?;
    let env = app.container(&id)?;
    let container = app.owned(&env).await?;
    if container["State"]["Running"] != true {
        return Err(anyhow::anyhow!("Start the container before writing files").into());
    }
    let args = [
        "exec",
        "--interactive",
        "--user",
        "1001:1001",
        container["Id"].as_str().context("Missing container ID")?,
        "python3",
        "-c",
        include_str!("../helpers/upload_file.py"),
        directory,
        name,
        &size.to_string(),
        if body.replace { "true" } else { "false" },
        "600",
    ]
    .map(str::to_owned);
    let output = process::capture_input(
        app.command(&args),
        Duration::from_secs(30),
        8192,
        process::Overflow::Tail,
        Some(data),
    )
    .await?;
    if !output.status.success() {
        return Err(Error::helper(&output));
    }
    Ok(Json(
        json!({"path":body.path,"bytes":size,"download_url":download_url(&id, &body.path)}),
    ))
}

pub(crate) const FILE_OPS: &str = include_str!("../helpers/file_ops.py");
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListFiles {
    /// Canonical absolute directory path.
    pub path: String,
    /// Maximum number of entries to return (1 to 2000).
    #[serde(default = "list_limit")]
    pub limit: usize,
}
fn list_limit() -> usize {
    500
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveFile {
    /// Canonical absolute path of the file or directory to move.
    pub source: String,
    /// Canonical absolute destination path on the same volume.
    pub destination: String,
    /// Replace an existing destination.
    #[serde(default)]
    pub replace: bool,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteFile {
    /// Canonical absolute path of the file, symlink or directory to delete.
    pub path: String,
    /// Delete a directory together with its contents.
    #[serde(default)]
    pub recursive: bool,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MakeDirectory {
    /// Canonical absolute path of the new folder, whose parent must exist.
    pub path: String,
}
impl ListFiles {
    pub fn validate(&self) -> anyhow::Result<()> {
        crate::apps::directory(&self.path)?;
        if self.limit == 0 || self.limit > 2000 {
            bail!("limit must be 1 to 2000");
        }
        Ok(())
    }
}
impl MoveFile {
    pub fn validate(&self) -> anyhow::Result<()> {
        split_path(&self.source)?;
        split_path(&self.destination)?;
        Ok(())
    }
}
impl DeleteFile {
    /// `mounts` are all the container mounts, its settings' and its compose's.
    pub fn validate(&self, mounts: &[crate::model::Mount]) -> anyhow::Result<()> {
        split_path(&self.path)?;
        if self.path[1..].split('/').count() < 2
            || self.path == "/home/desktop"
            || mounts.iter().any(|mount| mount.target == self.path)
        {
            bail!(
                "Refusing to delete a top-level directory, the desktop home or a mounted volume's root"
            );
        }
        Ok(())
    }
}
async fn operate(app: &App, id: &str, arguments: &[&str]) -> Result<Value> {
    let _guard = app.share(&format!("container:{id}")).await?;
    let env = app.container(id)?;
    let container = app.owned(&env).await?;
    if container["State"]["Running"] != true {
        return Err(anyhow::anyhow!("Start the container before using its files").into());
    }
    file_ops(
        app,
        container["Id"].as_str().context("Missing container ID")?,
        arguments,
    )
    .await
}
/// Run `file_ops.py` in the running container `cid`, whose lock the caller holds.
async fn file_ops(app: &App, cid: &str, arguments: &[&str]) -> Result<Value> {
    let mut args = vec![
        "exec",
        "--user",
        "1001:1001",
        cid,
        "python3",
        "-c",
        FILE_OPS,
    ];
    args.extend(arguments);
    let args = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let output = process::capture(
        app.command(&args),
        Duration::from_secs(120),
        1_048_576,
        process::Overflow::Reject,
    )
    .await?;
    if !output.status.success() {
        return Err(Error::helper(&output));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| anyhow::anyhow!(e).into())
}
pub async fn list_files(app: &App, id: &str, body: &ListFiles) -> Result<Value> {
    body.validate()?;
    operate(app, id, &["list", &body.path, &body.limit.to_string()]).await
}
pub async fn move_file(app: &App, id: &str, body: &MoveFile) -> Result<Value> {
    body.validate()?;
    operate(
        app,
        id,
        &[
            "move",
            &body.source,
            &body.destination,
            if body.replace { "true" } else { "false" },
        ],
    )
    .await
}
pub async fn delete_file(app: &App, id: &str, body: &DeleteFile) -> Result<Value> {
    let env = app.container(id)?;
    body.validate(&env.mounts())?;
    operate(
        app,
        id,
        &[
            "delete",
            &body.path,
            if body.recursive { "true" } else { "false" },
        ],
    )
    .await
}
pub async fn list(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<ListFiles>,
) -> Result<Json<Value>> {
    Ok(Json(list_files(&app, &id, &body).await?))
}
pub async fn move_(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<MoveFile>,
) -> Result<Json<Value>> {
    Ok(Json(move_file(&app, &id, &body).await?))
}
/// Create one folder as the desktop user, such as a new upload destination.
pub async fn make_directory(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<MakeDirectory>,
) -> Result<Json<Value>> {
    split_path(&body.path)?;
    Ok(Json(operate(&app, &id, &["mkdir", &body.path]).await?))
}
pub async fn delete(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<DeleteFile>,
) -> Result<Json<Value>> {
    Ok(Json(delete_file(&app, &id, &body).await?))
}

const DOWNLOAD: &str = include_str!("../helpers/download_file.py");
/// Root-relative dashboard URL that streams a desktop file, for handing a file to the user.
/// It is served from the same address as the dashboard and the MCP endpoint, so it is valid
/// wherever the manager is reachable, and it needs a dashboard sign-in in the browser.
pub fn download_url(id: &str, path: &str) -> String {
    format!(
        "/api/containers/{id}/files/download?path={}",
        encode_path(path)
    )
}
/// The `file://` URI an MCP result names a desktop file by, which MCP allows for resources that
/// behave like a filesystem without being on the client's own.
pub fn file_uri(path: &str) -> String {
    format!("file://{}", encode_path(path))
}
fn encode_path(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}
/// The most bytes `download_file` returns. They travel once, as base64 in a content block of
/// their own, and the whole MCP reply must stay within the 8 MiB `/mcp` sends.
pub const DOWNLOAD_LIMIT: usize = 2 * 1024 * 1024;
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Download {
    /// Canonical absolute path of a regular file.
    pub path: String,
    /// Maximum bytes to return (1 to 2097152, default 1048576); a larger file is refused.
    #[serde(default = "download_limit")]
    pub max_bytes: usize,
}
fn download_limit() -> usize {
    1024 * 1024
}
#[derive(Deserialize, JsonSchema)]
pub struct DownloadQuery {
    pub path: String,
}
pub fn mime_type(name: &str) -> &'static str {
    let extension = name
        .rsplit('.')
        .next()
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "txt" | "log" | "md" | "cfg" | "ini" | "csv" => "text/plain; charset=utf-8",
        "json" => "application/json",
        "html" | "htm" => "text/html; charset=utf-8",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "wav" => "audio/wav",
        "avi" => "video/x-msvideo",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
}
pub(crate) fn percent_encode(name: &str) -> String {
    name.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
async fn running_container(app: &App, id: &str) -> Result<Value> {
    let env = app.container(id)?;
    let container = app.owned(&env).await?;
    if container["State"]["Running"] != true {
        return Err(Error(
            StatusCode::CONFLICT,
            "Start the container before using its files".into(),
        ));
    }
    Ok(container)
}
/// A whole file for `download_file`: what it is, and its bytes, which the MCP result carries
/// once in a content block of their own.
pub async fn download_file(app: &App, id: &str, body: &Download) -> Result<(Value, Vec<u8>)> {
    let (_, name) = split_path(&body.path)?;
    if body.max_bytes == 0 || body.max_bytes > DOWNLOAD_LIMIT {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            format!("max_bytes must be 1 to {DOWNLOAD_LIMIT}"),
        ));
    }
    let _guard = app.share(&format!("container:{id}")).await?;
    let container = running_container(app, id).await?;
    let args = [
        "exec",
        "--user",
        "1001:1001",
        container["Id"].as_str().context("Missing container ID")?,
        "python3",
        "-c",
        DOWNLOAD,
        &body.path,
        &body.max_bytes.to_string(),
    ]
    .map(str::to_owned);
    let output = process::capture(
        app.command(&args),
        Duration::from_secs(120),
        body.max_bytes + 1,
        process::Overflow::Reject,
    )
    .await?;
    if !output.status.success() {
        return Err(Error::helper(&output));
    }
    let summary = json!({
        "path": body.path,
        "name": name,
        "bytes": output.stdout.len(),
        "mime_type": mime_type(name),
        "download_url": download_url(id, &body.path),
    });
    Ok((summary, output.stdout))
}
/// Stream a file or a folder from the desktop as the desktop user: a regular file as it is, with
/// its size known up front and limited by UPLOAD_MAX_MB, a folder as a `.tar.gz`.
pub async fn download(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Query(query): Query<DownloadQuery>,
    token: auth::Session,
) -> Result<Response> {
    let (_, name) = split_path(&query.path)?;
    let _guard = app.share(&format!("container:{id}")).await?;
    let container = running_container(&app, &id).await?;
    let cid = container["Id"].as_str().context("Missing container ID")?;
    let entry = file_ops(&app, cid, &["stat", &query.path]).await?;
    let size = match folder_archive::target(&entry)? {
        Target::File(size) => size,
        Target::Folder => {
            let mut args = ["exec", "--interactive", "--user", "1001:1001", cid]
                .map(str::to_owned)
                .to_vec();
            args.extend(folder_archive::command(&query.path));
            let source = folder_archive::Source {
                args,
                helper: None,
                lock: None,
                label: format!("{} in {}", query.path, app.container(&id)?.spec.name),
                path: query.path.clone(),
            };
            return folder_archive::respond(
                app.clone(),
                format!("container:{id}"),
                name,
                source,
                token,
            );
        }
    };
    if size > app.config.upload_limit {
        return Err(Error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "File exceeds the configured transfer limit (UPLOAD_MAX_MB)".into(),
        ));
    }
    let args = [
        "exec",
        "--user",
        "1001:1001",
        cid,
        "python3",
        "-c",
        DOWNLOAD,
        &query.path,
        &app.config.upload_limit.to_string(),
    ]
    .map(str::to_owned);
    let mut child = app
        .command(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(anyhow::Error::from)?;
    let stdout = child.stdout.take().unwrap();
    let stream = futures_util::stream::unfold((child, stdout), |(child, mut stdout)| async move {
        let mut buffer = vec![0; 65536];
        match stdout.read(&mut buffer).await {
            Ok(0) | Err(_) => None,
            Ok(n) => {
                buffer.truncate(n);
                Some((
                    Ok::<_, std::io::Error>(axum::body::Bytes::from(buffer)),
                    (child, stdout),
                ))
            }
        }
    });
    Ok((
        [
            (header::CONTENT_TYPE, mime_type(name).to_string()),
            (header::CONTENT_LENGTH, size.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename*=UTF-8''{}", percent_encode(name)),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}
#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;
