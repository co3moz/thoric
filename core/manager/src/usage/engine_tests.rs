//! Pins down what the collector asks Docker and believes of its answers: every container with the
//! managed label is listed, one counts only when its name and owner both match a record, only a
//! running one is followed while each gives its record a state, and a stats stream is read line by
//! line however its chunks split.
use super::*;
use crate::model::ContainerSpec;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const DOCKER_ID: &str = "4f1c5a0b9e2d3c7a8b6f5e4d3c2b1a0f9e8d7c6b5a4f3e2d1c0b9a8f7e6d5c4b";

fn record(id: &str, gpu: bool) -> Container {
    Container {
        id: id.into(),
        container: format!("thoric-{id}"),
        private_network: format!("thoric-{id}-private"),
        egress_network: format!("thoric-{id}-egress"),
        source_image_id: "sha256:base".into(),
        created_at: 1,
        spec: serde_json::from_value::<ContainerSpec>(
            json!({"name": format!("Desktop {id}"), "image": "thoric/ubuntu-xfce:24.04", "gpu": gpu}),
        )
        .unwrap(),
        last_error: None,
        desktop: None,
        working_image: None,
        setup: None,
    }
}

fn listed(docker_id: &str, name: &str, owner: &str) -> Value {
    json!({"Id": docker_id, "Names": [format!("/{name}")], "State": "running", "Labels": {LABEL: "true", "thoric.environment": owner}})
}

#[test]
fn the_list_asks_for_every_managed_container() {
    assert_eq!(
        list_path(),
        "/containers/json?all=1&filters=%7B%22label%22:%5B%22thoric.managed%3Dtrue%22%5D%7D"
    );
    assert_eq!(
        stats_path(DOCKER_ID).as_deref(),
        Some(format!("/containers/{DOCKER_ID}/stats?stream=true").as_str())
    );
    assert_eq!(
        inspect_path(DOCKER_ID).as_deref(),
        Some(format!("/containers/{DOCKER_ID}/json").as_str())
    );
    for invalid in ["", "abc", "../../info", &DOCKER_ID.replace('4', "g")] {
        assert_eq!(stats_path(invalid), None, "{invalid}");
        assert_eq!(inspect_path(invalid), None, "{invalid}");
    }
}

#[test]
fn only_a_running_container_is_followed_and_each_gives_its_record_a_state() {
    let records = [record("a", false), record("b", false), record("c", false)];
    let mut exited = listed(&DOCKER_ID.replace('4', "5"), "thoric-b", "b");
    exited["State"] = json!("exited");
    let list = json!([listed(DOCKER_ID, "thoric-a", "a"), exited]);
    assert_eq!(running(&records, &list).keys().collect::<Vec<_>>(), ["a"]);
    // Record c has no container Docker lists, which the metrics count as missing.
    assert_eq!(
        states(&records, &list),
        BTreeMap::from([
            ("a".to_owned(), "running".to_owned()),
            ("b".to_owned(), "exited".to_owned())
        ])
    );
}

#[test]
fn a_container_counts_only_when_name_owner_and_label_match_a_record() {
    let records = [record("a", true), record("b", false)];
    let other = DOCKER_ID.replace('4', "5");
    let mut unlabeled = listed(&other, "thoric-b", "b");
    unlabeled["Labels"][LABEL] = json!("false");
    let list = json!([
        listed(DOCKER_ID, "thoric-a", "a"),
        // Claims record b's owner label under another name.
        listed(&other, "thoric-impostor", "b"),
        unlabeled,
        // Owned by a record this manager does not have.
        listed(&other, "thoric-c", "c"),
        // Docker IDs are hex; anything else never reaches a URL.
        listed("../../info", "thoric-b", "b"),
        {"Names": ["/thoric-b"]}
    ]);
    let found = running(&records, &list);
    assert_eq!(
        found,
        BTreeMap::from([(
            "a".to_owned(),
            Running {
                docker_id: DOCKER_ID.into(),
                gpu: true
            }
        )])
    );
    assert!(running(&records, &json!({"message": "not a list"})).is_empty());
}

/// Serve one HTTP response on the far end of an in-memory connection and return the request.
async fn serve(mut server: tokio::io::DuplexStream, response: Vec<u8>) -> String {
    let mut request = Vec::new();
    let mut buffer = [0; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = server.read(&mut buffer).await.unwrap();
        assert!(read > 0, "the client closed before sending its request");
        request.extend_from_slice(&buffer[..read]);
    }
    server.write_all(&response).await.unwrap();
    String::from_utf8(request).unwrap()
}

fn chunked(chunks: &[&str]) -> Vec<u8> {
    let mut response =
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n"
            .to_vec();
    for chunk in chunks {
        response.extend(format!("{:x}\r\n{chunk}\r\n", chunk.len()).as_bytes());
    }
    response.extend(b"0\r\n\r\n");
    response
}

#[tokio::test]
async fn a_stats_stream_is_read_line_by_line_across_chunks() {
    let (client, server) = tokio::io::duplex(8192);
    let server = tokio::spawn(serve(
        server,
        chunked(&[
            "{\"read\":\"a\"}\n{\"re",
            "ad\":\"b\"}\n\n",
            "{\"read\":\"c\"}",
        ]),
    ));
    let mut lines = Vec::new();
    let path = stats_path(DOCKER_ID).unwrap();
    let mut opened = false;
    stream(
        client,
        &path,
        Duration::from_secs(5),
        || opened = true,
        |line| lines.push(String::from_utf8(line.to_vec()).unwrap()),
    )
    .await
    .unwrap();
    assert!(opened, "the stream says when Docker accepted it");
    let request = server.await.unwrap();
    assert!(
        request.starts_with(&format!("GET {path} HTTP/1.1\r\n")),
        "{request}"
    );
    // The last line had no newline; the stream's end is not one.
    assert_eq!(
        lines,
        ["{\"read\":\"a\"}", "{\"read\":\"b\"}"],
        "a partial last line is dropped"
    );
}

#[tokio::test]
async fn a_silent_stream_or_a_refusal_is_an_error() {
    let (client, mut server) = tokio::io::duplex(8192);
    let holder = tokio::spawn(async move {
        let mut buffer = [0; 1024];
        let _ = server.read(&mut buffer).await;
        server
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
            .await
            .unwrap();
        // Says nothing more, and keeps the connection open.
        tokio::time::sleep(Duration::from_secs(30)).await;
    });
    let error = stream(
        client,
        "/containers/x/stats",
        Duration::from_millis(100),
        || {},
        |_| {},
    )
    .await
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("stopped sending"),
        "{error:#}"
    );
    holder.abort();

    let (client, server) = tokio::io::duplex(8192);
    let body = r#"{"message":"No such container: abc"}"#;
    tokio::spawn(serve(
        server,
        format!(
            "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes(),
    ));
    let error = get(client, "/containers/json").await.unwrap_err();
    assert_eq!(
        format!("{error:#}"),
        "Docker: No such container: abc (404 Not Found)"
    );
}

#[tokio::test]
async fn a_list_is_one_json_response() {
    let (client, server) = tokio::io::duplex(8192);
    let body = json!([listed(DOCKER_ID, "thoric-a", "a")]).to_string();
    let server = tokio::spawn(serve(
        server,
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes(),
    ));
    let value = get(client, &list_path()).await.unwrap();
    assert_eq!(value[0]["Id"], DOCKER_ID);
    assert!(
        server
            .await
            .unwrap()
            .starts_with("GET /containers/json?all=1&filters=")
    );
}
