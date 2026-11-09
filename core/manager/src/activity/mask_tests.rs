//! Pins down what the activity log keeps of a call's arguments: that file contents, compose values,
//! base64 payloads, headers, environment values, typed text and a desktop server's arguments are
//! kept as their size only, that a script and the other kept values are cut, and that an error
//! text never repeats a masked value.
use super::*;
use serde_json::json;

fn masked_tool(tool: &str, arguments: Value) -> Masked {
    mask(Source::Tool, tool, arguments.as_object().unwrap())
}

/// Whether `text` appears anywhere in the kept arguments.
fn keeps(masked: &Masked, text: &str) -> bool {
    masked.arguments.to_string().contains(text)
}

#[test]
fn a_written_files_content_is_kept_as_its_size_and_its_path_as_it_is() {
    let masked = masked_tool(
        "write_file",
        json!({"container_id": "c1", "path": "/home/desktop/.netrc", "data": "machine x password hunter22", "replace": true}),
    );
    assert_eq!(
        masked.arguments,
        json!({
            "container_id": "c1",
            "path": "/home/desktop/.netrc",
            "data": {"$masked": "string", "length": 27},
            "replace": true,
        })
    );
    assert!(!keeps(&masked, "hunter22"));
    assert_eq!(masked.summary, "/home/desktop/.netrc");
}

#[test]
fn compose_values_and_composes_are_kept_as_their_size() {
    let masked = masked_tool(
        "create_container",
        json!({"name": "Unity", "image": "sha256:abc", "parameters": {"LICENSE_KEY": "SECRET-1234", "PORT": "8080"}}),
    );
    assert_eq!(
        masked.arguments["parameters"],
        json!({"LICENSE_KEY": {"$masked": "string", "length": 11}, "PORT": {"$masked": "string", "length": 4}})
    );
    assert!(!keeps(&masked, "SECRET-1234"));
    assert_eq!(masked.summary, "Unity · sha256:abc");
    for (tool, arguments) in [
        (
            "set_image_compose",
            json!({"image_id": "i1", "compose": {"parameters": [{"name": "TOKEN", "default": "SECRET-1234"}], "script": "echo"}}),
        ),
        (
            "build_image",
            json!({"name": "x", "base_image": "y", "script": "apt-get install -y z", "compose": {"script": "SECRET-1234"}, "icon": "data:image/png;base64,SECRET-1234"}),
        ),
    ] {
        let masked = masked_tool(tool, arguments);
        assert!(
            !keeps(&masked, "SECRET-1234"),
            "{tool}: {}",
            masked.arguments
        );
    }
}

#[test]
fn headers_environment_and_arguments_of_a_registered_server_are_kept_as_their_size() {
    let masked = masked_tool(
        "register_mcp_server",
        json!({"container_id": "c1", "name": "api", "url": "http://127.0.0.1:8931/mcp",
               "headers": {"Authorization": "Bearer bsk_0123456789"}, "env": {"API_KEY": "sk-live-0123"},
               "args": ["--token", "sk-live-4567"]}),
    );
    for secret in ["bsk_0123456789", "sk-live-0123", "sk-live-4567"] {
        assert!(!keeps(&masked, secret), "{secret}");
    }
    assert_eq!(
        masked.arguments["headers"]["Authorization"]["$masked"],
        "string"
    );
    assert_eq!(
        masked.arguments["args"],
        json!({"$masked": "array", "items": 2})
    );
    assert_eq!(masked.arguments["url"], "http://127.0.0.1:8931/mcp");
}

#[test]
fn a_registered_servers_url_is_kept_without_its_query() {
    let masked = masked_tool(
        "register_mcp_server",
        json!({"container_id": "c1", "name": "api",
               "url": "http://127.0.0.1:8931/mcp?token=tok_0123456789#part"}),
    );
    assert_eq!(masked.arguments["url"], "http://127.0.0.1:8931/mcp");
    assert!(!keeps(&masked, "tok_0123456789"));
    // The error text of a refusal that quotes the URL loses the query too.
    assert!(
        masked
            .secrets
            .iter()
            .any(|secret| secret.contains("tok_0123456789"))
    );
}

#[test]
fn base64_payloads_are_kept_as_their_size() {
    for (tool, key) in [
        ("inject_audio", "data_base64"),
        ("upload_file", "data_base64"),
    ] {
        let masked = masked_tool(
            tool,
            json!({"container_id": "c1", key: "UklGRiQAAABXQVZFZm10IBAAAAA=", "target": "microphone", "destination": "/home/desktop", "name": "a.wav"}),
        );
        assert_eq!(
            masked.arguments[key],
            json!({"$masked": "string", "length": 28}),
            "{tool}"
        );
    }
}

#[test]
fn clipboard_text_is_kept_as_its_size() {
    let masked = masked_tool(
        "set_clipboard",
        json!({"container_id": "c1", "text": "hunter2 şifre 密码", "selection": "both", "paste": true, "paste_keys": "ctrl+shift+v"}),
    );
    assert!(!keeps(&masked, "hunter2"));
    assert_eq!(
        masked.arguments["text"],
        json!({"$masked": "string", "length": 16})
    );
    assert_eq!(masked.arguments["selection"], "both");
    assert_eq!(masked.arguments["paste_keys"], "ctrl+shift+v");
}

#[test]
fn typed_text_and_single_keys_are_masked_while_clicks_and_named_keys_stay() {
    let actions = json!([
        {"type": "click", "x": 400, "y": 250},
        {"type": "type_text", "text": "correct horse"},
        {"type": "key_press", "keys": ["Control", "a"]},
        {"type": "key_press", "keys": ["Enter"]},
    ]);
    for tool in ["desktop_input", "act_and_observe"] {
        let masked = masked_tool(tool, json!({"container_id": "c1", "actions": actions}));
        assert!(!keeps(&masked, "correct horse"), "{tool}");
        let kept = &masked.arguments["actions"];
        assert_eq!(kept[0], json!({"type": "click", "x": 400, "y": 250}));
        assert_eq!(kept[1]["text"], json!({"$masked": "string", "length": 13}));
        assert_eq!(
            kept[2]["keys"],
            json!(["Control", {"$masked": "string", "length": 1}])
        );
        assert_eq!(kept[3]["keys"], json!(["Enter"]));
        assert_eq!(masked.summary, "click, type_text, key_press, key_press");
    }
    let masked = masked_tool(
        "terminal_input",
        json!({"container_id": "c1", "terminal_id": "t1", "text": "sudo-password", "press_enter": true}),
    );
    assert!(!keeps(&masked, "sudo-password"));
    assert_eq!(masked.arguments["press_enter"], true);
}

#[test]
fn a_desktop_servers_arguments_keep_their_keys_and_sizes_whatever_its_tool_is_called() {
    // A server's tool may share a Thoric tool's name; its arguments stay opaque.
    for tool in ["browser_type", "exec_container", "write_file"] {
        let masked = mask(
            Source::Server,
            tool,
            json!({"script": "rm -rf /", "path": "/etc/passwd", "count": 3, "submit": true, "fields": {"a": 1}})
                .as_object()
                .unwrap(),
        );
        assert_eq!(
            masked.arguments,
            json!({
                "count": {"$masked": "number"},
                "fields": {"$masked": "object", "keys": 1},
                "path": {"$masked": "string", "length": 11},
                "script": {"$masked": "string", "length": 8},
                "submit": {"$masked": "boolean"},
            }),
            "{tool}"
        );
        assert_eq!(masked.summary, "count, fields, path, script, submit");
    }
}

#[test]
fn an_argument_without_a_policy_and_a_tool_mcp_does_not_offer_keep_only_their_size() {
    let masked = masked_tool(
        "exec_container",
        json!({"container_id": "c1", "script": "id", "added_later": "a value"}),
    );
    assert_eq!(
        masked.arguments["added_later"],
        json!({"$masked": "string", "length": 7})
    );
    let masked = masked_tool("no_such_tool", json!({"path": "/etc/shadow"}));
    assert_eq!(
        masked.arguments,
        json!({"path": {"$masked": "string", "length": 11}})
    );
}

#[test]
fn a_script_is_kept_up_to_its_limit_and_a_text_up_to_its_own() {
    let script = format!("apt-get install -y curl\n{}", "x".repeat(SCRIPT * 2));
    let masked = masked_tool(
        "exec_container",
        json!({"container_id": "c1", "script": script, "root": true, "timeout_seconds": 60}),
    );
    let kept = &masked.arguments["script"];
    assert_eq!(kept["length"], script.chars().count());
    assert_eq!(kept["$truncated"].as_str().unwrap().chars().count(), SCRIPT);
    assert!(
        kept["$truncated"]
            .as_str()
            .unwrap()
            .starts_with("apt-get install -y curl\n")
    );
    assert_eq!(masked.summary, "apt-get install -y curl · root");
    let masked = masked_tool(
        "read_file",
        json!({"container_id": "c1", "path": "x".repeat(TEXT + 1)}),
    );
    assert_eq!(masked.arguments["path"]["length"], TEXT + 1);
    let masked = masked_tool("exec_container", json!({"script": "whoami"}));
    assert_eq!(masked.arguments["script"], "whoami");
}

#[test]
fn a_scripts_files_keep_their_paths_and_their_contents_as_type_and_size() {
    let masked = masked_tool(
        "exec_container",
        json!({
            "container_id": "c1",
            "script": "python3 /tmp/task.py",
            "wait_seconds": 30,
            "files": [
                {"path": "/tmp/task.py", "data": "token = 'hunter22-secret'", "replace": true},
                {"path": "/tmp/in.bin", "data": "aHVudGVyMjI=", "encoding": "base64", "extra": "kept as size"},
                "not a file",
            ],
        }),
    );
    assert_eq!(
        masked.arguments["files"],
        json!([
            {"path": "/tmp/task.py", "data": {"$masked": "string", "length": 25}, "replace": true},
            {
                "path": "/tmp/in.bin",
                "data": {"$masked": "string", "length": 12},
                "encoding": "base64",
                "extra": {"$masked": "string", "length": 12},
            },
            {"$masked": "string", "length": 10},
        ])
    );
    assert_eq!(masked.arguments["wait_seconds"], 30);
    for content in ["hunter22", "aHVudGVyMjI=", "not a file"] {
        assert!(!keeps(&masked, content), "{content}");
    }
    assert_eq!(
        masked.redact("files: /tmp/task.py: unexpected token = 'hunter22-secret'"),
        "files: /tmp/task.py: unexpected ***"
    );
    // Something else under the name is kept as its size, and a path's wrong type too.
    let masked = masked_tool(
        "exec_container",
        json!({"script": "id", "files": "print(1)"}),
    );
    assert_eq!(
        masked.arguments["files"],
        json!({"$masked": "string", "length": 8})
    );
    let masked = masked_tool(
        "exec_container",
        json!({"script": "id", "files": [{"path": {"data": "hunter22"}}]}),
    );
    assert!(!keeps(&masked, "hunter22"));
}

#[test]
fn an_error_text_never_repeats_a_masked_value() {
    let masked = masked_tool(
        "write_file",
        json!({"container_id": "c1", "path": "/tmp/x", "data": "hunter22", "encoding": 5}),
    );
    assert_eq!(
        masked.redact("Invalid tool arguments: unexpected \"hunter22\" for /tmp/x"),
        "Invalid tool arguments: unexpected \"***\" for /tmp/x"
    );
    // Values shorter than a word would be found everywhere and stay.
    let masked = masked_tool("terminal_input", json!({"terminal_id": "t", "text": "ls"}));
    assert_eq!(masked.redact("ls failed"), "ls failed");
}

#[test]
fn a_desktop_servers_call_with_many_arguments_keeps_a_bounded_number() {
    let arguments: Map<String, Value> = (0..100).map(|i| (format!("k{i:03}"), json!(i))).collect();
    let masked = mask(Source::Server, "t", &arguments);
    let kept = masked.arguments.as_object().unwrap();
    assert_eq!(kept.len(), KEYS + 1);
    assert_eq!(kept["$more"], 100 - KEYS);
}

#[test]
fn a_transfer_keeps_the_paths_and_names_it_was_issued_for() {
    let masked = mask(
        Source::Transfer,
        "container_download",
        json!({"container_id": "c1", "path": "/home/desktop/report.pdf"})
            .as_object()
            .unwrap(),
    );
    assert_eq!(
        masked.arguments,
        json!({"container_id": "c1", "path": "/home/desktop/report.pdf"})
    );
    assert_eq!(masked.summary, "/home/desktop/report.pdf");
}
