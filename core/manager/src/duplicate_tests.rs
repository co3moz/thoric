//! Pins down what a duplicate copies and what it does not: every setting but the name and the
//! machine identity, which are new; the compose values, secret ones too, and the proxy's password
//! record to record and never into the job log; copies of the volumes created for the original
//! while the others stay shared; a setup that completed is not run again; aliases another member
//! of a group answers to are left out. Also that a name is refused while another container has
//! it or another duplicate is making it, and that a failure takes back what the job made.
use super::*;
use crate::{model::Job, test_support::app};
use std::sync::Arc;

const SECRET: &str = "s3cret-value-for-the-setup";
const PASSWORD: &str = "proxy-pass-of-the-original";

/// A compose with a workspace volume each container gets of its own and a secret token.
fn compose() -> crate::compose::Compose {
    serde_json::from_value(json!({
        "parameters": [
            {"name": "WORKSPACE", "type": "volume", "path": "/home/desktop/Projects", "new_volume": true},
            {"name": "TOKEN", "secret": true},
            {"name": "EDITOR", "default": "vim"},
        ],
        "script": "true\n",
    }))
    .unwrap()
}

/// The container `orig` as a creation left it: in a group, with an alias, its own proxy with a
/// password, a shared volume, the workspace volume created for it and a setup that completed.
fn original() -> (Container, Pinned) {
    let values = compose()
        .resolve(&[("TOKEN".to_string(), SECRET.to_string())].into(), "orig")
        .unwrap();
    let pinned = Pinned {
        compose: compose(),
        values,
    };
    let mut setup = pinned.setup("img");
    setup.status = "completed".into();
    setup.finished_at = Some(5);
    let spec: ContainerSpec = serde_json::from_value(json!({
        "name": "Unity", "image": "example/desktop:1", "audio": true, "vpn": true,
        "resolution": "2560x1440", "cpus": 2.5, "memory_mb": 4096, "shm_mb": 1024,
        "ports": [8090], "groups": ["build"], "aliases": ["editor", "license"],
        "hostname": "license-box", "mac_address": "02:42:ac:11:00:02",
        "machine_id": "0123456789abcdef0123456789abcdef",
        "proxy": {"mode": "custom", "url": "http://proxy.example:3128", "password": true},
        "mounts": [{"volume": "assets", "target": "/home/desktop/Assets", "readonly": true}],
        "lifecycle": {"idle_stop_minutes": 30, "expires_at": 4_102_444_800_u64,
            "on_expiry": "delete", "delete_volumes": true},
    }))
    .unwrap();
    let env = Container {
        id: "orig".into(),
        container: "thoric-orig".into(),
        private_network: "thoric-orig-private".into(),
        egress_network: "thoric-orig-egress".into(),
        source_image_id: "sha256:desktop".into(),
        created_at: 1,
        spec,
        last_error: None,
        desktop: None,
        working_image: None,
        setup: Some(setup),
    };
    (env, pinned)
}

fn snapshot() -> ImageRecord {
    serde_json::from_value(json!({
        "id": "job1", "name": "Unity copy", "image_id": "sha256:snap",
        "reference": "thoric/snapshot:job1", "parent_image_id": "sha256:desktop",
        "source_container": "orig", "kind": "snapshot", "created_at": 2, "description": "",
    }))
    .unwrap()
}

fn request(name: &str) -> Duplicate {
    serde_json::from_value(json!({"name": name})).unwrap()
}

#[test]
fn a_duplicate_has_every_setting_but_the_name_and_the_machine_identity() {
    let (original, _) = original();
    let request = request("Unity copy");
    assert!(request.start && request.mcp_endpoint);
    let spec = copied_spec(&original.spec, &request.name);
    let env = duplicate_record(&original, "dup", spec, &snapshot(), &[], true);
    assert_eq!(env.spec.name, "Unity copy");
    // A new machine: its own hostname, MAC address and machine ID.
    assert_eq!(env.spec.hostname.as_deref(), Some("thoric-dup"));
    for (new, old) in [
        (&env.spec.mac_address, &original.spec.mac_address),
        (&env.spec.machine_id, &original.spec.machine_id),
    ] {
        assert!(new.is_some() && new != old, "{new:?}");
    }
    // It starts from the snapshot, so a reset returns it to the moment it was made.
    assert_eq!(env.spec.image, "thoric/snapshot:job1");
    assert_eq!(env.source_image_id, "sha256:snap");
    assert_eq!(env.working_image, None);
    assert_eq!(
        [&env.container, &env.private_network, &env.egress_network],
        ["thoric-dup", "thoric-dup-private", "thoric-dup-egress"]
    );
    // Everything else rides along, also a field a later release adds to the settings.
    let mut same = env.spec.clone();
    same.name = original.spec.name.clone();
    same.image = original.spec.image.clone();
    same.hostname = original.spec.hostname.clone();
    same.mac_address = original.spec.mac_address.clone();
    same.machine_id = original.spec.machine_id.clone();
    same.lifecycle.expires_at = original.spec.lifecycle.expires_at;
    assert_eq!(same, original.spec);
    env.spec.validate().unwrap();
}

#[test]
fn a_duplicate_keeps_the_idle_stop_but_not_the_expiry() {
    let (original, _) = original();
    let spec = copied_spec(&original.spec, "Unity copy");
    // The idle stop is how the desktop is to be used, which the copy is too.
    assert_eq!(spec.lifecycle.idle_stop_minutes, 30);
    // An expiry is when the original's own task ends; the copy would otherwise go with it.
    assert!(original.spec.lifecycle.expires_at.is_some());
    assert_eq!(spec.lifecycle.expires_at, None);
    // What an expiry does stays, so one set later on the copy does what the original's did.
    assert_eq!(spec.lifecycle.on_expiry, crate::lifecycle::OnExpiry::Delete);
    assert!(spec.lifecycle.delete_volumes);
    spec.validate().unwrap();
}

#[test]
fn the_volumes_created_for_the_original_get_copies_and_the_others_stay_shared() {
    let (original, pinned) = original();
    let owned = vec![
        "thoric-orig-workspace".to_string(),
        "thoric-orig-unused".into(),
    ];
    let moves = moves(&owned, &original.mounts(), "orig", "dup");
    // Only the ones it mounts, each once.
    assert_eq!(
        moves,
        [Moved {
            from: "thoric-orig-workspace".into(),
            to: "thoric-dup-workspace".into(),
        }]
    );
    let spec = copied_spec(&original.spec, "Unity copy");
    let env = duplicate_record(&original, "dup", spec, &snapshot(), &moves, true);
    let volumes = env
        .mounts()
        .into_iter()
        .map(|mount| (mount.volume, mount.target, mount.readonly))
        .collect::<Vec<_>>();
    assert_eq!(
        volumes,
        [
            ("assets".into(), "/home/desktop/Assets".into(), true),
            (
                "thoric-dup-workspace".into(),
                "/home/desktop/Projects".into(),
                false
            ),
        ]
    );
    let setup = env.setup.as_ref().unwrap();
    assert_eq!(setup.parameters["WORKSPACE"], "thoric-dup-workspace");
    assert_eq!(setup.image, "job1");
    // The pinned values name the copy, as a creation of the duplicate would have named it.
    let mut values = pinned.values.clone();
    move_values(&mut values, &moves);
    let copied = Pinned {
        compose: compose(),
        values,
    };
    assert_eq!(
        copied.new_volumes("dup"),
        [("WORKSPACE".to_string(), "thoric-dup-workspace")]
    );
    assert_eq!(copied.values["TOKEN"], SECRET);
    assert_eq!(copied.mounts(), env.compose_mounts());
}

#[test]
fn a_copy_is_named_after_the_duplicate() {
    assert_eq!(
        moved_name("thoric-orig-unity-step-workspace", "orig", "dup"),
        "thoric-dup-unity-step-workspace"
    );
    // A volume created for the original under another name keeps it after the duplicate's.
    assert_eq!(moved_name("restored", "orig", "dup"), "thoric-dup-restored");
    let long = format!("thoric-orig-{}", "a".repeat(120));
    let id = "d".repeat(32);
    let name = moved_name(&long, "orig", &id);
    assert_eq!(name.len(), 128);
    crate::model::volume_name(&name).unwrap();
}

#[test]
fn a_completed_setup_is_not_run_again_and_an_unfinished_one_runs_when_started() {
    let (mut original, _) = original();
    let spec = || copied_spec(&original.spec, "Unity copy");
    let env = duplicate_record(&original, "dup", spec(), &snapshot(), &[], true);
    assert_eq!(env.setup.as_ref().unwrap().status, "completed");
    assert!(!crate::compose::setup_due(&env));
    assert_eq!(env.last_error, None);

    original.setup.as_mut().unwrap().status = "failed".into();
    let started = duplicate_record(&original, "dup", spec(), &snapshot(), &[], true);
    let setup = started.setup.as_ref().unwrap();
    assert_eq!(
        (setup.status.as_str(), setup.finished_at),
        ("pending", None)
    );
    assert!(crate::compose::setup_due(&started));
    assert_eq!(started.last_error, None);
    // One left stopped says that a retry runs it, as the dashboard's Retry offers.
    let stopped = duplicate_record(&original, "dup", spec(), &snapshot(), &[], false);
    assert_eq!(stopped.setup.as_ref().unwrap().status, "failed");
    assert_eq!(stopped.last_error.as_deref(), Some(SETUP_WAITS));
    // Only the names of the secrets are on the record, as on the original's.
    let shown = serde_json::to_string(&stopped).unwrap();
    assert!(!shown.contains(SECRET) && shown.contains("TOKEN"));
}

#[test]
fn aliases_another_member_of_a_group_answers_to_are_left_out() {
    let (original, _) = original();
    let others = vec![original.clone()];
    let mut spec = copied_spec(&original.spec, "Unity copy");
    assert_eq!(
        free_aliases("dup", &mut spec, &others),
        ["editor", "license"]
    );
    assert!(spec.aliases.is_empty() && spec.groups == ["build"]);
    crate::identity::assign(&mut spec, "thoric-dup");
    crate::groups::names_free("dup", &spec, &others).unwrap();
    // A name the group answers to already is refused rather than shared.
    let mut same = copied_spec(&original.spec, "unity");
    free_aliases("dup", &mut same, &others);
    crate::identity::assign(&mut same, "thoric-dup");
    assert!(crate::groups::names_free("dup", &same, &others).is_err());
    // Outside a group an alias reaches nothing, and all of them stay.
    let mut alone = copied_spec(&original.spec, "Unity copy");
    alone.groups.clear();
    assert!(free_aliases("dup", &mut alone, &others).is_empty());
    assert_eq!(alone.aliases, ["editor", "license"]);
}

#[test]
fn a_name_another_container_has_is_refused() {
    let (original, _) = original();
    let Err(Error(status, message)) = name_free(" unity ", std::slice::from_ref(&original)) else {
        panic!("a taken name was accepted");
    };
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(message.contains("\"Unity\""), "{message}");
    name_free("Unity copy", &[original]).unwrap();
}

#[tokio::test]
async fn a_name_is_reserved_while_a_duplicate_makes_it() {
    let app = app(false);
    let (original, _) = original();
    app.db.put(CONTAINERS, "orig", &original).unwrap();
    // The first duplicate waits for the original while a change holds it.
    let held = app.resource_lock("container:orig").lock_owned().await;
    let first = duplicate(
        State(app.clone()),
        Path("orig".into()),
        Json(request("Unity copy")),
    )
    .await
    .unwrap();
    let Err(Error(status, message)) = duplicate(
        State(app.clone()),
        Path("orig".into()),
        Json(request("unity COPY")),
    )
    .await
    else {
        panic!("two duplicates took one name");
    };
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(message.contains("Another duplicate"), "{message}");
    // The job shows what it makes before the record exists.
    let job: Job = app.db.job(first.0["job_id"].as_str().unwrap()).unwrap();
    assert_eq!(
        (job.kind.as_str(), job.name.as_deref()),
        ("duplicate", Some("Unity copy"))
    );
    assert!(job.resource.unwrap().starts_with("container:"));
    drop(held);
    // Without a manager container it fails, makes nothing and frees the name.
    let job = finished(&app, &first).await;
    assert_eq!(job.status, "failed");
    assert_eq!(app.db.ids(CONTAINERS).unwrap(), ["orig"]);
    let again = duplicate(
        State(app.clone()),
        Path("orig".into()),
        Json(request("Unity copy")),
    )
    .await
    .unwrap();
    finished(&app, &again).await;
    // A name that is not one is refused before any job.
    assert!(
        duplicate(State(app.clone()), Path("orig".into()), Json(request(" ")))
            .await
            .is_err()
    );
}

async fn finished(app: &App, started: &Json<Value>) -> Job {
    let id = started.0["job_id"].as_str().unwrap();
    loop {
        let job: Job = app.db.job(id).unwrap();
        if job.finished_at.is_some() {
            return job;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// A manager whose Docker CLI is a stand-in that answers as Docker with the original's container,
/// its volumes and a snapshot, and records its arguments in the returned file. With `owned`, the
/// workspace volume is one created for the original, which the stand-in lets a duplicate create
/// a copy of and then fails the helper that copies it; without, the workspace is a volume both
/// share and the stand-in fails creating the duplicate's network. The original's records are
/// saved: its compose values and its proxy's password.
#[cfg(unix)]
fn docker_app(owned: bool) -> (Arc<App>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "thoric-duplicate-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let calls = dir.join("calls");
    let docker = dir.join("docker");
    let created = dir.join("created");
    let answers = [
        (
            "container inspect thoric-manager",
            json!([{"Id": "manager", "Image": "sha256:manager",
                "Config": {"Labels": {"thoric.manager": "true"}}, "Mounts": []}]),
        ),
        (
            "container inspect thoric-orig",
            json!([{"Id": "orig-docker", "State": {"Running": true},
                "Config": {"Labels": {"thoric.managed": "true", "thoric.environment": "orig",
                    "thoric.desktop": "true"}}}]),
        ),
        (
            "volume inspect thoric-orig-workspace",
            json!([{"Name": "thoric-orig-workspace", "Labels": {"thoric.managed": "true",
                "thoric.environment": "orig", "thoric.parameter": "WORKSPACE"}}]),
        ),
        (
            "volume inspect assets",
            json!([{"Name": "assets", "Labels": {"thoric.managed": "true"}}]),
        ),
    ];
    let mut script = format!(
        "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$*\" in\n",
        calls.display()
    );
    for (command, answer) in answers {
        script.push_str(&format!("  '{command}') echo '{answer}' ;;\n"));
    }
    if owned {
        script.push_str("  'volume ls --filter label=thoric.managed=true --filter label=thoric.environment=orig '*) echo thoric-orig-workspace ;;\n");
    }
    // A copy is there once it was created, labeled for the container its name names.
    script.push_str(&format!(
        "  'volume create '*) touch '{created}' ;;\n  'volume inspect thoric-'*'-workspace') [ -f '{created}' ] || exit 1; id=${{3#thoric-}}; id=${{id%-workspace}}; echo \"[{{\\\"Name\\\":\\\"$3\\\",\\\"Labels\\\":{{\\\"thoric.managed\\\":\\\"true\\\",\\\"thoric.environment\\\":\\\"$id\\\"}}}}]\" ;;\n",
        created = created.display()
    ));
    // A commit takes a moment, as Docker's does while it pauses the desktop.
    script.push_str(&format!(
        "  'volume ls'*|'ps -a'*|'network ls'*) ;;\n  'commit '*) sleep 1; echo sha256:snap ;;\n  'image inspect '*) echo '{}' ;;\n  'image rm '*|'volume rm '*) echo removed ;;\n  *) echo \"boom: $*\" >&2; exit 1 ;;\nesac\n",
        json!([{"Id": "sha256:snap", "Config": {"Labels": {"thoric.desktop": "true", "thoric.managed": "true"}}}]),
    ));
    crate::test_support::write_script(&docker, &script);
    let mut app = app(false);
    let config = &mut Arc::get_mut(&mut app).unwrap().config;
    config.docker_bin = docker.display().to_string();
    config.manager = Some("thoric-manager".into());
    config.data = dir.join("data");
    let (original, pinned) = original();
    app.db.put(CONTAINERS, "orig", &original).unwrap();
    app.db.put(PINNED, "orig", &pinned).unwrap();
    app.db
        .put(
            CREDENTIALS,
            "orig",
            &Credentials {
                password: PASSWORD.into(),
            },
        )
        .unwrap();
    (app, calls)
}

#[cfg(unix)]
#[tokio::test]
async fn the_plan_copies_compose_values_and_the_proxy_password_record_to_record() {
    let (app, _) = docker_app(true);
    let (original, _) = original();
    let job: Job = serde_json::from_value(json!({
        "id": "j", "kind": "duplicate", "status": "running", "created_at": 1,
        "finished_at": null, "log": "", "result": null,
    }))
    .unwrap();
    app.db.put("job", "j", &job).unwrap();
    let job_id = "j";
    let manager = app.inspect("container", "thoric-manager").await.unwrap();
    let docker = app.inspect("container", "thoric-orig").await.unwrap();
    let plan = app
        .plan(
            &original,
            "dup",
            &request("Unity copy"),
            (&manager, &docker),
            job_id,
        )
        .await
        .unwrap();
    let pinned = plan.pinned.unwrap();
    assert_eq!(pinned.values["TOKEN"], SECRET);
    assert_eq!(pinned.values["WORKSPACE"], "thoric-dup-workspace");
    assert_eq!(plan.credentials.unwrap().password, PASSWORD);
    assert_eq!(plan.shared, ["assets"]);
    assert_eq!(plan.aliases_left_out, ["editor", "license"]);
    assert!(plan.switched_off.is_empty());
    let log = app.db.job(job_id).unwrap().log;
    assert!(log.contains("thoric-dup-workspace"), "{log}");
    assert!(!log.contains(SECRET) && !log.contains(PASSWORD), "{log}");
}

/// Run a duplicate of `orig` that is left stopped against `app` until it ends, and return the job
/// with the Docker commands it ran.
#[cfg(unix)]
async fn failed_duplicate(app: &Arc<App>, calls: &std::path::Path) -> (Job, String) {
    let started = duplicate(
        State(app.clone()),
        Path("orig".into()),
        Json(serde_json::from_value(json!({"name": "Unity copy", "start": false})).unwrap()),
    )
    .await
    .unwrap();
    let job = finished(app, &started).await;
    assert_eq!(job.status, "failed", "{}", job.log);
    let calls = std::fs::read_to_string(calls).unwrap();
    // The original's files were committed first.
    assert!(
        calls.contains(&format!(
            "commit --change LABEL thoric.managed=true --message Thoric snapshot of Unity orig-docker thoric/snapshot:{}",
            job.id
        )),
        "{calls}"
    );
    // The snapshot went again, and of the records only the original's are left.
    assert!(
        calls.contains(&format!("image rm thoric/snapshot:{}", job.id)),
        "{calls}"
    );
    assert!(app.db.ids("image").unwrap().is_empty());
    assert_eq!(app.db.ids(CONTAINERS).unwrap(), ["orig"]);
    assert_eq!(app.db.ids(PINNED).unwrap(), ["orig"]);
    assert_eq!(app.db.ids(CREDENTIALS).unwrap(), ["orig"]);
    assert!(!job.log.contains(SECRET) && !job.log.contains(PASSWORD));
    (job, calls)
}

#[cfg(unix)]
fn begun(job: &Job) -> Vec<&str> {
    job.stages
        .iter()
        .filter(|stage| stage.started_at.is_some())
        .map(|stage| stage.name.as_str())
        .collect()
}

#[cfg(unix)]
#[tokio::test]
async fn a_duplicate_whose_volume_copy_fails_takes_back_the_volume_and_the_snapshot() {
    let (app, calls) = docker_app(true);
    let (job, calls) = failed_duplicate(&app, &calls).await;
    assert_eq!(begun(&job), ["check", "commit", "record", "volumes"]);
    // The copy was created for the duplicate, with its owner's labels, and removed again.
    let created = calls
        .lines()
        .find(|line| line.starts_with("volume create "))
        .unwrap_or_else(|| panic!("{calls}"));
    let name = created.rsplit(' ').next().unwrap();
    assert!(name.starts_with("thoric-") && name.ends_with("-workspace") && !name.contains("orig"));
    assert!(created.contains("thoric.parameter=WORKSPACE"), "{created}");
    assert!(calls.contains(&format!("volume rm {name}")), "{calls}");
    assert!(!calls.contains("volume rm thoric-orig"), "{calls}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_duplicate_whose_container_cannot_be_created_takes_back_its_records() {
    let (app, calls) = docker_app(false);
    let (job, calls) = failed_duplicate(&app, &calls).await;
    // Its record was saved, then deleted with the networks it had, which were none yet.
    assert_eq!(begun(&job), ["check", "commit", "record", "create"]);
    assert!(
        job.log
            .contains("Checking ownership before deleting \"Unity copy\""),
        "{}",
        job.log
    );
    assert!(calls.contains("network create"), "{calls}");
    assert!(!calls.contains("volume rm"), "{calls}");
}

#[cfg(unix)]
#[tokio::test]
async fn the_original_is_held_alone_while_docker_commits_it() {
    let (app, _) = docker_app(false);
    let started = duplicate(
        State(app.clone()),
        Path("orig".into()),
        Json(request("Unity copy")),
    )
    .await
    .unwrap();
    let id = started.0["job_id"].as_str().unwrap();
    loop {
        let job = app.db.job(id).unwrap();
        if job.log.contains("Committing container changes") {
            break;
        }
        assert!(job.finished_at.is_none(), "{}", job.log);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    // Docker pauses the desktop while it commits, so work that only uses it, such as input or a
    // script, waits as it waits for a snapshot, and a request gives up.
    let wait = std::time::Duration::from_millis(100);
    assert!(app.share_for("container:orig", wait).await.is_err());
    assert_eq!(finished(&app, &started).await.status, "failed");
    app.share_for("container:orig", wait).await.unwrap();
}

/// The dialog checks a name before the manager does, with the same rules.
#[test]
fn the_dialog_compares_names_as_the_manager_does() {
    let dialog = include_str!("../../web/src/lib/duplicate.ts");
    assert!(dialog.contains("const NAME_LIMIT = 100"));
    assert!(dialog.contains("name.trim().toLowerCase()"));
    assert_eq!(name_key(" Editor COPY "), "editor copy");
    let mut spec = crate::test_support::spec();
    spec.name = "x".repeat(100);
    spec.validate().unwrap();
    spec.name.push('x');
    assert!(spec.validate().is_err());
}

#[test]
fn the_guides_describe_the_route_and_the_tool() {
    let api = include_str!("../../../docs/API.md");
    assert!(api.contains("| POST | `/api/containers/{id}/duplicate` |"));
    let guide = include_str!("../../../docs/MCP.md");
    assert!(guide.contains("`duplicate_container`"));
}
