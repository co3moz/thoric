//! Pins down which searches are refused before anything runs, the defaults of one that names only
//! what it looks for, that the request the helper reads on stdin is one line with the pattern in
//! it, and that the tool says it only reads.
use super::*;
use crate::test_support::app;
use axum::http::StatusCode;
use serde_json::json;

fn request(value: Value) -> std::result::Result<SearchFiles, serde_json::Error> {
    serde_json::from_value(value)
}

#[test]
fn a_search_has_bounded_defaults() {
    let search = request(json!({"path": "/home/desktop", "pattern": "TODO"})).unwrap();
    assert!(search.validate().is_ok());
    assert!(search.case_sensitive && !search.literal && !search.include_ignored);
    assert_eq!(
        (search.max_results, search.context, search.max_seconds),
        (100, 0, 10)
    );
    assert!(request(json!({"path": "/", "name": "*.py"})).is_ok_and(|s| s.validate().is_ok()));
    // An argument nobody reads is refused, not ignored.
    assert!(request(json!({"path": "/", "pattern": "x", "root": true})).is_err());
}

#[test]
fn searches_out_of_bounds_are_refused() {
    let refused = |value: Value| request(value).unwrap().validate().unwrap_err().to_string();
    assert!(refused(json!({"path": "/home/desktop"})).contains("pattern, name or both"));
    for path in ["relative", "/home/../etc", "/home/desktop/", "/a\nb", ""] {
        assert!(
            request(json!({"path": path, "pattern": "x"}))
                .unwrap()
                .validate()
                .is_err(),
            "{path:?}"
        );
    }
    for pattern in [
        String::new(),
        "two\nlines".to_owned(),
        "x".repeat(PATTERN + 1),
    ] {
        assert!(refused(json!({"path": "/", "pattern": pattern})).contains("pattern must be"));
    }
    assert!(
        request(json!({"path": "/", "pattern": "é".repeat(PATTERN)}))
            .unwrap()
            .validate()
            .is_ok()
    );
    for name in [String::new(), "src/*.py".to_owned(), "x".repeat(NAME + 1)] {
        assert!(refused(json!({"path": "/", "name": name})).contains("name must be"));
    }
    for (key, value) in [
        ("max_results", 0),
        ("max_results", MAX_RESULTS as u64 + 1),
        ("context", u64::from(MAX_CONTEXT) + 1),
        ("max_seconds", 0),
        ("max_seconds", MAX_SECONDS + 1),
    ] {
        let mut body = json!({"path": "/", "pattern": "x"});
        body[key] = json!(value);
        assert!(refused(body).contains(key), "{key}={value}");
    }
    for (key, value) in [
        ("max_results", MAX_RESULTS as u64),
        ("context", u64::from(MAX_CONTEXT)),
        ("max_seconds", MAX_SECONDS),
    ] {
        let mut body = json!({"path": "/", "pattern": "x"});
        body[key] = json!(value);
        assert!(request(body).unwrap().validate().is_ok(), "{key}={value}");
    }
}

/// The helper reads its request from stdin as one line, so whatever a pattern holds, a quote, a
/// dash or a shell's own characters, is data there and never an argument of a command.
#[test]
fn the_helpers_request_is_one_line_of_json() {
    let search = request(json!({
        "path": "/home/desktop/it's here",
        "pattern": "--help; $(reboot) \"quoted\" \u{2028}",
        "name": "*.py",
        "literal": true,
        "context": 2,
    }))
    .unwrap();
    assert!(search.validate().is_ok());
    let line = serde_json::to_string(&search).unwrap();
    assert!(!line.contains('\n'));
    let read: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(read["pattern"], "--help; $(reboot) \"quoted\" \u{2028}");
    assert_eq!(read["path"], "/home/desktop/it's here");
    assert_eq!(
        (&read["literal"], &read["context"], &read["max_seconds"]),
        (&json!(true), &json!(2), &json!(10))
    );
    // What was not given is null, which the helper reads as absent.
    let names = request(json!({"path": "/", "name": "*.py"})).unwrap();
    assert!(serde_json::to_value(&names).unwrap()["pattern"].is_null());
}

#[tokio::test]
async fn a_bad_search_is_refused_before_docker() {
    let app = app(false);
    let bad = request(json!({"path": "/", "pattern": "x", "max_results": 0})).unwrap();
    let refused = search_files(&app, "missing", &bad).await.unwrap_err();
    assert_eq!(refused.0, StatusCode::BAD_REQUEST);
    // A good search of a container that does not exist goes no further than its record.
    let good = request(json!({"path": "/", "pattern": "x"})).unwrap();
    let missing = search_files(&app, "missing", &good).await.unwrap_err();
    assert_eq!(missing.0, StatusCode::NOT_FOUND);
}

#[test]
fn the_tool_only_reads_and_needs_a_path() {
    let tool = crate::mcp::listed_tool("search_files").unwrap();
    let annotations = tool.annotations.unwrap();
    assert_eq!(annotations.read_only_hint, Some(true));
    assert_eq!(annotations.idempotent_hint, Some(true));
    assert_eq!(
        tool.input_schema["required"],
        json!(["path", "container_id"])
    );
    for argument in [
        "pattern",
        "literal",
        "name",
        "case_sensitive",
        "max_results",
        "context",
        "include_ignored",
        "max_seconds",
    ] {
        assert!(
            tool.input_schema["properties"][argument].is_object(),
            "{argument}"
        );
    }
    // The description names the limits the helper has, so they are read from it here.
    let description = tool.description.unwrap();
    let helper = HELPER;
    for (said, constant) in [
        ("100000 looked at", "ENTRIES = 100_000"),
        ("256 KiB of results", "ANSWER = 256 * 1024"),
        ("4 MiB of a file", "FILE_BYTES = 4 * 1024 * 1024"),
        ("400 characters", "LINE = 400"),
    ] {
        assert!(description.contains(said), "{said}");
        assert!(helper.contains(constant), "{constant}");
    }
}
