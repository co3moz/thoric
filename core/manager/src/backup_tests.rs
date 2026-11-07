//! Pins down what the backup routes promise as a whole: one backup or restore at a time, a file
//! name that sorts by date, routes the API guide lists and no MCP tool that reaches them. The
//! records the other tests of this module share are built here.
use super::*;
use crate::{
    compose::Pinned,
    desktop_mcp::Endpoint,
    model::{Container, ImageRecord},
    scale::{ScaleSet, Template},
};
use serde_json::json;
use std::collections::BTreeMap;

/// A container record as the manager saves one, mounting `volumes` at folders of its home.
pub(super) fn container(id: &str, name: &str, image: &str, volumes: &[&str]) -> Container {
    serde_json::from_value(json!({
        "id": id,
        "container": format!("thoric-{id}"),
        "private_network": format!("thoric-{id}-private"),
        "egress_network": format!("thoric-{id}-egress"),
        "source_image_id": format!("sha256:source-of-{id}"),
        "created_at": 10,
        "spec": {
            "name": name,
            "image": image,
            "hostname": format!("host-{id}"),
            "mac_address": "02:42:ac:11:00:02",
            "machine_id": "0123456789abcdef0123456789abcdef",
            "mounts": volumes.iter().enumerate().map(|(i, volume)| json!({
                "volume": volume, "target": format!("/home/desktop/v{i}"),
            })).collect::<Vec<_>>(),
        },
        "setup": {"status": "completed", "image": "img", "parameters": {"EMAIL": "a@example.com"}, "secrets": ["PASSWORD"]},
    }))
    .unwrap()
}

/// A saved image record of `kind`.
pub(super) fn image(id: &str, name: &str, kind: &str) -> ImageRecord {
    serde_json::from_value(json!({
        "id": id,
        "name": name,
        "image_id": format!("sha256:image-{id}"),
        "reference": format!("thoric/{kind}:{id}"),
        "parent_image_id": null,
        "source_container": null,
        "kind": kind,
        "created_at": 5,
        "description": "",
    }))
    .unwrap()
}

/// The secret value the fixtures' composes hold, which no file may carry without being asked.
pub(super) const SECRET: &str = "correct-horse-battery-staple";

/// A pinned compose with one ordinary and one secret value.
pub(super) fn pinned() -> Pinned {
    serde_json::from_value(json!({
        "compose": {
            "parameters": [{"name": "EMAIL"}, {"name": "PASSWORD", "secret": true}],
            "script": "echo set up",
        },
        "values": {"EMAIL": "a@example.com", "PASSWORD": SECRET},
    }))
    .unwrap()
}

/// An endpoint covering `containers`, with a token hash.
pub(super) fn endpoint(name: &str, containers: &[&str]) -> Endpoint {
    serde_json::from_value(json!({
        "name": name,
        "automatic": false,
        "grants": containers.iter().enumerate().map(|(i, id)| json!({
            "container_id": id, "alias": format!("desk-{i}"), "scopes": ["view"], "servers": "all",
        })).collect::<Vec<_>>(),
        "token_hash": "ab".repeat(32),
        "token_created_at": 3,
        "created_at": 1,
    }))
    .unwrap()
}

/// The password of a scale set's own proxy, which no file may carry without being asked.
pub(super) const SET_PROXY_SECRET: &str = "proxy-password-of-the-set";

/// A scale set whose template starts from `image` with the volume `shared`, whose instances are
/// `instances` by number and whose endpoint is named after it.
pub(super) fn scale_set(name: &str, image: &str, instances: &[(u32, &str)]) -> ScaleSet {
    serde_json::from_value(json!({
        "name": name,
        "created_at": 20,
        "template": {
            "name": name,
            "image": image,
            "mounts": [{"volume": "shared", "target": "/home/desktop/shared"}],
            "proxy": {"mode": "custom", "url": "http://proxy.example:3128", "username": "agent", "password": true},
        },
        "min": 1,
        "max": 4,
        "volumes": {"/home/desktop/shared": "first"},
        "instances": instances
            .iter()
            .map(|(number, id)| (number.to_string(), json!({"container_id": id, "note": "an agent's"})))
            .collect::<serde_json::Map<_, _>>(),
        "endpoint": name,
    }))
    .unwrap()
}

/// A set's template: the fixtures' compose pinned for the image record `i1`, the values a request
/// gave it, secret ones too, and the password of its own proxy.
pub(super) fn set_template() -> Template {
    Template {
        parameters: BTreeMap::from([
            ("EMAIL".into(), "a@example.com".into()),
            ("PASSWORD".into(), SECRET.into()),
        ]),
        pinned: Some(("i1".into(), pinned())),
        proxy_password: Some(SET_PROXY_SECRET.into()),
    }
}

/// The endpoint made for the scale set `set`, with a token hash.
pub(super) fn set_endpoint(name: &str, set: &str) -> Endpoint {
    serde_json::from_value(json!({
        "name": name,
        "automatic": true,
        "grants": [{"set": set, "alias": set, "scopes": ["view", "scale"], "servers": "all"}],
        "token_hash": "cd".repeat(32),
        "created_at": 2,
    }))
    .unwrap()
}

/// A manager whose Docker CLI is a shell script running `cases`, arms of a `case` over its
/// arguments, in a new directory of its own, where each call is appended to `calls` and the
/// manager container is `thoric-manager` with its data volume `thoric-data`.
#[cfg(unix)]
pub(super) fn fake_docker(cases: &str) -> (std::sync::Arc<App>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("thoric-backup-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("manager.json"),
        json!([{
            "Id": "manager",
            "Image": "sha256:manager",
            "Config": {"Labels": {"thoric.manager": "true"}},
            "Mounts": [{"Type": "volume", "Name": "thoric-data", "Destination": "/data"}],
            "NetworkSettings": {"Networks": {}},
        }])
        .to_string(),
    )
    .unwrap();
    let script = format!(
        "#!/bin/sh\ncd '{}'\nprintf '%s\\n' \"$*\" >> calls\ncase \"$*\" in\n  'container inspect thoric-manager') cat manager.json ;;\n{cases}\n  *) exit 1 ;;\nesac\n",
        dir.display()
    );
    let docker = dir.join("docker");
    crate::test_support::write_script(&docker, &script);
    let mut app = crate::test_support::app(false);
    let config = &mut std::sync::Arc::get_mut(&mut app).unwrap().config;
    config.docker_bin = docker.display().to_string();
    config.manager = Some("thoric-manager".into());
    config.archive_limit = 64 << 20;
    (app, dir)
}

#[cfg(unix)]
/// A running job of `kind` for the work under test to log to.
pub(super) fn job(app: &App, id: &str, kind: &str) {
    let job: crate::model::Job = serde_json::from_value(json!({
        "id": id, "kind": kind, "status": "running", "created_at": 0, "finished_at": null,
        "log": "", "result": null,
    }))
    .unwrap();
    app.db.put("job", id, &job).unwrap();
}

#[cfg(unix)]
/// A `docker save` of a one-layer image as Docker writes one, naming `name` in the places Docker
/// reads names, with `labels` on its configuration.
pub(super) fn saved_image(labels: serde_json::Value, name: &str, layer: &[u8]) -> Vec<u8> {
    use crate::archives::tar::entry;
    let config = json!({"architecture": "amd64", "os": "linux", "config": {"Labels": labels}});
    let manifest = json!([{"Config": "config.json", "RepoTags": [name], "Layers": ["layer.tar"]}]);
    let mut archive = Vec::new();
    archive.extend(entry("layer.tar", layer, 0));
    archive.extend(entry("config.json", config.to_string().as_bytes(), 0));
    archive.extend(entry("manifest.json", manifest.to_string().as_bytes(), 0));
    archive.extend(entry(
        "repositories",
        json!({name: {"latest": "x"}}).to_string().as_bytes(),
        0,
    ));
    archive.extend([0; 1024]);
    archive
}

/// Every call the fake Docker CLI received.
#[cfg(unix)]
pub(super) fn calls(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join("calls")).unwrap_or_default()
}

#[test]
fn a_backup_file_is_named_by_its_time_in_utc() {
    assert_eq!(file_name(0), "thoric-backup-19700101-000000.tar.gz");
    // 2026-10-22 13:45:06 UTC, and a leap day.
    assert_eq!(
        file_name(1_792_676_706),
        "thoric-backup-20261022-134506.tar.gz"
    );
    assert_eq!(
        file_name(951_782_400),
        "thoric-backup-20000229-000000.tar.gz"
    );
}

#[test]
fn only_one_backup_or_restore_runs_at_a_time() {
    let app = crate::test_support::app(false);
    let held = installation(&app).unwrap();
    let refused = installation(&app).unwrap_err();
    assert_eq!(refused.0, StatusCode::CONFLICT);
    drop(held);
    assert!(installation(&app).is_ok());
}

/// The API guide lists every route, and MCP has no tool that backs up or restores: a restore
/// creates containers and images at once, and a backup carries the installation's secrets.
#[test]
fn the_routes_are_documented_and_out_of_mcps_reach() {
    let api = include_str!("../../../docs/API.md");
    for route in [
        "| GET | `/api/backup` |",
        "| POST | `/api/backup/check` |",
        "| POST | `/api/backup/restore` |",
    ] {
        assert!(api.contains(route), "docs/API.md must list {route}");
    }
    // The administrator's channel: its transport, its tool catalog and what each tool runs.
    for (file, source) in [
        ("mcp.rs", include_str!("mcp.rs")),
        ("mcp/tools.rs", include_str!("mcp/tools.rs")),
        ("mcp/dispatch.rs", include_str!("mcp/dispatch.rs")),
    ] {
        assert!(
            !source.contains("backup"),
            "core/manager/src/{file} must not reach backups"
        );
    }
}
