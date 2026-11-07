//! Pins down the `docker create` arguments: every setting of a container created again, from the
//! commit that holds its files, and Docker's own seccomp profile where user namespaces are off.
use super::*;
use crate::test_support::{DESKTOP_CONTEXT, app, spec};
use std::sync::Arc;

#[test]
fn a_recreated_container_gets_every_setting_from_the_commit_that_holds_its_files() {
    let mut app = app(false);
    Arc::get_mut(&mut app).unwrap().config.adapter = "NVIDIA".into();
    let mut spec = spec();
    spec.resolution = "1280x720".into();
    spec.audio = true;
    spec.gpu = true;
    spec.gamepad = true;
    spec.vpn = true;
    spec.debugger = true;
    spec.realtime = true;
    spec.profiler = true;
    spec.kvm = true;
    spec.shm_mb = 1024;
    spec.tmp_mb = 2048;
    spec.memory_mb = Some(2048);
    spec.mounts = vec![Mount {
        volume: "work".into(),
        target: "/workspace".into(),
        readonly: true,
    }];
    let mut env = Container {
        id: "args-id".into(),
        container: "thoric-args-id".into(),
        private_network: "thoric-args-id-private".into(),
        egress_network: "thoric-args-id-egress".into(),
        source_image_id: "sha256:source".into(),
        created_at: 0,
        spec,
        last_error: None,
        desktop: None,
        working_image: None,
        setup: None,
    };
    let joined = |env: &Container| app.container_args(env, Some("172.30.0.0/16")).join(" ");
    let args = joined(&env);
    let profile = std::path::Path::new(DESKTOP_CONTEXT).join("seccomp.json");
    let seccomp = format!("--security-opt seccomp={}", profile.display());
    for flag in [
        seccomp.as_str(),
        "--name thoric-args-id",
        "--shm-size 1024m",
        "--tmpfs /tmp:rw,nosuid,size=2048m",
        "--env DESKTOP_RESOLUTION=1280x720",
        "--env DESKTOP_AUDIO=true",
        "--env DESKTOP_GPU=true",
        "--env GPU_ADAPTER=NVIDIA",
        "--env DESKTOP_PRIVATE_SUBNET=172.30.0.0/16",
        "--memory 2048m",
        "--mount type=volume,src=work,dst=/workspace,readonly",
        "--gpus all",
        "--device /dev/dxg",
        "--device /dev/uinput",
        "--cap-add NET_ADMIN",
        "--device /dev/net/tun",
        "--sysctl net.ipv4.conf.all.src_valid_mark=1",
        "--cap-add SYS_PTRACE",
        "--cap-add PERFMON",
        "--ulimit rtprio=95",
        "--device /dev/kvm",
        "--label thoric.environment=args-id",
    ] {
        assert!(args.contains(flag), "{flag} missing from {args}");
    }
    assert!(args.ends_with(" sha256:source"), "{args}");
    env.working_image = Some(WorkingImage {
        image_id: "sha256:working".into(),
        reference: "thoric/container:job".into(),
    });
    assert!(joined(&env).ends_with(" sha256:working"));
    assert_eq!(env.source_image_id, "sha256:source");
}

#[test]
fn an_installation_without_user_namespaces_leaves_dockers_seccomp_profile_in_place() {
    let mut app = app(false);
    Arc::get_mut(&mut app).unwrap().config.user_namespaces = false;
    let env = Container {
        id: "plain-id".into(),
        container: "thoric-plain-id".into(),
        private_network: "thoric-plain-id-private".into(),
        egress_network: "thoric-plain-id-egress".into(),
        source_image_id: "sha256:source".into(),
        created_at: 0,
        spec: spec(),
        last_error: None,
        desktop: None,
        working_image: None,
        setup: None,
    };
    assert_eq!(app.config.seccomp_profile(), None);
    let args = app.container_args(&env, None);
    assert!(!args.iter().any(|arg| arg == "--security-opt"), "{args:?}");
}

#[test]
fn every_variable_a_desktop_is_given_is_one_the_script_editor_knows() {
    let app = app(false);
    let env = Container {
        id: "known-id".into(),
        container: "thoric-known-id".into(),
        private_network: "thoric-known-id-private".into(),
        egress_network: "thoric-known-id-egress".into(),
        source_image_id: "sha256:source".into(),
        created_at: 0,
        spec: spec(),
        last_error: None,
        desktop: None,
        working_image: None,
        setup: None,
    };
    let args = app.container_args(&env, Some("172.30.0.0/16"));
    let given: Vec<&str> = args
        .windows(2)
        .filter(|pair| pair[0] == "--env")
        .filter_map(|pair| pair[1].split('=').next())
        .collect();
    // The dashboard's script editor completes these names, or leaves them out on purpose.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../web/src/lib/script-completions.json");
    let list: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let names = |value: &serde_json::Value| -> Vec<String> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| {
                entry
                    .get("name")
                    .unwrap_or(entry)
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    };
    let mut known = names(&list["variables"]);
    known.extend(names(&list["unlisted"]["variables"]));
    assert!(given.contains(&"DESKTOP_GPU"), "{given:?}");
    for name in given {
        assert!(
            known.iter().any(|known| known == name),
            "{name} is given to every desktop; name it in core/web/src/lib/script-completions.json"
        );
    }
}
