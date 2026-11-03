//! Pins down the collector's lifecycle: a stream is open exactly while its container runs, a
//! recreated container is followed under its new Docker ID, a deleted container's history
//! outlives its record, under the name the record had, and `/metrics` learns each running
//! container's start and latest counters.
use super::*;

fn running(entries: &[(&str, &str)]) -> BTreeMap<String, Running> {
    entries
        .iter()
        .map(|(id, docker_id)| {
            (
                id.to_string(),
                Running {
                    docker_id: docker_id.to_string(),
                    gpu: false,
                },
            )
        })
        .collect()
}

#[test]
fn streams_follow_exactly_the_running_containers() {
    let followed = BTreeMap::from([
        ("stays".to_owned(), "1".to_owned()),
        ("stopped".to_owned(), "2".to_owned()),
        ("recreated".to_owned(), "3".to_owned()),
    ]);
    let (stop, start) = plan(
        &followed,
        &running(&[("stays", "1"), ("recreated", "4"), ("started", "5")]),
    );
    assert_eq!(stop, ["recreated", "stopped"]);
    assert_eq!(
        start,
        [
            ("recreated".to_owned(), "4".to_owned()),
            ("started".to_owned(), "5".to_owned())
        ]
    );
    let (stop, start) = plan(&BTreeMap::new(), &BTreeMap::new());
    assert!(stop.is_empty() && start.is_empty());
}

#[tokio::test]
async fn without_the_engine_socket_usage_says_why() {
    let app = crate::test_support::app(false);
    run(app.clone(), Duration::from_millis(10)).await;
    let store = app.usage.store();
    assert!(store.error.as_deref().unwrap().contains("DOCKER_HOST"));
}

#[cfg(unix)]
mod engine_socket {
    //! The whole loop against a fake Engine API on a Unix socket.
    use super::*;
    use crate::model::ContainerSpec;
    use serde_json::json;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const DOCKER_ID: &str = "4f1c5a0b9e2d3c7a8b6f5e4d3c2b1a0f9e8d7c6b5a4f3e2d1c0b9a8f7e6d5c4b";

    fn stats(second: u64) -> String {
        json!({
            "read": format!("2026-10-16T09:{:02}:{:02}Z", second / 60, second % 60),
            "cpu_stats": {"cpu_usage": {"total_usage": 500_000_000 * (second + 1)}, "system_cpu_usage": 2_000_000_000 * (second + 1), "online_cpus": 2},
            "precpu_stats": {"cpu_usage": {"total_usage": 500_000_000 * second}, "system_cpu_usage": 2_000_000_000 * second, "online_cpus": 2},
            "memory_stats": {"usage": 100 << 20, "limit": 1u64 << 30, "stats": {"inactive_file": 0}},
            "networks": {"eth0": {"rx_bytes": 1000 * second, "tx_bytes": 0}}
        })
        .to_string()
    }

    /// Answers container lists from `listed`, an inspect with the container's start, and streams
    /// stats until the client goes away.
    async fn engine(
        listener: tokio::net::UnixListener,
        listed: Arc<Mutex<Value>>,
        streams: Arc<AtomicUsize>,
    ) {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (listed, streams) = (listed.clone(), streams.clone());
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => request.extend_from_slice(&buffer[..read]),
                    }
                }
                let request = String::from_utf8(request).unwrap();
                if request.starts_with("GET /containers/json?") {
                    let body = listed.lock().unwrap().to_string();
                    let _ = socket
                        .write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes())
                        .await;
                    return;
                }
                if request.starts_with(&format!("GET /containers/{DOCKER_ID}/json ")) {
                    let body =
                        json!({"State": {"StartedAt": "2026-10-16T09:00:00.5Z"}}).to_string();
                    let _ = socket
                        .write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes())
                        .await;
                    return;
                }
                assert!(
                    request.starts_with(&format!("GET /containers/{DOCKER_ID}/stats?stream=true ")),
                    "{request}"
                );
                streams.fetch_add(1, Ordering::SeqCst);
                let mut out =
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n").await;
                let mut second = 0;
                while out.is_ok() {
                    let line = format!("{}\n", stats(second));
                    out = socket
                        .write_all(format!("{:x}\r\n{line}\r\n", line.len()).as_bytes())
                        .await;
                    second += 1;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                streams.fetch_sub(1, Ordering::SeqCst);
            });
        }
    }

    async fn until(what: &str, mut check: impl FnMut() -> bool) {
        for _ in 0..250 {
            if check() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("timed out waiting until {what}");
    }

    #[tokio::test]
    async fn a_container_is_followed_while_it_runs_and_remembered_once_deleted() {
        let dir =
            std::env::temp_dir().join(format!("thoric-usage-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("docker.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let entry = json!({"Id": DOCKER_ID, "Names": ["/thoric-a"], "State": "running", "Labels": {"thoric.managed": "true", "thoric.environment": "a"}});
        let listed = Arc::new(Mutex::new(json!([entry])));
        let streams = Arc::new(AtomicUsize::new(0));
        let server = tokio::spawn(engine(listener, listed.clone(), streams.clone()));

        let mut app = crate::test_support::app(false);
        Arc::get_mut(&mut app).unwrap().config.docker_socket = Some(socket);
        let record = crate::model::Container {
            id: "a".into(),
            container: "thoric-a".into(),
            private_network: "thoric-a-private".into(),
            egress_network: "thoric-a-egress".into(),
            source_image_id: "sha256:base".into(),
            created_at: 1,
            spec: serde_json::from_value::<ContainerSpec>(
                json!({"name": "A", "image": "thoric/ubuntu-xfce:24.04"}),
            )
            .unwrap(),
            last_error: None,
            desktop: None,
            working_image: None,
            setup: None,
        };
        app.db.put(CONTAINERS, "a", &record).unwrap();
        let collector = tokio::spawn(run(app.clone(), Duration::from_millis(50)));

        let samples = |app: &App| {
            app.usage
                .store()
                .containers
                .get("a")
                .map_or(0, |s| s.samples.len())
        };
        until("samples arrive", || samples(&app) >= 3).await;
        {
            let store = app.usage.store();
            let sample = store.containers["a"]
                .samples
                .since(0)
                .last()
                .copied()
                .unwrap();
            // Half a CPU second in each second of a two-CPU host is half a core.
            assert!((sample.cpu - 0.5).abs() < 1e-6, "{sample:?}");
            assert_eq!(sample.net_rx, 1000.0);
            assert_eq!(sample.memory, 100 << 20);
            assert_eq!(store.containers["a"].online_cpus, 2);
            assert_eq!(store.error, None);
        }
        assert_eq!(
            streams.load(Ordering::SeqCst),
            1,
            "one stream per container"
        );
        // The inspect runs beside the stream.
        until("the start is known", || {
            app.usage
                .current()
                .running
                .first()
                .is_some_and(|running| running.started.is_some())
        })
        .await;
        let current = app.usage.current();
        assert_eq!(
            current.states,
            Some(BTreeMap::from([("a".to_owned(), "running".to_owned())]))
        );
        let [running] = current.running.as_slice() else {
            panic!("{current:?}");
        };
        // 2026-10-16T09:00:00.5Z
        assert_eq!(running.started, Some(1_792_141_200.5));
        let counters = running.counters.unwrap();
        assert!(counters.cpu_time >= 500_000_000, "{counters:?}");
        assert_eq!(counters.memory_limit, 1 << 30);

        // Stopped: the stream closes, the history stays.
        *listed.lock().unwrap() = json!([]);
        until("the stream closes", || streams.load(Ordering::SeqCst) == 0).await;
        let current = app.usage.current();
        assert_eq!(current.states, Some(BTreeMap::new()));
        assert!(current.running.is_empty());
        let kept = samples(&app);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(samples(&app), kept, "no samples after a stop");
        assert!(kept >= 3);

        // Started again: followed again.
        *listed.lock().unwrap() = json!([entry]);
        until("samples resume", || samples(&app) > kept).await;

        // Deleted: the stream closes and the history stays, labeled with the record it had.
        app.db.remove(CONTAINERS, "a").unwrap();
        until("the stream closes", || streams.load(Ordering::SeqCst) == 0).await;
        let kept = samples(&app);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(samples(&app), kept, "no samples after a delete");
        assert!(kept >= 3);
        assert_eq!(
            app.usage.store().containers["a"]
                .label
                .as_ref()
                .map(|label| label.name.as_str()),
            Some("A")
        );

        collector.abort();
        server.abort();
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn a_start_time_is_read_from_the_inspect_and_never_from_go_s_zero_time() {
    use serde_json::json;
    let inspected = |at: &str| json!({"State": {"StartedAt": at}});
    assert_eq!(
        started_at(&inspected("2026-10-16T09:00:00.25Z")),
        Some(1_792_141_200.25)
    );
    assert_eq!(started_at(&inspected("0001-01-01T00:00:00Z")), None);
    assert_eq!(started_at(&json!({"State": {}})), None);
}
