//! How a notice reaches curl and the address: a configuration on stdin that curl reads back to
//! the exact body, whatever the text holds, a real curl posting it to a receiver of the test's
//! own, and answers and failures said without the address.
use super::*;
use crate::notify::target::targets;
use serde_json::json;

fn target(address: &str) -> Target {
    targets(Some(address)).unwrap().remove(0)
}

/// A quoted value of curl's configuration as curl reads it: a backslash keeps the character
/// after it, except for t, n, r and v.
fn unquote(line: &str, key: &str) -> String {
    let quoted = line
        .strip_prefix(&format!("{key} = \""))
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or_else(|| panic!("{line}"));
    let mut value = String::new();
    let mut chars = quoted.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => value.push(match chars.next().unwrap() {
                't' => '\t',
                'n' => '\n',
                'r' => '\r',
                'v' => '\u{b}',
                other => other,
            }),
            '"' => panic!("an unescaped quote ends the value early: {line}"),
            c => value.push(c),
        }
    }
    value
}

#[test]
fn curl_reads_the_body_back_exactly_whatever_it_holds() {
    let body = json!({
        "title": "Quotes \" and \\ backslashes \\n not a newline",
        "message": "Two\nlines\tand ünïcödé, 漢字, an emoji 🙂 and \u{7f}",
    });
    let request = Request {
        url: "https://example.com/hook?a=1&b=2".into(),
        headers: vec![
            "Content-Type: application/json".into(),
            "Authorization: Bearer tk_x".into(),
        ],
        body: body.clone(),
    };
    let config = config(&request);
    assert!(config.is_ascii(), "{config}");
    let lines: Vec<&str> = config.lines().collect();
    assert_eq!(lines.len(), 5, "{config}");
    assert_eq!(unquote(lines[0], "url"), "https://example.com/hook?a=1&b=2");
    assert_eq!(
        unquote(lines[1], "header"),
        "Content-Type: application/json"
    );
    assert_eq!(unquote(lines[2], "header"), "Authorization: Bearer tk_x");
    assert_eq!(unquote(lines[3], "header"), "Expect:");
    let sent: serde_json::Value = serde_json::from_str(&unquote(lines[4], "data-binary")).unwrap();
    assert_eq!(sent, body);
    // Nothing but the fixed options is on the command line.
    assert!(!OPTIONS.iter().any(|option| option.contains("example.com")));
}

/// A receiver of one HTTP request on a port of its own, which answers with `status` and `answer`
/// and returns the request as it came.
#[cfg(unix)]
async fn receiver(status: u16, answer: &'static str) -> (u16, tokio::task::JoinHandle<String>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let received = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let read = socket.read(&mut buffer).await.unwrap();
            request.extend_from_slice(&buffer[..read]);
            let text = String::from_utf8_lossy(&request).into_owned();
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(str::to_owned)
                    })
                    .and_then(|length| length.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if body.len() >= length {
                    break;
                }
            }
            assert!(read > 0, "the request ended early");
        }
        socket
            .write_all(
                format!(
                    "HTTP/1.1 {status} Answer\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        String::from_utf8(request).unwrap()
    });
    (port, received)
}

#[cfg(unix)]
#[tokio::test]
async fn curl_posts_the_body_and_headers_to_the_address() {
    let (port, received) = receiver(200, "queued").await;
    let target = target(&format!(
        "ntfy+http://127.0.0.1:{port}/thoric-topic?token=tk_receiver"
    ));
    let body = json!({"topic": "thoric-topic", "message": "Größe \"quoted\"\nnext"});
    let request = Request {
        url: target.url.clone(),
        headers: vec![
            "Content-Type: application/json".into(),
            "Authorization: Bearer tk_receiver".into(),
        ],
        body: body.clone(),
    };
    let delivery = deliver(PROGRAM, &target, &request).await;
    assert_eq!(delivery.error, None);
    assert_eq!(delivery.status, Some(200));
    assert_eq!(delivery.answer, "queued");
    let received = received.await.unwrap();
    let (head, sent) = received.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("POST / HTTP/1.1\r\n"), "{head}");
    assert!(
        head.contains("\r\nContent-Type: application/json"),
        "{head}"
    );
    assert!(
        head.contains("\r\nAuthorization: Bearer tk_receiver"),
        "{head}"
    );
    assert!(!head.to_ascii_lowercase().contains("expect:"), "{head}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(sent).unwrap(),
        body
    );
}

#[cfg(unix)]
#[tokio::test]
async fn an_answer_that_refuses_is_reported_without_the_address() {
    let (port, received) = receiver(404, "no such hook: /hooks/hook-secret-part").await;
    let target = target(&format!("http://127.0.0.1:{port}/hooks/hook-secret-part"));
    let request = Request {
        url: target.url.clone(),
        headers: Vec::new(),
        body: json!({}),
    };
    let delivery = deliver(PROGRAM, &target, &request).await;
    received.await.unwrap();
    assert_eq!(delivery.status, Some(404));
    assert_eq!(
        delivery.error.as_deref(),
        Some(
            format!("JSON webhook at 127.0.0.1:{port} answered HTTP 404: no such hook: [hidden]")
                .as_str()
        )
    );
    assert!(!delivery.answer.contains("hook-secret-part"));
}

#[cfg(unix)]
#[tokio::test]
async fn an_address_that_does_not_answer_is_reported_without_it() {
    // A port nothing listens on, once the listener that had it is gone.
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let target = target(&format!("http://127.0.0.1:{port}/hooks/unreachable-secret"));
    let request = Request {
        url: target.url.clone(),
        headers: Vec::new(),
        body: json!({}),
    };
    let delivery = deliver(PROGRAM, &target, &request).await;
    assert_eq!(delivery.status, None);
    let error = delivery.error.unwrap();
    assert!(error.starts_with("curl: (7)"), "{error}");
    assert!(!error.contains("unreachable-secret"), "{error}");
}

#[test]
fn an_answer_is_cut_to_one_short_line() {
    let long = format!("first\nsecond {}", "x".repeat(1000));
    let short = short(&long);
    assert!(short.starts_with("first second x"));
    assert_eq!(short.chars().count(), ANSWER_LIMIT);
}
