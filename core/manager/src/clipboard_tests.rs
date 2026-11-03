//! Pins down what a clipboard request takes: its defaults, a text bounded in bytes of UTF-8 so
//! Turkish letters, CJK and emoji count as what they weigh, no NUL, a paste only of the
//! clipboard, the chords a paste presses, the tools' annotations and the endpoint scopes that
//! offer them, that a text at the limit fits a request however JSON escapes it, the statuses of a
//! helper that timed out or failed, and that a missing container is answered before anything runs.
use super::*;
use crate::{
    desktop_input::Step,
    test_support::{app, mcp_app, mcp_request},
};
use axum::{body::Body, http::Request};
use tower::ServiceExt;

const UNICODE: &str = "Türkçe: ç ğ ı İ ö ş ü / 中文 日本語 한국어 / emoji 😀👍🏽";

fn set(value: Value) -> serde_json::Result<SetClipboard> {
    serde_json::from_value(value)
}

#[test]
fn requests_take_the_clipboard_and_ctrl_v_unless_they_say_otherwise() {
    let get: GetClipboard = serde_json::from_value(json!({})).unwrap();
    assert_eq!(get.selection, Selection::Clipboard);
    let get: GetClipboard = serde_json::from_value(json!({"selection": "primary"})).unwrap();
    assert_eq!(get.selection, Selection::Primary);
    // A read has one selection; both is what a set gives the text to.
    assert!(serde_json::from_value::<GetClipboard>(json!({"selection": "both"})).is_err());

    let body = set(json!({"text": UNICODE})).unwrap();
    assert_eq!(body.text, UNICODE);
    assert_eq!(
        (body.selection, body.paste, body.paste_keys),
        (Selections::Clipboard, false, PasteKeys::CtrlV)
    );
    let body =
        set(json!({"text": "x", "selection": "both", "paste": true, "paste_keys": "ctrl+shift+v"}))
            .unwrap();
    assert_eq!(
        (body.selection, body.paste, body.paste_keys),
        (Selections::Both, true, PasteKeys::CtrlShiftV)
    );
    for bad in [
        json!({}),
        json!({"text": "x", "paste_keys": "shift+insert"}),
        json!({"text": "x", "container_id": "c1"}),
    ] {
        assert!(set(bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn the_limit_counts_bytes_of_utf8_and_a_paste_needs_the_clipboard() {
    // Two bytes a letter: exactly the limit passes, and one byte more is refused.
    let full = "ğ".repeat(LIMIT / 2);
    assert_eq!(full.len(), LIMIT);
    assert!(set(json!({"text": full})).unwrap().validate().is_ok());
    let over = set(json!({"text": format!("{full}a")}))
        .unwrap()
        .validate()
        .unwrap_err()
        .to_string();
    assert!(over.contains(&format!("{} bytes", LIMIT)), "{over}");
    assert!(over.contains(&format!("{}", LIMIT + 1)), "{over}");
    // An emoji is four bytes, though it is one character.
    let emoji = "😀".repeat(LIMIT / 4 + 1);
    assert!(emoji.chars().count() < LIMIT);
    assert!(set(json!({"text": emoji})).unwrap().validate().is_err());

    assert!(set(json!({"text": UNICODE})).unwrap().validate().is_ok());
    assert!(set(json!({"text": ""})).unwrap().validate().is_ok());
    let nul = set(json!({"text": "a\u{0}b"}))
        .unwrap()
        .validate()
        .unwrap_err();
    assert!(nul.to_string().contains("NUL"), "{nul}");
    let primary = set(json!({"text": "x", "selection": "primary", "paste": true}))
        .unwrap()
        .validate()
        .unwrap_err();
    assert!(
        primary.to_string().contains("clipboard or both"),
        "{primary}"
    );
    for selection in ["clipboard", "both"] {
        let body = set(json!({"text": "x", "selection": selection, "paste": true})).unwrap();
        assert!(body.validate().is_ok(), "{selection}");
    }
    assert!(
        set(json!({"text": "x", "selection": "primary"}))
            .unwrap()
            .validate()
            .is_ok()
    );
}

/// The keysyms a paste chord sends, pressed in order and released in reverse.
fn chord(keys: PasteKeys) -> Vec<(bool, u32)> {
    let actions = [Action::KeyPress { keys: keys.keys() }];
    let planned = desktop_input::plan(&actions, 1920, 1080, None).unwrap();
    let [Step::Keys(bytes)] = planned.steps.as_slice() else {
        panic!("a paste is one key press");
    };
    bytes
        .chunks(8)
        .map(|event| {
            assert_eq!(event[0], 4, "an RFB key event");
            (
                event[1] == 1,
                u32::from_be_bytes(event[4..8].try_into().unwrap()),
            )
        })
        .collect()
}

#[test]
fn a_paste_presses_its_chord_with_the_capital_letter_under_shift() {
    let (control, shift) = (0xffe3, 0xffe1);
    assert_eq!(
        chord(PasteKeys::CtrlV),
        [
            (true, control),
            (true, 0x76),
            (false, 0x76),
            (false, control)
        ]
    );
    // A small v under Shift would have the VNC server release Shift to type it.
    assert_eq!(
        chord(PasteKeys::CtrlShiftV),
        [
            (true, control),
            (true, shift),
            (true, 0x56),
            (false, 0x56),
            (false, shift),
            (false, control)
        ]
    );
    assert_eq!(
        (PasteKeys::CtrlV.id(), PasteKeys::CtrlShiftV.id()),
        ("ctrl+v", "ctrl+shift+v")
    );
}

#[tokio::test]
async fn reading_is_a_view_and_setting_is_input() {
    let get = crate::mcp::listed_tool("get_clipboard").unwrap();
    let annotations = get.annotations.unwrap();
    assert_eq!(annotations.read_only_hint, Some(true));
    let set = crate::mcp::listed_tool("set_clipboard").unwrap();
    let annotations = set.annotations.unwrap();
    assert_eq!(annotations.read_only_hint, Some(false));
    assert_eq!(annotations.destructive_hint, Some(true));
    // A second paste pastes again.
    assert_eq!(annotations.idempotent_hint, Some(false));
    assert_eq!(
        set.input_schema["required"],
        json!(["text", "container_id"])
    );
    let keys = &set.input_schema["properties"]["paste_keys"];
    let keys = match keys["$ref"].as_str() {
        Some(name) => &set.input_schema["$defs"][name.trim_start_matches("#/$defs/")],
        None => keys,
    };
    assert_eq!(keys["enum"], json!(["ctrl+v", "ctrl+shift+v"]));

    let scopes = crate::desktop_mcp::scopes_list().await.0;
    let tools = |scope: &str| {
        scopes["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == scope)
            .unwrap()["tools"]
            .clone()
    };
    let offering = |tool: &str| {
        scopes["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["tools"].as_array().unwrap().contains(&json!(tool)))
            .map(|entry| entry["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert!(
        tools("view")
            .as_array()
            .unwrap()
            .contains(&json!("get_clipboard"))
    );
    assert_eq!(offering("get_clipboard"), ["view"]);
    assert_eq!(offering("set_clipboard"), ["input"]);
}

#[tokio::test]
async fn a_missing_container_is_answered_before_anything_runs() {
    let response = crate::router(app(false))
        .oneshot(
            Request::builder()
                .uri("/api/containers/missing/clipboard?selection=primary")
                .header("host", "localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let error = set_clipboard(
        &app(false),
        "missing",
        &set(json!({"text": UNICODE})).unwrap(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::NOT_FOUND);
    // A refused text is refused before the container is looked up.
    let error = set_clipboard(
        &app(false),
        "missing",
        &set(json!({"text": "a\u{0}"})).unwrap(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_text_at_the_limit_fits_a_request_even_when_json_doubles_every_byte() {
    // A quote takes two bytes in JSON, as a newline and a backslash do.
    let quotes = "\"".repeat(LIMIT);
    let body = json!({"text": quotes}).to_string();
    assert!(body.len() > 2 * LIMIT && body.len() < crate::mcp::BODY_LIMIT);
    let response = crate::router(app(false))
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/containers/missing/clipboard")
                .header("host", "localhost")
                .header("content-type", "application/json")
                .header("x-thoric-request", "1")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    // Past the body limit and the text's own check, only the container it names is missing.
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let text = |reply: &Value| {
        assert_eq!(reply["result"]["isError"], true, "{reply}");
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let call = |text: String| json!({"name": "set_clipboard", "arguments": {"container_id": "missing", "text": text}});
    let (status, reply) = mcp_request(mcp_app(), "tools/call", call(quotes)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(text(&reply).contains("Record not found"), "{reply}");
    // One byte more is refused by the check, which says why.
    let (_, reply) = mcp_request(mcp_app(), "tools/call", call("a".repeat(LIMIT + 1))).await;
    assert!(text(&reply).contains("limited to"), "{reply}");
}

#[cfg(unix)]
#[test]
fn a_helper_that_timed_out_or_failed_is_not_the_requests_fault() {
    use std::os::unix::process::ExitStatusExt;
    for (code, status) in [
        (1, StatusCode::BAD_REQUEST),
        (3, StatusCode::BAD_REQUEST),
        (TIMED_OUT, StatusCode::GATEWAY_TIMEOUT),
        (FAILED, StatusCode::BAD_GATEWAY),
    ] {
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: Vec::new(),
            stderr: b"Clipboard operation failed: why\n".to_vec(),
        };
        let Error(answered, message) = failure(&output);
        assert_eq!(answered, status, "{code}");
        assert_eq!(message, "Clipboard operation failed: why");
    }
    // The helper exits with the codes this side reads.
    assert!(HELPER.contains(&format!(
        "class TimedOut(Failure):\n    code = {TIMED_OUT}\n"
    )));
    assert!(HELPER.contains(&format!(
        "    code = {FAILED}\n\n\nclass Unsupported(Failure):"
    )));
}
