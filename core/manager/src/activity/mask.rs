//! What the activity log keeps of a call's arguments.
//!
//! Every Thoric tool has a policy: the argument keys whose values are kept, and how. Any other
//! argument, also one a later release adds before its policy says otherwise, is kept as its type
//! and size only, `{"$masked": "string", "length": 12}`. That is how file contents, compose
//! values, base64 payloads, headers, environment values and text typed into a desktop or a
//! terminal are kept: a typed password or a token is never written down. A desktop server's tool
//! is opaque to the manager, so its arguments keep their keys and sizes whatever it is called.
//!
//! A kept text is cut at [`TEXT`] characters and a script at [`SCRIPT`], as
//! `{"$truncated": "the start", "length": 9000}`: what `exec_container` runs is the first thing
//! anyone looks for. An error text is stripped of every masked value before it is kept, since a
//! refusal may quote the argument it refused.
use super::Source;
use serde_json::{Map, Value, json};

/// The characters of a kept text.
pub(super) const TEXT: usize = 256;
/// The characters of a kept script: what `exec_container` or `build_image` runs.
pub(super) const SCRIPT: usize = 4096;
/// The bytes of JSON kept of a list or an object.
const WHOLE: usize = 2048;
/// The arguments kept of one call; a desktop server's tool can take any number.
const KEYS: usize = 64;
/// The characters of a kept key name, and of a key of a desktop shortcut.
const KEY: usize = 64;
/// The shortest masked value an error text is stripped of; shorter ones are common words.
const SECRET: usize = 4;
/// The characters of the summary line.
const SUMMARY: usize = 160;

/// How a kept argument is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Keep {
    /// The value, a text cut at [`TEXT`] characters.
    Text,
    /// A URL without its query or fragment, which may carry a token, cut at [`TEXT`] characters.
    Url,
    /// A script, cut at [`SCRIPT`] characters.
    Script,
    /// A list or an object as it is, up to [`WHOLE`] bytes of JSON.
    Whole,
    /// An object's keys, each value as its type and size: environment variables, headers and
    /// compose values.
    Keys,
    /// Desktop input actions: what is clicked, scrolled and pressed, with typed text and keys of a
    /// single character as their size.
    Actions,
}
use Keep::*;

/// An argument a policy keeps, and whether the summary line leads with it.
pub(super) type Rule = (&'static str, Keep, bool);

/// The arguments every Thoric tool keeps: the container, by ID on `/mcp` and by its alias on
/// an endpoint of several.
const COMMON: &[Rule] = &[
    ("container_id", Text, false),
    ("container", Text, false),
    ("instance", Text, false),
];

/// What a transfer link's operation keeps: the paths, names and volumes it names.
const TRANSFER: &[Rule] = &[
    ("path", Text, true),
    ("destination", Text, true),
    ("name", Text, true),
    ("volume", Text, true),
    ("image_id", Text, true),
    ("replace", Text, false),
    ("description", Text, false),
];

/// The policy of a Thoric tool, `None` for a name `/mcp` does not offer.
pub(super) fn policy(tool: &str) -> Option<&'static [Rule]> {
    Some(match tool {
        "get_settings"
        | "get_system"
        | "list_containers"
        | "list_images"
        | "list_volumes"
        | "list_jobs"
        | "list_mcp_endpoints"
        | "list_mcp_servers"
        | "get_container"
        | "container_logs"
        | "container_diagnostics"
        | "retry_container"
        | "reset_container"
        | "list_applications"
        | "list_windows"
        | "list_recordings"
        | "list_terminals" => &[],
        "pull_image"
        | "build_catalog_image"
        | "get_image_layers"
        | "get_image_compose"
        | "delete_image" => &[("image_id", Text, true)],
        "create_container" => &[
            ("name", Text, true),
            ("image", Text, true),
            ("resolution", Text, false),
            ("gpu", Text, false),
            ("internet", Text, false),
            ("audio", Text, false),
            ("microphone", Text, false),
            ("sudo", Text, false),
            ("gamepad", Text, false),
            ("vpn", Text, false),
            ("debugger", Text, false),
            ("realtime", Text, false),
            ("profiler", Text, false),
            ("kvm", Text, false),
            ("proxy", Whole, false),
            ("hostname", Text, false),
            ("mac_address", Text, false),
            ("machine_id", Text, false),
            ("ports", Whole, false),
            ("groups", Whole, false),
            ("aliases", Whole, false),
            ("mcp_endpoint", Text, false),
            ("cpus", Text, false),
            ("memory_mb", Text, false),
            ("shm_mb", Text, false),
            ("tmp_mb", Text, false),
            ("mounts", Whole, false),
            ("lifecycle", Whole, false),
            ("scaling", Whole, false),
            // Secret compose values are among these.
            ("parameters", Keys, false),
        ],
        "container_action" => &[("action", Text, true)],
        "set_internet" => &[("enabled", Text, true)],
        "request_human" => &[
            ("message", Text, true),
            ("request_id", Text, true),
            ("wait_seconds", Text, false),
        ],
        "update_container" => &[
            ("name", Text, true),
            ("dry_run", Text, false),
            ("resolution", Text, false),
            ("gpu", Text, false),
            ("internet", Text, false),
            ("audio", Text, false),
            ("microphone", Text, false),
            ("sudo", Text, false),
            ("gamepad", Text, false),
            ("vpn", Text, false),
            ("debugger", Text, false),
            ("realtime", Text, false),
            ("profiler", Text, false),
            ("kvm", Text, false),
            ("proxy", Whole, false),
            ("ports", Whole, false),
            ("groups", Whole, false),
            ("aliases", Whole, false),
            ("hostname", Text, false),
            ("mac_address", Text, false),
            ("machine_id", Text, false),
            ("cpus", Text, false),
            ("memory_mb", Text, false),
            ("shm_mb", Text, false),
            ("tmp_mb", Text, false),
            ("mounts", Whole, false),
            ("lifecycle", Whole, false),
        ],
        "delete_container" => &[("delete_volumes", Text, false)],
        "exec_container" => &[
            ("script", Script, true),
            ("root", Text, true),
            ("timeout_seconds", Text, false),
        ],
        "import_image" => &[("reference", Text, true), ("name", Text, true)],
        // The compose and the icon stay out: the one may hold parameters' defaults, the other
        // is base64.
        "build_image" => &[
            ("name", Text, true),
            ("base_image", Text, true),
            ("description", Text, false),
            ("script", Script, false),
            ("internet", Text, false),
            ("inherit_compose", Text, false),
            ("template_id", Text, false),
            ("template_ids", Whole, false),
        ],
        "set_image_compose" => &[("image_id", Text, true)],
        "snapshot" => &[("name", Text, true), ("description", Text, false)],
        "list_instances" => &[("set", Text, true)],
        "create_instance" => &[
            ("set", Text, true),
            ("volumes", Whole, false),
            ("note", Text, false),
        ],
        "delete_instance" => &[("set", Text, true), ("instance", Text, true)],
        "duplicate_container" => &[
            ("name", Text, true),
            ("start", Text, false),
            ("mcp_endpoint", Text, false),
        ],
        "create_volume" | "delete_volume" => &[("name", Text, true)],
        "copy_volume" => &[("volume", Text, true), ("name", Text, true)],
        "get_usage" => &[("seconds", Text, false)],
        "cancel_job" => &[("job_id", Text, true)],
        "get_job" => &[
            ("job_id", Text, true),
            ("wait_seconds", Text, false),
            ("log_tail_bytes", Text, false),
            ("log_offset", Text, false),
        ],
        "create_transfer" => &[
            ("operation", Text, true),
            ("path", Text, true),
            ("destination", Text, true),
            ("name", Text, true),
            ("volume", Text, true),
            ("image_id", Text, true),
            ("replace", Text, false),
            ("description", Text, false),
        ],
        // Arguments, environment values and headers may carry a server's own credentials.
        "register_mcp_server" => &[
            ("name", Text, true),
            ("command", Text, true),
            ("url", Url, true),
            ("description", Text, false),
            ("cwd", Text, false),
            ("env", Keys, false),
            ("headers", Keys, false),
        ],
        // A desktop server's tool is opaque, so its arguments keep their names and sizes as they
        // do when an endpoint calls it.
        "call_mcp_server_tool" => &[
            ("server", Text, true),
            ("tool", Text, true),
            ("arguments", Keys, false),
        ],
        "remove_mcp_server" | "delete_mcp_endpoint" | "create_mcp_endpoint_token" => {
            &[("name", Text, true)]
        }
        "create_mcp_endpoint" | "update_mcp_endpoint" => {
            &[("name", Text, true), ("grants", Whole, false)]
        }
        "upload_file" => &[
            ("destination", Text, true),
            ("name", Text, true),
            ("replace", Text, false),
        ],
        "read_file" => &[
            ("path", Text, true),
            ("encoding", Text, false),
            ("offset", Text, false),
            ("limit", Text, false),
        ],
        "write_file" => &[
            ("path", Text, true),
            ("encoding", Text, false),
            ("replace", Text, false),
        ],
        "screenshot" => &[
            ("max_width", Text, false),
            ("region", Text, false),
            ("window_id", Text, false),
        ],
        "save_screenshot" => &[
            ("path", Text, true),
            ("replace", Text, false),
            ("capture", Whole, false),
        ],
        "desktop_input" => &[("actions", Actions, true)],
        "act_and_observe" => &[
            ("actions", Actions, true),
            ("settle_ms", Text, false),
            ("capture", Whole, false),
        ],
        // An application's environment may carry credentials; its command line is what it runs.
        "launch_application" => &[
            ("command", Text, true),
            ("args", Whole, false),
            ("name", Text, false),
            ("cwd", Text, false),
            ("gpu", Text, false),
            ("env", Keys, false),
        ],
        "get_application" | "application_diagnostics" => &[("application_id", Text, true)],
        "application_logs" => &[("application_id", Text, true), ("tail_bytes", Text, false)],
        "stop_application" => &[
            ("application_id", Text, true),
            ("grace_seconds", Text, false),
        ],
        "sample_metrics" => &[("application_id", Text, true), ("duration_ms", Text, false)],
        // A search is the agent's own words about the desktop, short and bounded.
        "list_processes" => &[
            ("sort", Text, false),
            ("limit", Text, false),
            ("query", Text, true),
            ("interval_ms", Text, false),
        ],
        "kill_process" => &[
            ("pid", Text, true),
            ("signal", Text, true),
            ("tree", Text, false),
            ("root", Text, true),
            ("wait_ms", Text, false),
        ],
        "focus_window" => &[("window_id", Text, true)],
        "set_window_geometry" => &[
            ("window_id", Text, true),
            ("x", Text, false),
            ("y", Text, false),
            ("width", Text, false),
            ("height", Text, false),
        ],
        "list_files" => &[("path", Text, true), ("limit", Text, false)],
        "move_file" => &[
            ("source", Text, true),
            ("destination", Text, true),
            ("replace", Text, false),
        ],
        "delete_file" => &[("path", Text, true), ("recursive", Text, true)],
        "download_file" => &[("path", Text, true), ("max_bytes", Text, false)],
        "get_changes" => &[
            ("path", Text, true),
            ("kinds", Text, false),
            ("all", Text, false),
            ("limit", Text, false),
        ],
        "wait_for_condition" => &[
            ("window_title", Text, true),
            ("text", Text, true),
            ("application_id", Text, false),
            ("application_state", Text, false),
            ("log_contains", Text, false),
            ("screen_stable_ms", Text, false),
            ("screen_changed", Text, false),
            ("screen_change_percent", Text, false),
            ("text_state", Text, false),
            ("region", Text, false),
            ("timeout_ms", Text, false),
            ("interval_ms", Text, false),
        ],
        "find_text" => &[
            ("text", Text, true),
            ("region", Text, false),
            ("level", Text, false),
            ("min_confidence", Text, false),
            ("case_sensitive", Text, false),
        ],
        "start_recording" => &[
            ("name", Text, true),
            ("fps", Text, false),
            ("max_seconds", Text, false),
            ("max_width", Text, false),
            ("region", Text, false),
        ],
        "stop_recording" => &[("recording_id", Text, true)],
        "export_artifacts" => &[
            ("destination", Text, true),
            ("name", Text, true),
            ("recording_ids", Whole, false),
            ("application_ids", Whole, false),
            ("replace", Text, false),
        ],
        "capture_audio" => &[("source", Text, true), ("seconds", Text, false)],
        "inject_audio" => &[("target", Text, true)],
        // A clipboard can hold a password or a token, so its text is kept as its size.
        "get_clipboard" => &[("selection", Text, true)],
        "set_clipboard" => &[
            ("selection", Text, true),
            ("paste", Text, false),
            ("paste_keys", Text, false),
        ],
        "open_terminal" => &[("cwd", Text, true), ("root", Text, true)],
        // A terminal is where a password prompt waits, so what is typed into one is kept as its size.
        "terminal_input" => &[("terminal_id", Text, true), ("press_enter", Text, false)],
        "terminal_output" => &[
            ("terminal_id", Text, true),
            ("offset", Text, false),
            ("limit", Text, false),
            ("strip_ansi", Text, false),
        ],
        "close_terminal" => &[("terminal_id", Text, true)],
        "inspect_ui" => &[
            ("name", Text, true),
            ("pid", Text, false),
            ("max_depth", Text, false),
            ("max_nodes", Text, false),
        ],
        "gamepad_input" => &[("actions", Whole, false)],
        _ => return None,
    })
}

/// A call's arguments as the log keeps them, the line that sums them up, and the masked values an
/// error text must not repeat.
pub(super) struct Masked {
    pub arguments: Value,
    pub summary: String,
    secrets: Vec<String>,
}

impl Masked {
    /// `text` without any masked value it quotes.
    pub fn redact(&self, text: &str) -> String {
        let mut text = text.to_owned();
        for secret in &self.secrets {
            if text.contains(secret.as_str()) {
                text = text.replace(secret.as_str(), "***");
            }
        }
        text
    }
}

/// Mask the arguments of a call to `tool`.
pub(super) fn mask(source: Source, tool: &str, arguments: &Map<String, Value>) -> Masked {
    let rules: &[Rule] = match source {
        Source::Server => &[],
        Source::Transfer => TRANSFER,
        Source::Tool => policy(tool).unwrap_or(&[]),
    };
    let rule = |key: &str| {
        (source != Source::Server)
            .then(|| COMMON.iter().chain(rules).find(|rule| rule.0 == key))
            .flatten()
    };
    let mut kept = Map::new();
    let mut secrets = Vec::new();
    for (key, value) in arguments.iter().take(KEYS) {
        let shown = match rule(key) {
            Some(&(_, keep, _)) => keep_value(keep, value, &mut secrets),
            None => masked(value, &mut secrets),
        };
        kept.insert(super::clip(key, KEY), shown);
    }
    if arguments.len() > KEYS {
        kept.insert("$more".into(), json!(arguments.len() - KEYS));
    }
    // Longest first, so a value inside another is not left half replaced.
    secrets.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    secrets.dedup();
    let summary = match source {
        Source::Server => arguments
            .keys()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        _ => summary(rules, &kept),
    };
    Masked {
        arguments: Value::Object(kept),
        summary: super::clip(&summary, SUMMARY),
        secrets,
    }
}

fn keep_value(keep: Keep, value: &Value, secrets: &mut Vec<String>) -> Value {
    match (keep, value) {
        (Text | Script, Value::String(text)) => {
            cut(text, if keep == Script { SCRIPT } else { TEXT })
        }
        (Url, Value::String(text)) => {
            let (kept, hidden) = crate::desktop_mcp::split_query(text);
            if !hidden.is_empty() {
                remember(&Value::String(hidden.to_owned()), secrets);
            }
            cut(kept, TEXT)
        }
        (Text | Script | Url, Value::Array(_) | Value::Object(_)) | (Whole, _) => whole(value),
        (Text | Script | Url, _) => value.clone(),
        (Keys, Value::Object(object)) => Value::Object(
            object
                .iter()
                .take(KEYS)
                .map(|(key, value)| (super::clip(key, KEY), masked(value, secrets)))
                .collect(),
        ),
        (Actions, Value::Array(actions)) => Value::Array(
            actions
                .iter()
                .take(100)
                .map(|action| match action {
                    Value::Object(action) => Value::Object(
                        action
                            .iter()
                            .map(|(key, value)| (key.clone(), input(key, value, secrets)))
                            .collect(),
                    ),
                    other => masked(other, secrets),
                })
                .collect(),
        ),
        (Keys | Actions, _) => masked(value, secrets),
    }
}

/// One field of a desktop input action: typed text by its size, a key of a single character
/// masked, since a password can be pressed a key at a time, and the rest as it is.
fn input(key: &str, value: &Value, secrets: &mut Vec<String>) -> Value {
    match (key, value) {
        ("text", _) => masked(value, secrets),
        ("keys", Value::Array(keys)) => Value::Array(
            keys.iter()
                .map(|name| match name {
                    Value::String(name) if name.chars().count() > 1 => {
                        Value::String(super::clip(name, KEY))
                    }
                    other => masked(other, secrets),
                })
                .collect(),
        ),
        (_, Value::String(text)) => Value::String(super::clip(text, KEY)),
        (_, Value::Number(_) | Value::Bool(_) | Value::Null) => value.clone(),
        _ => masked(value, secrets),
    }
}

/// A value as its type and size, noting what an error text must not repeat.
fn masked(value: &Value, secrets: &mut Vec<String>) -> Value {
    remember(value, secrets);
    match value {
        Value::String(text) => json!({"$masked": "string", "length": text.chars().count()}),
        Value::Number(_) => json!({"$masked": "number"}),
        Value::Bool(_) => json!({"$masked": "boolean"}),
        Value::Null => Value::Null,
        Value::Array(items) => json!({"$masked": "array", "items": items.len()}),
        Value::Object(object) => json!({"$masked": "object", "keys": object.len()}),
    }
}

fn remember(value: &Value, secrets: &mut Vec<String>) {
    if secrets.len() >= 256 {
        return;
    }
    match value {
        Value::String(text) if text.chars().count() >= SECRET => secrets.push(text.clone()),
        Value::Number(number) if number.to_string().len() >= SECRET => {
            secrets.push(number.to_string())
        }
        Value::Array(items) => items.iter().for_each(|item| remember(item, secrets)),
        Value::Object(object) => object.values().for_each(|item| remember(item, secrets)),
        _ => {}
    }
}

/// A text, or its start when it is longer than `limit` characters.
fn cut(text: &str, limit: usize) -> Value {
    let length = text.chars().count();
    if length <= limit {
        return Value::String(text.to_owned());
    }
    json!({"$truncated": text.chars().take(limit).collect::<String>(), "length": length})
}

/// A list or an object, or the start of its JSON when that is longer than [`WHOLE`] bytes.
fn whole(value: &Value) -> Value {
    let text = value.to_string();
    if text.len() <= WHOLE {
        return value.clone();
    }
    let mut end = WHOLE;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    json!({"$truncated": &text[..end], "length": text.chars().count()})
}

/// The kept arguments that say most about a call, on one line: those its policy leads with, in
/// its order, or else the first few kept values.
fn summary(rules: &[Rule], kept: &Map<String, Value>) -> String {
    let parts: Vec<String> = rules
        .iter()
        .filter(|rule| rule.2)
        .filter_map(|&(key, keep, _)| shown(key, keep, kept.get(key)?))
        .collect();
    if !parts.is_empty() {
        return parts.join(" · ");
    }
    kept.iter()
        .filter_map(|(key, value)| match value {
            Value::String(text) => Some(format!("{key}={}", first_line(text))),
            Value::Number(number) => Some(format!("{key}={number}")),
            Value::Bool(true) => Some(key.clone()),
            _ => None,
        })
        .take(3)
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A kept value on the summary line, or nothing for one that is masked, empty or false.
fn shown(key: &str, keep: Keep, value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.trim().is_empty() => Some(first_line(text)),
        Value::Number(number) => Some(format!("{key}={number}")),
        Value::Bool(true) => Some(key.to_owned()),
        Value::Array(actions) if keep == Actions => {
            let mut types: Vec<&str> = actions
                .iter()
                .filter_map(|action| action["type"].as_str())
                .take(6)
                .collect();
            if types.is_empty() {
                return None;
            }
            let more = actions.len().saturating_sub(types.len());
            let more = (more > 0).then(|| format!("+{more}"));
            types.extend(more.as_deref());
            Some(types.join(", "))
        }
        Value::Object(object) => object
            .get("$truncated")
            .and_then(Value::as_str)
            .map(first_line),
        _ => None,
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
#[path = "mask_tests.rs"]
mod tests;
