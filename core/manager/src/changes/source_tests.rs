//! Reading Docker's change list: the Engine API's JSON with its numbered kinds, the CLI's lines
//! with a name that holds a line break, and the Engine API's refusal and a list too long to read.
use super::*;

#[test]
fn the_engine_apis_kinds_are_numbered_and_an_empty_list_is_null() {
    let entries = engine_entries(
        br#"[{"Path":"/etc","Kind":0},{"Path":"/etc/new","Kind":1},{"Path":"/etc/legal","Kind":2},{"Path":"/home/desktop/two\nlines","Kind":1}]"#,
    )
    .unwrap();
    assert_eq!(
        entries,
        [
            Change {
                path: "/etc".into(),
                kind: Kind::Changed
            },
            Change {
                path: "/etc/new".into(),
                kind: Kind::Added
            },
            Change {
                path: "/etc/legal".into(),
                kind: Kind::Deleted
            },
            Change {
                path: "/home/desktop/two\nlines".into(),
                kind: Kind::Added
            },
        ]
    );
    assert!(engine_entries(b"null").unwrap().is_empty());
    assert!(engine_entries(b"[]").unwrap().is_empty());
    assert!(engine_entries(br#"[{"Path":"/x","Kind":7}]"#).is_err());
    assert!(engine_entries(b"<html>").is_err());
}

#[test]
fn a_cli_line_that_does_not_start_an_entry_continues_the_name_before_it() {
    let text = "C /home\nA /home/desktop/two\nlines\nA /home/desktop/sp ace\nD /etc/legal\nA /home/desktop/end\n\n";
    let entries = cli_entries(text);
    let paths: Vec<(&str, Kind)> = entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry.kind))
        .collect();
    assert_eq!(
        paths,
        [
            ("/home", Kind::Changed),
            ("/home/desktop/two\nlines", Kind::Added),
            ("/home/desktop/sp ace", Kind::Added),
            ("/etc/legal", Kind::Deleted),
            // Only the newline that ends the output goes; a name may end in one too.
            ("/home/desktop/end\n", Kind::Added),
        ]
    );
    assert!(cli_entries("").is_empty());
    assert!(cli_entries("\n").is_empty());
}

/// An Engine API that answers one request with `status` and `body`.
#[cfg(unix)]
async fn engine_answering(status: u16, body: Vec<u8>) -> tokio::io::DuplexStream {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (client, mut server) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let mut request = vec![0; 4096];
        let read = server.read(&mut request).await.unwrap();
        let head = String::from_utf8_lossy(&request[..read]).to_string();
        assert!(
            head.starts_with("GET /containers/abc123/changes "),
            "{head}"
        );
        let answer = format!(
            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        server.write_all(answer.as_bytes()).await.unwrap();
        let _ = server.write_all(&body).await;
        let _ = server.flush().await;
        // The connection stays open until the client has read the answer.
        let _ = server.read(&mut request).await;
    });
    client
}

#[cfg(unix)]
#[tokio::test]
async fn the_engine_api_is_asked_for_the_container_and_its_refusal_named() {
    let stream = engine_answering(200, br#"[{"Path":"/root","Kind":0}]"#.to_vec()).await;
    let entries = engine(stream, "abc123").await.unwrap();
    assert_eq!(entries[0].path, "/root");
    let stream =
        engine_answering(404, br#"{"message":"No such container: abc123"}"#.to_vec()).await;
    let error = format!("{:#}", engine(stream, "abc123").await.unwrap_err());
    assert!(error.contains("No such container: abc123"), "{error}");
    // Docker's own failure is the gateway's, as the CLI's is.
    let stream = engine_answering(500, br#"{"message":"walk failed"}"#.to_vec()).await;
    let error = crate::error::Error::from(engine(stream, "abc123").await.unwrap_err());
    assert_eq!(error.0, axum::http::StatusCode::BAD_GATEWAY);
}

#[cfg(unix)]
#[tokio::test]
async fn a_list_beyond_the_limit_is_refused_in_so_many_words() {
    let mut body = b"[".to_vec();
    let entry = br#"{"Path":"/home/desktop/node_modules/some-package/lib/index.js","Kind":1},"#;
    while body.len() <= LIST_LIMIT {
        body.extend_from_slice(entry);
    }
    let stream = engine_answering(200, body).await;
    let error = crate::error::Error::from(engine(stream, "abc123").await.unwrap_err());
    assert_eq!(error.0, axum::http::StatusCode::BAD_GATEWAY);
    assert!(error.1.contains("larger than 32 MiB"), "{}", error.1);
}
