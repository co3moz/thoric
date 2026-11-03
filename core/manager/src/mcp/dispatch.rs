//! The calls of the administrator's channel at `/mcp`, which MCP endpoints make too for the tools
//! they bind to a container: each tool runs the HTTP handler or the function behind its route,
//! and that answer becomes the call's result.
use super::ThoricMcp;
use crate::{
    api, apps, audio, capture, changes, clipboard, compose, desktop_input, desktop_mcp, duplicate,
    error, exec, files, gamepad, human, image_removal, jobs, layers, ocr, recordings, terminal,
    transfers, uploads, usage, volumes, waits, windows,
};
use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::StatusCode,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use rmcp::model::{CallToolResult, ContentBlock, ResourceContents};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

const FILE_LIMIT: usize = 512 * 1024;

fn parse<T: DeserializeOwned>(value: Value) -> error::Result<T> {
    serde_json::from_value(value).map_err(|e| {
        error::Error(
            StatusCode::BAD_REQUEST,
            format!("Invalid tool arguments: {e}"),
        )
    })
}
fn string(args: &Value, key: &str) -> error::Result<String> {
    args[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| error::Error(StatusCode::BAD_REQUEST, format!("{key} must be a string")))
}
/// A screenshot with its size and its place on the desktop, which the agent needs as text too to
/// map a point of a scaled or cropped image back to desktop_input's coordinates.
pub(super) fn image(shot: capture::Screenshot, input: Option<Value>) -> CallToolResult {
    let mut summary = shot.summary();
    if let Some(input) = input {
        summary["input"] = input;
    }
    with_data(
        ContentBlock::image(STANDARD.encode(&shot.jpeg), "image/jpeg"),
        summary,
    )
}
fn sound(mut value: Value) -> CallToolResult {
    let wav = value
        .as_object_mut()
        .and_then(|object| object.remove("wav_base64"))
        .and_then(|data| data.as_str().map(str::to_owned))
        .unwrap_or_default();
    with_data(ContentBlock::audio(wav, "audio/wav"), value)
}
fn structured(value: Value) -> CallToolResult {
    // Keep an object result for clients using the 2025 protocol versions.
    CallToolResult::structured(if value.is_object() {
        value
    } else {
        json!({"items":value})
    })
}
/// A result whose data travels once, in a content block of its own, before the JSON that
/// describes it as text and as structuredContent. Data inside the JSON would travel twice, since
/// the text block repeats structuredContent for clients that read only text.
fn with_data(data: ContentBlock, summary: Value) -> CallToolResult {
    let mut result = CallToolResult::structured(summary);
    result.content.insert(0, data);
    result
}
/// The pictures a model can look at, which a downloaded file is returned as an image block for.
const VIEWABLE: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];
/// A file `download_file` read: a picture as an image block, anything else as an embedded
/// resource whose blob is the file.
pub(super) fn file(summary: Value, bytes: &[u8]) -> CallToolResult {
    let data = STANDARD.encode(bytes);
    let mime_type = summary["mime_type"]
        .as_str()
        .unwrap_or("application/octet-stream")
        .to_owned();
    let block = if VIEWABLE.contains(&mime_type.as_str()) {
        ContentBlock::image(data, mime_type)
    } else {
        let uri = files::file_uri(summary["path"].as_str().unwrap_or_default());
        ContentBlock::resource(ResourceContents::blob(data, uri).with_mime_type(mime_type))
    };
    with_data(block, summary)
}

impl ThoricMcp {
    /// [`Self::dispatch`] for a call whose MCP request is known, which `request_human` waits on
    /// and which tells the terminal tools the endpoint that calls.
    pub(crate) async fn dispatch_with(
        &self,
        name: &str,
        mut args: Value,
        caller: human::Caller,
    ) -> error::Result<CallToolResult> {
        if name == "get_job" {
            // A wait for the job reports progress and ends when the client gives the call up.
            let (id, read) = jobs::read::mcp_arguments(args)?;
            return Ok(structured(
                Box::pin(jobs::read::answer(&self.app, &id, &read, Some(&caller))).await?,
            ));
        }
        if name != "request_human" && !terminal::TOOLS.contains(&name) {
            return self.dispatch(name, args).await;
        }
        let id = string(&args, "container_id")?;
        if let Some(object) = args.as_object_mut() {
            object.remove("container_id");
        }
        if name == "request_human" {
            return Box::pin(human::request(&self.app, &id, args, caller)).await;
        }
        self.dispatch_terminal(name, &id, args, caller.endpoint())
            .await
            .map(structured)
    }
    /// The terminal tools, which an MCP endpoint calls for the terminals it opened alone.
    async fn dispatch_terminal(
        &self,
        name: &str,
        id: &str,
        mut args: Value,
        endpoint: Option<&str>,
    ) -> error::Result<Value> {
        let terminal_id = match name {
            "open_terminal" => {
                return terminal::open_terminal(&self.app, id, &parse(args)?, endpoint).await;
            }
            "list_terminals" => return terminal::list_terminals(&self.app, id, endpoint).await,
            _ => string(&args, "terminal_id")?,
        };
        args.as_object_mut().unwrap().remove("terminal_id");
        match name {
            "terminal_input" => {
                terminal::terminal_input(&self.app, id, &terminal_id, &parse(args)?, endpoint).await
            }
            "terminal_output" => {
                terminal::terminal_output(&self.app, id, &terminal_id, &parse(args)?, endpoint)
                    .await
            }
            "close_terminal" => {
                terminal::close_terminal(&self.app, id, &terminal_id, endpoint).await
            }
            _ => unreachable!("tool catalog and dispatch must agree"),
        }
    }
    pub(crate) async fn dispatch(
        &self,
        name: &str,
        mut args: Value,
    ) -> error::Result<CallToolResult> {
        let state = State(self.app.clone());
        let id = if args.get("container_id").is_some() {
            Some(string(&args, "container_id")?)
        } else {
            None
        };
        if let Some(object) = args.as_object_mut() {
            object.remove("container_id");
        }
        let value = match name {
            "get_settings" => api::settings(state).await.0,
            "get_system" => api::system(state).await.0,
            "pull_image" => {
                api::pull_catalog(state, Path(string(&args, "image_id")?))
                    .await?
                    .0
            }
            "build_catalog_image" => {
                api::build_catalog(state, Path(string(&args, "image_id")?))
                    .await?
                    .0
            }
            "list_containers" => api::containers(state).await?.0,
            "get_container" => api::container(state, Path(id.unwrap())).await?.0,
            "create_container" => api::create(state, Json(parse(args)?)).await?.0,
            "container_action" => {
                api::action(state, Path(id.unwrap()), Json(parse(args)?))
                    .await?
                    .0
            }
            "set_internet" => {
                api::internet(state, Path(id.unwrap()), Json(parse(args)?))
                    .await?
                    .0
            }
            "update_container" => {
                api::update_settings(state, Path(id.unwrap()), Json(args))
                    .await?
                    .0
            }
            "retry_container" => api::retry(state, Path(id.unwrap())).await?.0,
            "delete_container" => {
                api::remove(state, Path(id.unwrap()), Query(parse(args)?))
                    .await?
                    .0
            }
            "container_logs" => api::logs(state, Path(id.unwrap())).await?.0,
            "container_diagnostics" => api::diagnostics(state, Path(id.unwrap())).await?.0,
            "exec_container" => {
                exec::execute(state, Path(id.unwrap()), Json(parse(args)?))
                    .await?
                    .0
            }
            "reset_container" => api::reset(state, Path(id.unwrap())).await?.0,
            "list_images" => api::images(state).await?.0,
            "get_image_layers" => {
                layers::layers(state, Path(string(&args, "image_id")?))
                    .await?
                    .0
            }
            "import_image" => api::register_image(state, Json(parse(args)?)).await?.0,
            "build_image" => api::build(state, Json(parse(args)?)).await?.0,
            "get_image_compose" => {
                json!(
                    compose::get(state, Path(string(&args, "image_id")?))
                        .await?
                        .0
                )
            }
            "set_image_compose" => {
                let image = Path(string(&args, "image_id")?);
                match args.get("compose") {
                    None | Some(Value::Null) => compose::remove(state, image).await?.0,
                    Some(compose) => {
                        compose::set(state, image, Json(parse(compose.clone())?))
                            .await?
                            .0
                    }
                }
            }
            "snapshot" => {
                api::snapshot(state, Path(id.unwrap()), Json(parse(args)?))
                    .await?
                    .0
            }
            "duplicate_container" => {
                duplicate::duplicate(state, Path(id.unwrap()), Json(parse(args)?))
                    .await?
                    .0
            }
            "delete_image" => {
                Box::pin(image_removal::remove(
                    state,
                    Path(string(&args, "image_id")?),
                ))
                .await?
                .0
            }
            "list_volumes" => api::volumes(state).await?.0,
            "create_volume" => api::create_volume(state, Json(parse(args)?)).await?.0,
            "copy_volume" => {
                let source = Path(string(&args, "volume")?);
                args.as_object_mut().unwrap().remove("volume");
                Box::pin(volumes::copy(state, source, Json(parse(args)?)))
                    .await?
                    .0
            }
            "delete_volume" => {
                Box::pin(volumes::remove(state, Path(string(&args, "name")?)))
                    .await?
                    .0
            }
            "get_usage" => usage::summary(&self.app, id.as_deref(), &parse(args)?)?,
            "get_changes" => {
                Box::pin(changes::changes(&self.app, &id.unwrap(), &parse(args)?)).await?
            }
            "list_jobs" => api::jobs(state).await?.0,
            "cancel_job" => {
                jobs::cancel::cancel(state, Path(string(&args, "job_id")?))
                    .await?
                    .0
            }
            "get_job" => {
                let (id, read) = jobs::read::mcp_arguments(args)?;
                jobs::read::answer(&self.app, &id, &read, None).await?
            }
            "create_transfer" => {
                // The link binds its container like every other argument.
                if let Some(id) = id {
                    args["container_id"] = json!(id);
                }
                transfers::create(&self.app, parse(args)?)?
            }
            // Without its MCP request the call reports no progress and outlives its client.
            "request_human" => {
                let caller = human::Caller::default();
                return Box::pin(human::request(&self.app, &id.unwrap(), args, caller)).await;
            }
            "list_mcp_servers"
            | "call_mcp_server_tool"
            | "register_mcp_server"
            | "remove_mcp_server"
            | "list_mcp_endpoints"
            | "create_mcp_endpoint"
            | "update_mcp_endpoint"
            | "delete_mcp_endpoint"
            | "create_mcp_endpoint_token" => {
                return Box::pin(self.dispatch_endpoints(name, id, args)).await;
            }
            _ => return Box::pin(self.dispatch_desktop(name, id, args)).await,
        };
        Ok(structured(value))
    }
    /// Desktop MCP servers and the MCP endpoints that publish them.
    async fn dispatch_endpoints(
        &self,
        name: &str,
        id: Option<String>,
        mut args: Value,
    ) -> error::Result<CallToolResult> {
        let state = State(self.app.clone());
        let value = match name {
            "list_mcp_servers" => match id {
                Some(id) => desktop_mcp::servers_of(&self.app, &id, true).await?,
                None => desktop_mcp::all_servers(state).await?.0,
            },
            // The server's result is passed on as it is, as an endpoint passes it on.
            "call_mcp_server_tool" => {
                let arguments = match args.get_mut("arguments").map(Value::take) {
                    None | Some(Value::Null) => Default::default(),
                    Some(Value::Object(arguments)) => arguments,
                    Some(_) => {
                        return Err(error::Error(
                            StatusCode::BAD_REQUEST,
                            "arguments must be an object".into(),
                        ));
                    }
                };
                return Box::pin(desktop_mcp::call_server_tool(
                    &self.app,
                    &id.unwrap(),
                    &string(&args, "server")?,
                    &string(&args, "tool")?,
                    arguments,
                ))
                .await;
            }
            "register_mcp_server" => {
                desktop_mcp::register_server(state, Path(id.unwrap()), Json(parse(args)?))
                    .await?
                    .0
            }
            "remove_mcp_server" => {
                desktop_mcp::unregister_server(state, Path((id.unwrap(), string(&args, "name")?)))
                    .await?
                    .0
            }
            "list_mcp_endpoints" => desktop_mcp::endpoints(state).await?.0,
            "create_mcp_endpoint" => {
                desktop_mcp::create_endpoint(state, Json(parse(args)?))
                    .await?
                    .0
            }
            "update_mcp_endpoint" => {
                let endpoint = string(&args, "name")?;
                args.as_object_mut().unwrap().remove("name");
                desktop_mcp::update_endpoint(state, Path(endpoint), Json(parse(args)?))
                    .await?
                    .0
            }
            "delete_mcp_endpoint" => {
                desktop_mcp::delete_endpoint(state, Path(string(&args, "name")?))
                    .await?
                    .0
            }
            "create_mcp_endpoint_token" => {
                desktop_mcp::token(state, Path(string(&args, "name")?))
                    .await?
                    .0
            }
            _ => unreachable!("tool catalog and dispatch must agree"),
        };
        Ok(structured(value))
    }
    /// Tools that work inside a running desktop. They are matched apart from the others because
    /// a debug build gives every arm of one `match` its own room in a single stack frame, which
    /// together outgrew the 2 MiB of a test thread.
    async fn dispatch_desktop(
        &self,
        name: &str,
        id: Option<String>,
        mut args: Value,
    ) -> error::Result<CallToolResult> {
        let state = State(self.app.clone());
        let value = match name {
            "read_file" => {
                files::read(state, Path(id.unwrap()), Json(parse(args)?))
                    .await?
                    .0
            }
            "write_file" => {
                files::write(state, Path(id.unwrap()), Json(parse(args)?))
                    .await?
                    .0
            }
            "list_files" => files::list_files(&self.app, &id.unwrap(), &parse(args)?).await?,
            "download_file" => {
                let (summary, bytes) =
                    files::download_file(&self.app, &id.unwrap(), &parse(args)?).await?;
                return Ok(file(summary, &bytes));
            }
            "capture_audio" => {
                let value = audio::capture_audio(&self.app, &id.unwrap(), &parse(args)?).await?;
                return Ok(sound(value));
            }
            "inject_audio" => audio::inject_audio(&self.app, &id.unwrap(), &parse(args)?).await?,
            "get_clipboard" => {
                Box::pin(clipboard::get_clipboard(
                    &self.app,
                    &id.unwrap(),
                    &parse(args)?,
                ))
                .await?
            }
            "set_clipboard" => {
                Box::pin(clipboard::set_clipboard(
                    &self.app,
                    &id.unwrap(),
                    &parse(args)?,
                ))
                .await?
            }
            "inspect_ui" => windows::inspect_ui(&self.app, &id.unwrap(), &parse(args)?).await?,
            "gamepad_input" => {
                gamepad::gamepad_input(&self.app, &id.unwrap(), &parse(args)?).await?
            }
            "move_file" => files::move_file(&self.app, &id.unwrap(), &parse(args)?).await?,
            "delete_file" => files::delete_file(&self.app, &id.unwrap(), &parse(args)?).await?,
            "desktop_input" => {
                desktop_input::input(state, Path(id.unwrap()), Json(parse(args)?))
                    .await?
                    .0
            }
            "screenshot" => {
                let options: capture::Capture = parse(args)?;
                let shot = capture::capture(&self.app, &id.unwrap(), &options).await?;
                return Ok(image(shot, None));
            }
            "save_screenshot" => {
                capture::save_screenshot(&self.app, &id.unwrap(), &parse(args)?).await?
            }
            "act_and_observe" => {
                let body: capture::Observe = parse(args)?;
                let (input, shot) =
                    capture::act_and_observe(&self.app, &id.unwrap(), &body).await?;
                return Ok(image(shot, Some(input)));
            }
            "find_text" => Box::pin(ocr::find_text(&self.app, &id.unwrap(), &parse(args)?)).await?,
            "launch_application" => {
                apps::launch_application(&self.app, &id.unwrap(), &parse(args)?).await?
            }
            "list_applications" => apps::list_applications(&self.app, &id.unwrap()).await?,
            "get_application" => {
                apps::get_application(&self.app, &id.unwrap(), &string(&args, "application_id")?)
                    .await?
            }
            "application_logs" => {
                let application = string(&args, "application_id")?;
                args.as_object_mut().unwrap().remove("application_id");
                let options: apps::Logs = parse(args)?;
                apps::application_logs(&self.app, &id.unwrap(), &application, options.tail_bytes)
                    .await?
            }
            "stop_application" => {
                let application = string(&args, "application_id")?;
                args.as_object_mut().unwrap().remove("application_id");
                let options: apps::Stop = parse(args)?;
                apps::stop_application(&self.app, &id.unwrap(), &application, options.grace_seconds)
                    .await?
            }
            "wait_for_condition" => {
                Box::pin(waits::wait_for_condition(
                    &self.app,
                    &id.unwrap(),
                    &parse(args)?,
                ))
                .await?
            }
            "application_diagnostics" => {
                apps::application_diagnostics(
                    &self.app,
                    &id.unwrap(),
                    &string(&args, "application_id")?,
                )
                .await?
            }
            "sample_metrics" => {
                let application = string(&args, "application_id")?;
                args.as_object_mut().unwrap().remove("application_id");
                let options: apps::Metrics = parse(args)?;
                apps::sample_metrics(&self.app, &id.unwrap(), &application, options.duration_ms)
                    .await?
            }
            "start_recording" => {
                recordings::start_recording(&self.app, &id.unwrap(), &parse(args)?).await?
            }
            "stop_recording" => {
                recordings::stop_recording(&self.app, &id.unwrap(), &string(&args, "recording_id")?)
                    .await?
            }
            "list_recordings" => recordings::list_recordings(&self.app, &id.unwrap()).await?,
            "export_artifacts" => {
                recordings::export_artifacts(&self.app, &id.unwrap(), &parse(args)?).await?
            }
            "list_windows" => windows::list_windows(&self.app, &id.unwrap()).await?,
            "focus_window" => {
                windows::focus_window(&self.app, &id.unwrap(), &string(&args, "window_id")?).await?
            }
            "set_window_geometry" => {
                let window = string(&args, "window_id")?;
                args.as_object_mut().unwrap().remove("window_id");
                let geometry: windows::Geometry = parse(args)?;
                windows::set_window_geometry(&self.app, &id.unwrap(), &window, &geometry).await?
            }
            "upload_file" => {
                let encoded = string(&args, "data_base64")?;
                if encoded.len() > FILE_LIMIT.div_ceil(3) * 4 {
                    return Err(
                        anyhow::anyhow!("MCP uploads are limited to 512 KiB per file").into(),
                    );
                }
                let data = STANDARD
                    .decode(encoded)
                    .map_err(|_| anyhow::anyhow!("data_base64 must be valid standard base64"))?;
                if data.len() > FILE_LIMIT {
                    return Err(
                        anyhow::anyhow!("MCP uploads are limited to 512 KiB per file").into(),
                    );
                }
                let mut request = args;
                request.as_object_mut().unwrap().remove("data_base64");
                request["size"] = json!(data.len());
                uploads::save_file(
                    &self.app,
                    id.unwrap(),
                    parse(request)?,
                    Body::from(data),
                    std::future::pending(),
                )
                .await?
                .0
            }
            _ => unreachable!("tool catalog and dispatch must agree"),
        };
        Ok(structured(value))
    }
}
