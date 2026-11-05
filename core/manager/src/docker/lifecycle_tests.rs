//! Pins down what a container's creation, retry and reset refuse before Docker changes anything:
//! the manager's own volume, also through a compose, a volume `VOLUME_FILTER` leaves out and a
//! feature the installation switched off, which a container that has it keeps; and that a missing
//! tun device is named by the VPN setting. Also that a desktop that is not ready leaves its error on
//! the container only when the compose setup it kept from running was due.
use super::*;
#[cfg(unix)]
use crate::test_support::manager_docker;
use crate::test_support::{app, spec};
use crate::{config, gamepad, router};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use std::sync::Arc;
use tower::ServiceExt;

#[cfg(unix)]
#[tokio::test]
async fn no_desktop_is_created_with_the_managers_own_volume() {
    let mut app = app(false);
    let calls = manager_docker(&mut app);
    let mut spec = spec();
    spec.mounts.push(Mount {
        volume: "thoric-data".into(),
        target: "/workspace".into(),
        readonly: true,
    });
    let refused = |error: anyhow::Error| {
        let message = error.to_string();
        assert!(
            message.contains("thoric-data is the manager's own volume"),
            "{message}"
        );
    };
    // Creation refuses it before the volume or the image is looked at, and saves nothing.
    refused(
        app.create_container("refused", spec.clone(), Default::default(), "create-job")
            .await
            .unwrap_err(),
    );
    assert!(app.db.list::<Container>(CONTAINERS).unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(&calls).unwrap(),
        "container inspect thoric-manager\n"
    );

    // A record saved before the manager refused it: a reset keeps its container, and a retry
    // does not recreate the missing one.
    let env = Container {
        id: "legacy".into(),
        container: "thoric-legacy".into(),
        private_network: "thoric-legacy-private".into(),
        egress_network: "thoric-legacy-egress".into(),
        source_image_id: "sha256:source".into(),
        created_at: 0,
        spec,
        last_error: None,
        desktop: None,
        working_image: None,
        setup: None,
    };
    app.db.put(CONTAINERS, "legacy", &env).unwrap();
    std::fs::remove_file(&calls).unwrap();
    refused(
        app.reset_container("legacy", "reset-job")
            .await
            .unwrap_err(),
    );
    assert_eq!(
        std::fs::read_to_string(&calls).unwrap(),
        "container inspect thoric-manager\n"
    );
    std::fs::remove_file(&calls).unwrap();
    refused(
        app.retry_container("legacy", "retry-job")
            .await
            .unwrap_err(),
    );
    let made = std::fs::read_to_string(&calls).unwrap();
    assert_eq!(
        made.lines().last(),
        Some("container inspect thoric-manager"),
        "{made}"
    );
    assert!(
        !made.contains("network") && !made.contains("create"),
        "{made}"
    );
    let saved: Container = app.db.get(CONTAINERS, "legacy").unwrap();
    assert!(saved.last_error.unwrap().contains("manager's own volume"));
    std::fs::remove_dir_all(calls.parent().unwrap()).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_compose_volume_parameter_cannot_bring_back_the_managers_own_volume() {
    let mut app = app(false);
    let calls = manager_docker(&mut app);
    // A container whose compose volume parameter named the data volume before creation refused
    // it: neither a reset nor a retry builds a desktop with it again.
    let setup: crate::compose::Setup = serde_json::from_value(serde_json::json!({
        "status": "completed", "image": "img", "parameters": {"DATA": "/opt/data"},
        "secrets": [],
        "mounts": [{"volume": "thoric-data", "target": "/opt/data", "readonly": false}],
    }))
    .unwrap();
    let env = Container {
        id: "composed".into(),
        container: "thoric-composed".into(),
        private_network: "thoric-composed-private".into(),
        egress_network: "thoric-composed-egress".into(),
        source_image_id: "sha256:source".into(),
        created_at: 0,
        spec: spec(),
        last_error: None,
        desktop: None,
        working_image: None,
        setup: Some(setup),
    };
    app.db.put(CONTAINERS, "composed", &env).unwrap();
    let refused = |error: anyhow::Error| {
        let message = error.to_string();
        assert!(
            message.contains("thoric-data is the manager's own volume"),
            "{message}"
        );
    };
    refused(
        app.reset_container("composed", "reset-job")
            .await
            .unwrap_err(),
    );
    // Refused before the old container is removed.
    assert_eq!(
        std::fs::read_to_string(&calls).unwrap(),
        "container inspect thoric-manager\n"
    );
    std::fs::remove_file(&calls).unwrap();
    refused(
        app.retry_container("composed", "retry-job")
            .await
            .unwrap_err(),
    );
    let made = std::fs::read_to_string(&calls).unwrap();
    assert_eq!(
        made.lines().last(),
        Some("container inspect thoric-manager"),
        "{made}"
    );
    assert!(
        !made.contains("network") && !made.contains("create"),
        "{made}"
    );
    std::fs::remove_dir_all(calls.parent().unwrap()).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_desktop_cannot_be_created_with_a_volume_the_filter_leaves_out() {
    let mut app = app(false);
    let calls = manager_docker(&mut app);
    Arc::get_mut(&mut app).unwrap().config.volume_filter = config::volume_filter(None).unwrap();
    let mut spec = spec();
    spec.mounts.push(Mount {
        volume: "outside".into(),
        target: "/workspace".into(),
        readonly: false,
    });
    let error = app
        .create_container("outside", spec, Default::default(), "create-job")
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Volume not found");
    // Refused before the image is looked at or downloaded, and nothing is saved.
    assert_eq!(
        std::fs::read_to_string(&calls).unwrap(),
        "container inspect thoric-manager\nvolume inspect outside\n"
    );
    assert!(app.db.list::<Container>(CONTAINERS).unwrap().is_empty());
    std::fs::remove_dir_all(calls.parent().unwrap()).unwrap();
}

#[test]
fn a_host_without_the_tun_device_names_the_vpn_setting() {
    let app = app(false);
    let mut env = Container {
        id: "vpn-id".into(),
        container: "thoric-vpn-id".into(),
        private_network: "thoric-vpn-id-private".into(),
        egress_network: "thoric-vpn-id-egress".into(),
        source_image_id: "sha256:source".into(),
        created_at: 0,
        spec: spec(),
        last_error: None,
        desktop: None,
        working_image: None,
        setup: None,
    };
    let docker = || {
        anyhow::anyhow!(
            "error gathering device information while adding custom device \"/dev/net/tun\": no such file or directory"
        )
    };
    // Without VPN access the device is not the container's, and Docker's words stay as they are.
    assert!(!format!("{:#}", app.device_error(&env, docker())).contains("vpn"));
    env.spec.vpn = true;
    let error = format!("{:#}", app.device_error(&env, docker()));
    assert!(
        error.starts_with("VPN access needs /dev/net/tun"),
        "{error}"
    );
    assert!(error.contains("without vpn"), "{error}");
}

#[tokio::test]
async fn switched_off_features_are_refused_and_a_container_that_has_them_keeps_them() {
    let mut app = app(false);
    {
        let config = &mut Arc::get_mut(&mut app).unwrap().config;
        config.internet = false;
        config.sudo = false;
        config.gamepad = false;
        config.vpn = false;
        config.debugger = false;
        config.realtime = false;
        config.profiler = false;
        config.kvm = false;
    }
    let without = || {
        let mut spec = spec();
        spec.internet = false;
        spec.sudo = false;
        spec.gamepad = false;
        spec.vpn = false;
        spec
    };
    // A new container cannot have them.
    for (feature, says) in [
        ("internet", "INTERNET_ENABLED"),
        ("sudo", "SUDO_ENABLED"),
        ("gamepad", "GAMEPAD_ENABLED"),
        ("vpn", "VPN_ENABLED"),
        ("debugger", "DEBUGGER_ENABLED"),
        ("realtime", "REALTIME_ENABLED"),
        ("profiler", "PROFILER_ENABLED"),
        ("kvm", "KVM_ENABLED"),
    ] {
        let mut spec = without();
        match feature {
            "internet" => spec.internet = true,
            "sudo" => spec.sudo = true,
            "gamepad" => spec.gamepad = true,
            "vpn" => spec.vpn = true,
            "debugger" => spec.debugger = true,
            "realtime" => spec.realtime = true,
            "profiler" => spec.profiler = true,
            _ => spec.kvm = true,
        }
        let error = app
            .create_container("refused", spec.clone(), Default::default(), "create-job")
            .await
            .unwrap_err();
        assert!(error.to_string().contains(says), "{error}");
        // Nor can a settings change turn one on.
        let env = Container {
            id: "off".into(),
            container: "thoric-off".into(),
            private_network: "thoric-off-private".into(),
            egress_network: "thoric-off-egress".into(),
            source_image_id: "sha256:source".into(),
            created_at: 0,
            spec: without(),
            last_error: None,
            desktop: None,
            working_image: None,
            setup: None,
        };
        let request = serde_json::json!({ feature: true });
        let error = app
            .settings_preview(&env, request.as_object().unwrap())
            .unwrap_err();
        assert!(error.to_string().contains(says), "{error}");
    }
    // A container created before the switch keeps them all, also when it is created again.
    let mut kept = spec();
    kept.gamepad = true;
    kept.vpn = true;
    kept.debugger = true;
    kept.realtime = true;
    kept.profiler = true;
    kept.kvm = true;
    let env = Container {
        id: "kept".into(),
        container: "thoric-kept".into(),
        private_network: "thoric-kept-private".into(),
        egress_network: "thoric-kept-egress".into(),
        source_image_id: "sha256:source".into(),
        created_at: 0,
        spec: kept,
        last_error: None,
        desktop: None,
        working_image: None,
        setup: None,
    };
    let (_, changes) = app
        .settings_preview(
            &env,
            serde_json::json!({"name": "Renamed"}).as_object().unwrap(),
        )
        .unwrap();
    assert_eq!(changes.live, ["name"]);
    let args = app.container_args(&env, None);
    assert!(args.iter().any(|arg| arg == "/dev/uinput"), "{args:?}");
    assert!(args.iter().any(|arg| arg == "NET_ADMIN"), "{args:?}");
    for kept in ["SYS_PTRACE", "PERFMON", "rtprio=95", "/dev/kvm"] {
        assert!(
            args.iter().any(|arg| arg == kept),
            "{kept} missing from {args:?}"
        );
    }
    assert!(
        args.iter().any(|arg| arg == "DESKTOP_SUDO=true"),
        "{args:?}"
    );
    // What the manager carries itself stops while the feature is off.
    app.db.put(CONTAINERS, &env.id, &env).unwrap();
    let input: gamepad::GamepadInput =
        serde_json::from_value(serde_json::json!({"actions": [{"type": "reset"}]})).unwrap();
    let error = gamepad::gamepad_input(&app, "kept", &input)
        .await
        .unwrap_err();
    assert!(error.1.contains("GAMEPAD_ENABLED"), "{}", error.1);
    // Internet cannot be given while it is off; the switch refuses before any job, while turning
    // it off goes on as before.
    let internet = |enabled: bool| {
        let app = app.clone();
        async move {
            router(app)
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/containers/kept/internet")
                        .header("host", "localhost:8080")
                        .header("x-thoric-request", "1")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::json!({ "enabled": enabled }).to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        }
    };
    assert_eq!(internet(true).await, StatusCode::BAD_REQUEST);
    assert!(app.db.list::<Job>("job").unwrap().is_empty());
}

/// A desktop that is not ready fails the job either way, while the container keeps the error only
/// when its compose setup was due, which then did not run. No other such error outlives a desktop
/// that comes up later.
#[cfg(unix)]
#[tokio::test]
async fn only_a_setup_that_did_not_run_keeps_a_readiness_failure_on_the_container() {
    let dir = std::env::temp_dir().join(format!(
        "thoric-provisioned-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let docker = dir.join("docker");
    crate::test_support::write_script(
        &docker,
        r#"#!/bin/sh
case "$*" in
  'container inspect thoric-e') echo '[{"Id":"c0ffee","Config":{"Labels":{"thoric.managed":"true","thoric.environment":"e"}},"State":{"Running":false,"Restarting":false,"ExitCode":1}}]' ;;
  'logs --tail 20 c0ffee') echo 'GPU requested, but OpenGL initialization failed.' >&2 ;;
  *) exit 1 ;;
esac
"#,
    );
    let mut app = app(false);
    Arc::get_mut(&mut app).unwrap().config.docker_bin = docker.display().to_string();
    let job = Job {
        id: "create".into(),
        kind: "create".into(),
        resource: Some("container:e".into()),
        status: "running".into(),
        created_at: 1,
        finished_at: None,
        log: String::new(),
        result: None,
        progress: None,
        stages: Vec::new(),
        script: None,
        name: None,
    };
    app.db.put("job", "create", &job).unwrap();
    let pending = serde_json::from_value::<crate::compose::Setup>(
        json!({"status": "pending", "image": "unity", "parameters": {}, "secrets": []}),
    )
    .unwrap();
    let unready = "The desktop stopped before it was ready; it last exited with code 1. Its last log lines:\nGPU requested, but OpenGL initialization failed.";
    for (setup, kept) in [
        (None, None),
        (
            Some(pending),
            Some(format!("The compose setup did not run: {unready}")),
        ),
    ] {
        let env = Container {
            id: "e".into(),
            container: "thoric-e".into(),
            private_network: "thoric-e-private".into(),
            egress_network: "thoric-e-egress".into(),
            source_image_id: "sha256:desktop".into(),
            created_at: 1,
            spec: spec(),
            last_error: None,
            desktop: None,
            working_image: None,
            setup,
        };
        app.db.put(CONTAINERS, "e", &env).unwrap();
        let error = app.provisioned_ready(&env, "create").await.unwrap_err();
        assert_eq!(
            format!("{error:#}"),
            kept.clone().unwrap_or_else(|| unready.into())
        );
        let saved: Container = app.db.get(CONTAINERS, "e").unwrap();
        assert_eq!(saved.last_error, kept);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
