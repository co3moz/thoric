//! Pins down which routes a GPU desktop gets from the settings and what the host showed, the
//! `docker create` arguments of each, and the errors that name the setting when a route is missing
//! or Docker refuses its devices. Also that a GPU container is refused before Docker when no route
//! is used, at creation, retry and a settings change that creates it again.
use super::*;
use crate::model;
use crate::test_support::{app, spec};
use host::Seen;
use std::sync::Arc;

fn config(seen: Option<Seen>) -> std::sync::Arc<crate::App> {
    let mut app = crate::test_support::app(false);
    std::sync::Arc::get_mut(&mut app).unwrap().config.gpu_host = Host { seen };
    app
}

fn set(app: &mut std::sync::Arc<crate::App>) -> &mut Config {
    &mut std::sync::Arc::get_mut(app).unwrap().config
}

fn wsl() -> Seen {
    Seen {
        wsl: true,
        ..Default::default()
    }
}

fn nvidia(toolkit: Option<bool>) -> Seen {
    Seen {
        nvidia: true,
        toolkit,
        ..Default::default()
    }
}

fn render(vendors: &[&str]) -> Seen {
    Seen {
        render: vendors.iter().map(|vendor| vendor.to_string()).collect(),
        ..Default::default()
    }
}

fn routes_used(config: &Config) -> Vec<Route> {
    routes(config).used
}

#[test]
fn an_unseen_host_keeps_the_settings_as_written_without_dri() {
    let mut app = config(None);
    let routes = routes(&app.config);
    assert_eq!(routes.used, [Route::Nvidia, Route::Wsl]);
    assert_eq!(routes.unused, [(Route::Dri, Missing::Unseen)]);
    set(&mut app).wsl_gpu = false;
    assert_eq!(routes_used(&app.config), [Route::Nvidia]);
}

#[test]
fn wsl_gets_the_wsl_gpu_and_cuda() {
    let app = config(Some(wsl()));
    assert_eq!(
        routes(&app.config),
        Routes {
            used: vec![Route::Nvidia, Route::Wsl],
            unused: vec![(Route::Dri, Missing::NoRenderDevice)],
        }
    );
    // Byte for byte what WSL desktops were created with before routes were chosen.
    assert_eq!(
        docker_args(&routes_used(&app.config)).join(" "),
        "--gpus all --device /dev/dxg --mount type=bind,src=/usr/lib/wsl,dst=/usr/lib/wsl,readonly"
    );
}

#[test]
fn a_linux_nvidia_host_gets_opengl_from_the_toolkit() {
    for toolkit in [Some(true), None] {
        let app = config(Some(nvidia(toolkit)));
        let routes = routes(&app.config);
        assert_eq!(routes.used, [Route::Nvidia], "{toolkit:?}");
        assert_eq!(
            routes.unused,
            [
                (Route::Wsl, Missing::NoWslGpu),
                (Route::Dri, Missing::NoRenderDevice)
            ]
        );
        assert_eq!(
            docker_args(&routes.used),
            ["--gpus", r#"all,"capabilities=compute,utility,graphics""#]
        );
    }
    let app = config(Some(nvidia(Some(false))));
    assert!(routes(&app.config).used.is_empty());
    assert!(
        routes(&app.config)
            .unused
            .contains(&(Route::Nvidia, Missing::NoToolkit))
    );
}

#[test]
fn an_amd_or_intel_host_gets_its_render_devices() {
    let mut app = config(Some(render(&["AMD"])));
    assert_eq!(routes_used(&app.config), [Route::Dri]);
    assert_eq!(
        docker_args(&routes_used(&app.config)),
        ["--device", "/dev/dri"]
    );
    set(&mut app).dri_gpu = false;
    assert!(routes_used(&app.config).is_empty());
    assert!(!app.config.gpu_enabled());
    // A hybrid laptop gets both, and NVIDIA keeps OpenGL.
    let app = config(Some(Seen {
        nvidia: true,
        toolkit: Some(true),
        render: vec!["Intel".into()],
        ..Default::default()
    }));
    assert_eq!(
        docker_args(&routes_used(&app.config)),
        [
            "--gpus",
            r#"all,"capabilities=compute,utility,graphics""#,
            "--device",
            "/dev/dri"
        ]
    );
}

#[test]
fn render_devices_never_go_beside_the_wsl_gpu() {
    let app = config(Some(Seen {
        wsl: true,
        render: vec!["AMD".into()],
        ..Default::default()
    }));
    assert_eq!(routes_used(&app.config), [Route::Nvidia, Route::Wsl]);
    assert_eq!(
        routes(&app.config).unused,
        [(Route::Dri, Missing::BesideWsl)]
    );
}

#[test]
fn a_host_without_the_allowed_gpus_refuses_naming_each_setting() {
    // WSL_GPU left at its default on a Linux host without GPUs.
    let mut app = config(Some(Seen::default()));
    assert!(!app.config.gpu_enabled());
    assert!(!app.config.feature_defaults().gpu);
    let text = refusal(&app.config).unwrap();
    for expected in [
        "WSL_GPU is true, but this host has no WSL GPU (/dev/dxg)",
        "NVIDIA_COMPUTE is true, but this host has no NVIDIA GPU with NVIDIA's driver",
        "DRI_GPU is true, but this host has no render device under /dev/dri",
        "use the container without GPU",
    ] {
        assert!(text.contains(expected), "{expected} in {text}");
    }
    let settings = set(&mut app);
    (settings.wsl_gpu, settings.cuda, settings.dri_gpu) = (false, false, false);
    assert_eq!(
        refusal(&app.config).unwrap(),
        "GPU access is disabled for this installation (WSL_GPU, NVIDIA_COMPUTE and DRI_GPU are false); use the container without GPU"
    );
    assert_eq!(refusal(&config(Some(wsl())).config), None);
}

#[test]
fn docker_errors_name_the_setting_of_the_device_they_name() {
    let all = [Route::Nvidia, Route::Wsl, Route::Dri];
    for (docker, setting) in [
        (
            r#"Docker: Error response from daemon: invalid mount config for type "bind": bind source path does not exist: /usr/lib/wsl"#,
            "WSL_GPU",
        ),
        (
            r#"Docker: Error response from daemon: error gathering device information while adding custom device "/dev/dxg": no such file or directory"#,
            "WSL_GPU",
        ),
        (
            r#"Docker: Error response from daemon: could not select device driver "" with capabilities: [[gpu]]"#,
            "NVIDIA_COMPUTE",
        ),
        (
            "Docker: Error response from daemon: failed to create task for container: nvidia-container-cli: initialization error: load library failed",
            "NVIDIA_COMPUTE",
        ),
        (
            r#"Docker: Error response from daemon: error gathering device information while adding custom device "/dev/dri": no such file or directory"#,
            "DRI_GPU",
        ),
    ] {
        let text = format!("{:#}", explain(anyhow::anyhow!(docker), &all));
        assert!(text.starts_with(setting), "{setting}: {text}");
        assert!(text.ends_with(docker), "{text}");
    }
    // An error about something else, or a route the desktop did not get, stays as it was.
    let other = "Docker: Error response from daemon: Conflict. The container name is in use";
    assert_eq!(
        format!("{:#}", explain(anyhow::anyhow!(other), &all)),
        other
    );
    let wsl = "Docker: bind source path does not exist: /usr/lib/wsl";
    assert_eq!(
        format!("{:#}", explain(anyhow::anyhow!(wsl), &[Route::Nvidia])),
        wsl
    );
}

#[test]
fn the_dashboard_names_every_route_and_reason() {
    // The match fails to compile once a variant is added, so this list keeps up with the enums.
    let reasons = [
        Missing::NoWslGpu,
        Missing::NoNvidiaGpu,
        Missing::NoToolkit,
        Missing::NoRenderDevice,
        Missing::BesideWsl,
        Missing::Unseen,
    ];
    for reason in reasons {
        match reason {
            Missing::NoWslGpu
            | Missing::NoNvidiaGpu
            | Missing::NoToolkit
            | Missing::NoRenderDevice
            | Missing::BesideWsl
            | Missing::Unseen => {}
        }
    }
    let catalog = include_str!("../../web/src/locales/en.ts");
    for reason in reasons {
        let key = format!(
            "\"settings.gpu.reason.{}\"",
            json!(reason).as_str().unwrap()
        );
        assert!(catalog.contains(&key), "en.ts has no {key}");
    }
    for route in ROUTES {
        let key = format!("\"settings.gpu.route.{}\"", json!(route).as_str().unwrap());
        assert!(catalog.contains(&key), "en.ts has no {key}");
    }
}

#[test]
fn the_settings_route_reports_routes_and_what_the_host_showed() {
    let app = config(Some(nvidia(Some(true))));
    assert_eq!(
        report(&app.config),
        json!({
            "routes": ["nvidia"],
            "unused": [
                {"route": "wsl", "setting": "WSL_GPU", "reason": "no_wsl_gpu"},
                {"route": "dri", "setting": "DRI_GPU", "reason": "no_render_device"},
            ],
            "found": {"wsl": false, "nvidia": true, "toolkit": true, "render": []},
        })
    );
    assert_eq!(report(&config(None).config)["found"], Value::Null);
}

#[tokio::test]
async fn gpu_containers_are_rejected_when_the_installation_has_no_gpu() {
    let mut app = app(false);
    {
        let config = &mut Arc::get_mut(&mut app).unwrap().config;
        config.wsl_gpu = false;
        config.cuda = false;
    }
    let mut spec = spec();
    spec.gpu = true;
    let error = app
        .create_container("gpu-test", spec, Default::default(), "gpu-job")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("GPU"), "{error}");
    assert!(
        app.db
            .list::<model::Container>(model::CONTAINERS)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_host_without_a_wsl_gpu_refuses_a_gpu_container_naming_wsl_gpu() {
    // A Linux host without GPUs, with WSL_GPU left at its default.
    let mut app = app(false);
    Arc::get_mut(&mut app).unwrap().config.gpu_host = Host {
        seen: Some(Default::default()),
    };
    let mut spec = spec();
    spec.gpu = true;
    let error = app
        .create_container("gpu-test", spec.clone(), Default::default(), "gpu-job")
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("WSL_GPU is true, but this host has no WSL GPU (/dev/dxg)"),
        "{error}"
    );
    assert!(
        app.db
            .list::<model::Container>(model::CONTAINERS)
            .unwrap()
            .is_empty()
    );
    // A retry or reset of a GPU container that exists already stops before Docker too.
    let env = model::Container {
        id: "gpu-test".into(),
        container: "thoric-gpu-test".into(),
        private_network: "thoric-gpu-test-private".into(),
        egress_network: "thoric-gpu-test-egress".into(),
        source_image_id: "sha256:base".into(),
        created_at: 1,
        spec,
        last_error: None,
        desktop: None,
        working_image: None,
        setup: None,
    };
    let error = app.create_stopped(&env, "manager").await.unwrap_err();
    assert!(error.to_string().contains("WSL_GPU"), "{error}");
    // A settings change that creates it again is refused the same way; one applied in place is not.
    let request = |body: serde_json::Value| body.as_object().unwrap().clone();
    let error = app
        .settings_preview(&env, &request(serde_json::json!({"shm_mb": 1024})))
        .unwrap_err();
    assert!(error.to_string().contains("WSL_GPU"), "{error}");
    let (_, changes) = app
        .settings_preview(&env, &request(serde_json::json!({"name": "Renamed"})))
        .unwrap();
    assert!(changes.restart.is_empty());
    // And the create form starts without GPU access.
    assert!(!app.config.feature_defaults().gpu);
}
