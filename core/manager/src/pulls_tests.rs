//! Pins down how catalog image downloads are reported: Docker's per-layer messages become one byte
//! count that never runs backwards, the registry manifest supplies the total up front, and the
//! Engine API stream is read correctly however its chunks split the JSON lines.
use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn layer(id: &str, status: &str) -> Value {
    json!({"id": id, "status": status})
}
fn downloading(id: &str, current: u64, total: u64) -> Value {
    json!({"id": id, "status": "Downloading", "progressDetail": {"current": current, "total": total}})
}

#[test]
fn manifest_sizes_give_a_total_before_any_layer_downloads() {
    let mut progress = PullProgress {
        sizes: HashMap::from([("aaaaaaaaaaaa".into(), 100), ("bbbbbbbbbbbb".into(), 300)]),
        ..Default::default()
    };
    let opening = json!({"id": "24.04", "status": "Pulling from co3moz/thoric/ubuntu-xfce"});
    assert_eq!(
        progress.apply(&opening).unwrap().as_deref(),
        Some("24.04: Pulling from co3moz/thoric/ubuntu-xfce")
    );
    for message in [
        layer("cccccccccccc", "Already exists"),
        layer("aaaaaaaaaaaa", "Pulling fs layer"),
        layer("bbbbbbbbbbbb", "Pulling fs layer"),
    ] {
        assert_eq!(progress.apply(&message).unwrap(), None);
    }
    // A layer already on the host is done but is not part of what has to be downloaded.
    assert_eq!(
        progress.summary(),
        json!({"current": 0, "total": 400, "layers": 3, "layers_done": 1})
    );
    let mut seen = Vec::new();
    for message in [
        downloading("aaaaaaaaaaaa", 40, 100),
        downloading("bbbbbbbbbbbb", 120, 300),
        layer("aaaaaaaaaaaa", "Verifying Checksum"),
        layer("aaaaaaaaaaaa", "Download complete"),
        layer("aaaaaaaaaaaa", "Extracting"),
        layer("aaaaaaaaaaaa", "Pull complete"),
        downloading("bbbbbbbbbbbb", 300, 300),
        layer("bbbbbbbbbbbb", "Download complete"),
    ] {
        progress.apply(&message).unwrap();
        seen.push(progress.summary()["current"].as_u64().unwrap());
    }
    // A full bar waits for the layer's checksum, since Docker prints one for a broken attempt too.
    assert_eq!(seen, [40, 160, 220, 220, 220, 220, 220, 400]);
    assert_eq!(progress.summary()["layers_done"], 2);
    progress
        .apply(&layer("bbbbbbbbbbbb", "Pull complete"))
        .unwrap();
    assert_eq!(
        progress.summary(),
        json!({"current": 400, "total": 400, "layers": 3, "layers_done": 3})
    );
    assert_eq!(
        progress
            .apply(&json!({"status": "Status: Downloaded newer image for ghcr.io/co3moz/thoric/ubuntu-xfce:24.04"}))
            .unwrap()
            .as_deref(),
        Some("Status: Downloaded newer image for ghcr.io/co3moz/thoric/ubuntu-xfce:24.04")
    );
}

#[test]
fn without_a_manifest_the_total_stays_unknown_until_every_layer_reports_its_size() {
    let mut progress = PullProgress::default();
    progress
        .apply(&layer("aaaaaaaaaaaa", "Pulling fs layer"))
        .unwrap();
    progress.apply(&layer("bbbbbbbbbbbb", "Waiting")).unwrap();
    progress
        .apply(&downloading("aaaaaaaaaaaa", 10, 100))
        .unwrap();
    assert_eq!(progress.summary()["total"], Value::Null);
    assert_eq!(progress.summary()["current"], 10);
    progress.apply(&downloading("bbbbbbbbbbbb", 5, 50)).unwrap();
    assert_eq!(progress.summary()["total"], 150);
    assert_eq!(progress.summary()["current"], 15);
}

/// The messages Docker 28's classic image store sent when the registry connection of a 312 MB
/// layer broke during an acceptance run: the broken attempt closes with a full bar, Docker waits,
/// and the resumed attempt counts from zero against the bytes it still lacks.
#[test]
fn a_resumed_layer_download_keeps_counting_from_where_it_stopped() {
    let manifest = HashMap::from([("aaaaaaaaaaaa".into(), 1_000), ("bbbbbbbbbbbb".into(), 100)]);
    // Without the manifest, the size comes from the first attempt's count.
    for sizes in [manifest, HashMap::new()] {
        let mut progress = PullProgress {
            sizes,
            ..Default::default()
        };
        let mut seen = Vec::new();
        for message in [
            layer("aaaaaaaaaaaa", "Pulling fs layer"),
            layer("bbbbbbbbbbbb", "Pulling fs layer"),
            downloading("aaaaaaaaaaaa", 400, 1_000),
            downloading("bbbbbbbbbbbb", 100, 100),
            layer("bbbbbbbbbbbb", "Verifying Checksum"),
            downloading("aaaaaaaaaaaa", 600, 1_000),
            // The connection broke at 600 bytes.
            downloading("aaaaaaaaaaaa", 1_000, 1_000),
            layer("aaaaaaaaaaaa", "Retrying in 5 seconds"),
            downloading("aaaaaaaaaaaa", 50, 400),
            downloading("aaaaaaaaaaaa", 300, 400),
            downloading("aaaaaaaaaaaa", 400, 400),
            layer("aaaaaaaaaaaa", "Verifying Checksum"),
            layer("aaaaaaaaaaaa", "Download complete"),
            json!({"id": "aaaaaaaaaaaa", "status": "Extracting", "progressDetail": {"current": 32_768, "total": 2_000}}),
            layer("aaaaaaaaaaaa", "Pull complete"),
        ] {
            progress.apply(&message).unwrap();
            seen.push(progress.summary()["current"].as_u64().unwrap());
        }
        assert_eq!(
            seen,
            [
                0, 0, 400, 400, 500, 700, 700, 700, 750, 1_000, 1_000, 1_100, 1_100, 1_100, 1_100
            ]
        );
        assert_eq!(progress.summary()["total"], 1_100);
    }
}

/// A download that fails is tried again with the same count. Docker starts the layers the failed
/// attempt left unfinished over, and names those it finished as already present, which keeps
/// them apart from a layer the host had before.
#[test]
fn another_attempt_keeps_what_the_failed_one_downloaded() {
    let mut progress = PullProgress {
        sizes: HashMap::from([("aaaaaaaaaaaa".into(), 1_000), ("bbbbbbbbbbbb".into(), 100)]),
        ..Default::default()
    };
    let mut seen = Vec::new();
    for message in [
        layer("cccccccccccc", "Already exists"),
        layer("aaaaaaaaaaaa", "Pulling fs layer"),
        layer("bbbbbbbbbbbb", "Pulling fs layer"),
        downloading("aaaaaaaaaaaa", 300, 1_000),
        layer("bbbbbbbbbbbb", "Download complete"),
        layer("bbbbbbbbbbbb", "Pull complete"),
        // The next attempt.
        json!({"id": "24.04", "status": "Pulling from co3moz/thoric/ubuntu-xfce"}),
        layer("cccccccccccc", "Already exists"),
        layer("bbbbbbbbbbbb", "Already exists"),
        layer("aaaaaaaaaaaa", "Pulling fs layer"),
        downloading("aaaaaaaaaaaa", 100, 1_000),
        downloading("aaaaaaaaaaaa", 500, 1_000),
    ] {
        progress.apply(&message).unwrap();
        let summary = progress.summary();
        seen.push(summary["current"].as_u64().unwrap());
        if summary["layers"] == 3 {
            assert_eq!(summary["total"], 1_100, "{summary}");
        }
    }
    assert_eq!(seen, [0, 0, 0, 300, 400, 400, 400, 400, 400, 400, 400, 600]);
    assert_eq!(
        progress.summary(),
        json!({"current": 600, "total": 1_100, "layers": 3, "layers_done": 2})
    );
}

/// The shapes Docker 29 sends with the containerd image store, as captured from a real pull:
/// small layers finish without any byte count, extraction reports seconds rather than bytes, and
/// blobs that are not image layers report downloads without ever being announced.
#[test]
fn containerd_store_pulls_count_only_announced_layers() {
    let hidden = |id: &str, status: &str| json!({"id": id, "status": status, "progressDetail": {"hidecounts": true}});
    let mut progress = PullProgress {
        sizes: HashMap::from([
            ("aaaaaaaaaaaa".into(), 2_000),
            ("bbbbbbbbbbbb".into(), 50_000),
        ]),
        ..Default::default()
    };
    for message in [
        layer("aaaaaaaaaaaa", "Pulling fs layer"),
        layer("bbbbbbbbbbbb", "Pulling fs layer"),
        hidden("aaaaaaaaaaaa", "Download complete"),
        downloading("bbbbbbbbbbbb", 10_000, 50_000),
        // An attestation blob: downloaded, never announced and never pulled.
        downloading("cccccccccccc", 1_000, 9_000),
        hidden("cccccccccccc", "Download complete"),
        json!({"id": "aaaaaaaaaaaa", "status": "Extracting", "progressDetail": {"current": 1, "units": "s"}}),
        hidden("aaaaaaaaaaaa", "Pull complete"),
    ] {
        progress.apply(&message).unwrap();
    }
    assert_eq!(
        progress.summary(),
        json!({"current": 12_000, "total": 52_000, "layers": 2, "layers_done": 1})
    );
    progress
        .apply(&hidden("bbbbbbbbbbbb", "Download complete"))
        .unwrap();
    progress
        .apply(&hidden("bbbbbbbbbbbb", "Pull complete"))
        .unwrap();
    assert_eq!(
        progress.summary(),
        json!({"current": 52_000, "total": 52_000, "layers": 2, "layers_done": 2})
    );
}

#[test]
fn a_pull_error_message_fails_with_dockers_explanation() {
    let mut progress = PullProgress::default();
    let error = progress
        .apply(&json!({"errorDetail": {"message": "toomanyrequests: retry later"}, "error": "toomanyrequests: retry later"}))
        .unwrap_err();
    assert_eq!(error.to_string(), "Docker: toomanyrequests: retry later");
    assert!(
        progress
            .apply(&json!({"error": "unauthorized"}))
            .unwrap_err()
            .to_string()
            .contains("unauthorized")
    );
}

#[test]
fn manifest_sizes_pick_the_host_architecture_and_ignore_attestations() {
    let layers = json!({"layers": [
        {"digest": "sha256:aaaaaaaaaaaa0000", "size": 100},
        {"digest": "sha256:bbbbbbbbbbbb1111", "size": 300},
    ]});
    let arm = json!({"layers": [{"digest": "sha256:dddddddddddd3333", "size": 70}]});
    let index = json!([
        {"Descriptor": {"platform": {"os": "linux", "architecture": "amd64"}}, "OCIManifest": layers},
        {"Descriptor": {"platform": {"os": "linux", "architecture": "arm64", "variant": "v8"}}, "OCIManifest": arm},
        {"Descriptor": {"platform": {"os": "unknown", "architecture": "unknown"}}, "OCIManifest": {"layers": [{"digest": "sha256:cccccccccccc2222", "size": 9}]}},
    ]);
    let expected = HashMap::from([("aaaaaaaaaaaa".into(), 100), ("bbbbbbbbbbbb".into(), 300)]);
    assert_eq!(manifest_sizes(&index, "amd64"), expected);
    assert_eq!(
        manifest_sizes(&index, "arm64"),
        HashMap::from([("dddddddddddd".into(), 70)])
    );
    // An image without the host's architecture has no sizes to show.
    assert!(manifest_sizes(&index, "riscv64").is_empty());
    // A single-platform image is printed as one entry, and older images use the Docker schema.
    assert_eq!(
        manifest_sizes(&json!({"SchemaV2Manifest": layers}), "amd64"),
        expected
    );
    // Two candidates would be a guess; the stream's own totals are used instead.
    let ambiguous = json!([
        {"Descriptor": {"platform": {"os": "linux", "architecture": "amd64"}}, "OCIManifest": layers},
        {"Descriptor": {"platform": {"os": "linux", "architecture": "amd64"}}, "OCIManifest": layers},
    ]);
    assert!(manifest_sizes(&ambiguous, "amd64").is_empty());
    assert!(
        manifest_sizes(
            &json!({"OCIManifest": {"layers": [{"digest": "sha256:short", "size": 1}]}}),
            "amd64"
        )
        .is_empty()
    );
    assert!(manifest_sizes(&json!("not a manifest"), "amd64").is_empty());
}

#[test]
fn the_image_architecture_follows_docker_and_falls_back_to_the_manager() {
    let info = |machine: &str| json!({"Architecture": machine});
    assert_eq!(image_architecture(Some(&info("x86_64"))), "amd64");
    // A Raspberry Pi's kernel and the virtual machine of a Mac's Docker Desktop.
    assert_eq!(image_architecture(Some(&info("aarch64"))), "arm64");
    assert_eq!(image_architecture(Some(&info("armv7l"))), "arm");
    assert_eq!(image_architecture(Some(&info("riscv64"))), "riscv64");
    let own = image_architecture(None);
    assert_eq!(image_architecture(Some(&info(""))), own);
    assert_eq!(image_architecture(Some(&json!({}))), own);
    #[cfg(target_arch = "x86_64")]
    assert_eq!(own, "amd64");
    #[cfg(target_arch = "aarch64")]
    assert_eq!(own, "arm64");
}

#[test]
fn the_engine_socket_follows_the_clis_docker_host() {
    use crate::config::docker_socket;
    assert_eq!(
        docker_socket(None),
        Some(std::path::PathBuf::from("/var/run/docker.sock"))
    );
    assert_eq!(docker_socket(Some("")), docker_socket(None));
    assert_eq!(
        docker_socket(Some("unix:///run/user/1000/docker.sock")),
        Some(std::path::PathBuf::from("/run/user/1000/docker.sock"))
    );
    // A daemon the socket cannot reach is pulled through the CLI instead.
    assert_eq!(docker_socket(Some("tcp://10.0.0.2:2376")), None);
}

#[test]
fn image_references_are_encoded_for_the_query() {
    assert_eq!(
        query_value("ghcr.io/co3moz/thoric/ubuntu-xfce:24.04"),
        "ghcr.io/co3moz/thoric/ubuntu-xfce:24.04"
    );
    assert_eq!(query_value("a+b c&d"), "a%2Bb%20c%26d");
}

/// Serve one HTTP response on the far end of an in-memory connection and return the request.
async fn serve(mut server: tokio::io::DuplexStream, response: String) -> String {
    let mut request = Vec::new();
    let mut buffer = [0; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = server.read(&mut buffer).await.unwrap();
        assert!(read > 0, "the client closed before sending its request");
        request.extend_from_slice(&buffer[..read]);
    }
    server.write_all(response.as_bytes()).await.unwrap();
    String::from_utf8(request).unwrap()
}

#[tokio::test]
async fn engine_pull_reads_messages_split_across_chunks() {
    let (client, server) = tokio::io::duplex(8192);
    let first = "{\"status\":\"Pulling from co3moz/thoric/ubuntu-xfce\",\"id\":\"24.04\"}\r\n{\"status\":\"Downl";
    let second = "oading\",\"id\":\"aaaaaaaaaaaa\",\"progressDetail\":{\"current\":5,\"total\":10}}\r\n{\"status\":\"Status: Image is up to date\"}";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{first}\r\n{:x}\r\n{second}\r\n0\r\n\r\n",
        first.len(),
        second.len()
    );
    let server = tokio::spawn(serve(server, response));
    let mut messages = Vec::new();
    engine_pull(
        client,
        "ghcr.io/co3moz/thoric/ubuntu-xfce:24.04",
        |message| {
            messages.push(message);
            Ok(())
        },
    )
    .await
    .unwrap();
    let request = server.await.unwrap();
    assert!(
        request.starts_with(
            "POST /images/create?fromImage=ghcr.io/co3moz/thoric/ubuntu-xfce:24.04 HTTP/1.1\r\n"
        ),
        "{request}"
    );
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[1]["progressDetail"]["current"], 5);
    // The last message has no trailing newline.
    assert_eq!(messages[2]["status"], "Status: Image is up to date");
}

#[tokio::test]
async fn engine_pull_reports_refusals_and_interrupted_streams() {
    // A refusal is final, while a registry the daemon could not reach in time is tried again.
    for (status, message, refused) in [
        ("404 Not Found", "manifest unknown", true),
        (
            "500 Internal Server Error",
            "net/http: TLS handshake timeout",
            false,
        ),
    ] {
        let (client, server) = tokio::io::duplex(8192);
        let body = format!("{{\"message\":\"{message}\"}}");
        let server = tokio::spawn(serve(
            server,
            format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
        ));
        let error = engine_pull(client, "ghcr.io/example/missing:1", |_| Ok(()))
            .await
            .unwrap_err();
        server.await.unwrap();
        let text = error.to_string();
        assert!(
            text.contains(message) && text.contains(&status[..3]),
            "{text}"
        );
        assert_eq!(error.downcast_ref::<Refused>().is_some(), refused, "{text}");
    }

    // The daemon going away mid-download must fail the job, not end it as if complete.
    let (client, server) = tokio::io::duplex(8192);
    let server = tokio::spawn(serve(
        server,
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n10\r\n{\"status\":\"x\"}\r\n\r\n"
            .into(),
    ));
    let mut messages = 0;
    let result = engine_pull(client, "ghcr.io/example/image:1", |_| {
        messages += 1;
        Ok(())
    })
    .await;
    server.await.unwrap();
    assert!(result.is_err(), "a truncated stream was accepted");
    assert_eq!(messages, 1);

    // A message the callback rejects stops the pull with that error.
    let (client, server) = tokio::io::duplex(8192);
    let line = "{\"error\":\"denied\"}\n";
    let server = tokio::spawn(serve(
        server,
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{line}",
            line.len()
        ),
    ));
    let mut progress = PullProgress::default();
    let error = engine_pull(client, "ghcr.io/example/image:1", |message| {
        progress.apply(&message).map(drop)
    })
    .await
    .unwrap_err();
    server.await.unwrap();
    assert_eq!(error.to_string(), "Docker: denied");
}

#[cfg(unix)]
#[tokio::test]
async fn a_release_tags_the_desktop_published_with_it_as_the_catalog_reference() {
    let image = crate::catalog::catalog().remove(0);
    let reference = image.reference.clone();
    let release = format!("{}:1.2.3", reference.rsplit_once(':').unwrap().0);
    // A Docker CLI stand-in that records its arguments and has no manifest to offer. It lists
    // every image except the release tag, which it lists only once `release-tag-here` exists.
    let dir = std::env::temp_dir().join(format!("thoric-pull-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let calls = dir.join("calls");
    let docker = dir.join("docker");
    crate::test_support::write_script(
        &docker,
        &format!(
            "#!/bin/sh\ncd '{dir}'\necho \"$*\" >> calls\ncase \"$*\" in\n  'manifest inspect'*) exit 1 ;;\n  'image inspect {release}') [ -e release-tag-here ] || exit 1 ;;\nesac\ncase \"$1 $2\" in\n  'image inspect') echo '[{{\"Id\":\"sha256:feed\"}}]' ;;\nesac\n",
            dir = dir.display()
        ),
    );
    let mut app = crate::test_support::app(false);
    std::sync::Arc::get_mut(&mut app).unwrap().config.docker_bin = docker.display().to_string();
    let job = crate::model::Job {
        id: "pull".into(),
        kind: "image-pull".into(),
        resource: None,
        status: "running".into(),
        created_at: crate::model::now(),
        finished_at: None,
        log: String::new(),
        result: None,
        progress: None,
        stages: Vec::new(),
        script: None,
        name: None,
    };
    app.db.put("job", "pull", &job).unwrap();

    let result = app
        .pull_catalog_image_from(&image, &release, "pull")
        .await
        .unwrap();
    assert_eq!(result["reference"], reference);
    assert_eq!(result["image_id"], "sha256:feed");
    assert_eq!(
        std::fs::read_to_string(&calls)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        [
            format!("image inspect {reference}"),
            format!("image inspect {release}"),
            format!("image ls --format {{{{.Repository}}}}:{{{{.Tag}}}} {release}"),
            format!("manifest inspect --verbose {release}"),
            format!("pull {release}"),
            format!("tag {release} {reference}"),
            format!("image rm {release}"),
            format!("image inspect {reference}"),
        ]
    );

    // A release tag that was here before the download stays.
    std::fs::remove_file(&calls).unwrap();
    std::fs::write(dir.join("release-tag-here"), "").unwrap();
    app.pull_catalog_image_from(&image, &release, "pull")
        .await
        .unwrap();
    let after = std::fs::read_to_string(&calls).unwrap();
    assert!(
        after.contains(&format!("tag {release} {reference}")),
        "{after}"
    );
    assert!(!after.contains("image rm"), "{after}");

    // Any other build pulls the catalog reference itself and tags nothing.
    std::fs::remove_file(&calls).unwrap();
    app.pull_catalog_image_from(&image, &reference, "pull")
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&calls)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        [
            format!("image inspect {reference}"),
            format!("manifest inspect --verbose {reference}"),
            format!("pull {reference}"),
            format!("image inspect {reference}"),
        ]
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_download_removes_the_image_it_replaced_when_nothing_needs_it() {
    let image = crate::catalog::catalog().remove(0);
    let reference = image.reference.clone();
    // The reference names sha256:old until a pull moves it to sha256:new; nothing else tags old.
    let dir =
        std::env::temp_dir().join(format!("thoric-replaced-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let calls = dir.join("calls");
    let docker = dir.join("docker");
    crate::test_support::write_script(
        &docker,
        &format!(
            "#!/bin/sh\ncd '{dir}'\necho \"$*\" >> calls\ncase \"$*\" in\n  'manifest inspect'*) exit 1 ;;\n  'pull '*) touch pulled ;;\n  'image inspect {reference}') if [ -e pulled ]; then echo '[{{\"Id\":\"sha256:new\"}}]'; else echo '[{{\"Id\":\"sha256:old\"}}]'; fi ;;\n  'image inspect sha256:old') [ -e removed ] && exit 1; echo '[{{\"Id\":\"sha256:old\",\"RepoTags\":[]}}]' ;;\n  'image rm sha256:old') touch removed; echo 'Deleted: sha256:old' ;;\n  *) exit 1 ;;\nesac\n",
            dir = dir.display()
        ),
    );
    let mut app = crate::test_support::app(false);
    std::sync::Arc::get_mut(&mut app).unwrap().config.docker_bin = docker.display().to_string();
    let job = crate::model::Job {
        id: "pull".into(),
        kind: "image-pull".into(),
        resource: None,
        status: "running".into(),
        created_at: crate::model::now(),
        finished_at: None,
        log: String::new(),
        result: None,
        progress: None,
        stages: Vec::new(),
        script: None,
        name: None,
    };
    app.db.put("job", "pull", &job).unwrap();

    let result = app
        .pull_catalog_image_from(&image, &reference, "pull")
        .await
        .unwrap();
    assert_eq!(result["image_id"], "sha256:new");
    let made = std::fs::read_to_string(&calls).unwrap();
    assert!(
        made.lines().any(|call| call == "image rm sha256:old"),
        "{made}"
    );
    let log = app.db.job("pull").unwrap().log;
    assert!(
        log.contains(&format!("Removed the image {reference} named before, old.")),
        "{log}"
    );

    // A download that leaves the reference where it was removes nothing.
    std::fs::remove_file(&calls).unwrap();
    app.pull_catalog_image_from(&image, &reference, "pull")
        .await
        .unwrap();
    let made = std::fs::read_to_string(&calls).unwrap();
    assert!(!made.contains("image rm"), "{made}");
    std::fs::remove_dir_all(&dir).unwrap();
}
