//! Pins down what each instance of a scale set mounts: the template's volume for a shared folder,
//! an empty one of its own named after it for `new`, the template's for instance 1 alone with
//! `first`, what a request for one instance asks over the policy, and a volume of its own in place
//! of a shared one that is gone.
use super::*;
use crate::compose::{Compose, Pinned};
use serde_json::json;

fn set(volumes: &[(&str, VolumePolicy)]) -> ScaleSet {
    let spec: crate::model::ContainerSpec = serde_json::from_value(json!({
        "name": "re",
        "image": "thoric/custom:re",
        "mounts": [
            {"volume": "analysis", "target": "/home/desktop/analysis"},
            {"volume": "tools", "target": "/opt/tools", "readonly": true},
        ],
    }))
    .unwrap();
    let mut set = crate::scale::new_set("re", &spec, &crate::scale::Scaling::default());
    set.volumes = volumes
        .iter()
        .map(|(target, policy)| (target.to_string(), *policy))
        .collect();
    set
}

/// A compose of two steps, one with a volume parameter that makes a volume of its own when left
/// empty, as the chrome-mcp template's browser profile does.
fn pinned() -> Pinned {
    let compose: Compose = serde_json::from_value(json!({"steps": [
        {"id": "chrome-mcp", "script": "true", "parameters": [
            {"name": "BROWSER_PROFILE", "type": "volume", "path": "/home/desktop/.config/chrome-agent", "new_volume": true},
            {"name": "CAPS", "default": "network"},
        ]},
        {"id": "vscode", "script": "true", "parameters": [{"name": "GIT_NAME"}]},
    ]}))
    .unwrap();
    let values = compose.resolve(&Default::default(), "first").unwrap();
    Pinned { compose, values }
}

fn volume_at<'a>(planned: &'a Planned, target: &str) -> &'a str {
    planned
        .mounts
        .iter()
        .find(|mount| mount.target == target)
        .map(|mount| mount.volume.as_str())
        .unwrap()
}

#[test]
fn a_shared_volume_is_the_templates_in_every_instance() {
    let set = set(&[]);
    for number in [1, 2, 7] {
        let planned = plan(&set, None, number, "c7", &BTreeMap::new(), &[]);
        assert_eq!(volume_at(&planned, "/home/desktop/analysis"), "analysis");
        assert_eq!(volume_at(&planned, "/opt/tools"), "tools");
        assert!(planned.own.is_empty());
        assert_eq!(planned.shared, ["/home/desktop/analysis", "/opt/tools"]);
    }
}

#[test]
fn a_new_volume_is_each_instances_own_and_keeps_the_folders_access() {
    let set = set(&[("/opt/tools", VolumePolicy::New)]);
    let planned = plan(&set, None, 1, "c1", &BTreeMap::new(), &[]);
    assert_eq!(
        volume_at(&planned, "/opt/tools"),
        "thoric-c1-mount-opt-tools"
    );
    assert!(
        planned
            .mounts
            .iter()
            .any(|mount| mount.target == "/opt/tools" && mount.readonly)
    );
    assert_eq!(
        planned.own,
        [(
            "/opt/tools".to_owned(),
            "thoric-c1-mount-opt-tools".to_owned()
        )]
    );
    let other = plan(&set, None, 2, "c2", &BTreeMap::new(), &[]);
    assert_eq!(volume_at(&other, "/opt/tools"), "thoric-c2-mount-opt-tools");
    assert_eq!(volume_at(&other, "/home/desktop/analysis"), "analysis");
}

#[test]
fn first_keeps_the_templates_volume_for_instance_one_alone() {
    let set = set(&[("/home/desktop/analysis", VolumePolicy::First)]);
    let one = plan(&set, None, 1, "c1", &BTreeMap::new(), &[]);
    assert_eq!(volume_at(&one, "/home/desktop/analysis"), "analysis");
    assert!(one.own.is_empty());
    let two = plan(&set, None, 2, "c2", &BTreeMap::new(), &[]);
    assert_eq!(
        volume_at(&two, "/home/desktop/analysis"),
        "thoric-c2-mount-home-desktop-analysis"
    );
}

#[test]
fn a_request_for_one_instance_wins_over_the_policy() {
    let set = set(&[("/opt/tools", VolumePolicy::New)]);
    let asked = BTreeMap::from([
        ("/opt/tools".to_owned(), Override::Shared),
        ("/home/desktop/analysis".to_owned(), Override::New),
    ]);
    let planned = plan(&set, None, 3, "c3", &asked, &[]);
    assert_eq!(volume_at(&planned, "/opt/tools"), "tools");
    assert_eq!(
        volume_at(&planned, "/home/desktop/analysis"),
        "thoric-c3-mount-home-desktop-analysis"
    );
}

#[test]
fn a_compose_volume_is_planned_as_its_value_and_named_as_new_volume_names_one() {
    let pinned = pinned();
    // Shared, every instance mounts the volume the first one got of its own.
    let set_shared = set(&[]);
    let planned = plan(&set_shared, Some(&pinned), 2, "c2", &BTreeMap::new(), &[]);
    let values = planned.values.as_ref().unwrap();
    assert_eq!(
        values["chrome-mcp.BROWSER_PROFILE"],
        "thoric-first-chrome-mcp-browser-profile"
    );
    assert_eq!(values["chrome-mcp.CAPS"], "network");
    assert!(
        planned
            .mounts
            .iter()
            .all(|mount| mount.target != "/home/desktop/.config/chrome-agent"),
        "a compose's volume goes by its value, not the settings"
    );
    // New, each gets the volume its compose's own parameter would have given it.
    let set_new = set(&[("/home/desktop/.config/chrome-agent", VolumePolicy::New)]);
    let planned = plan(&set_new, Some(&pinned), 2, "c2", &BTreeMap::new(), &[]);
    let values = planned.values.as_ref().unwrap();
    assert_eq!(
        values["chrome-mcp.BROWSER_PROFILE"],
        "thoric-c2-chrome-mcp-browser-profile"
    );
    assert_eq!(
        planned.own,
        [(
            "chrome-mcp.BROWSER_PROFILE".to_owned(),
            "thoric-c2-chrome-mcp-browser-profile".to_owned()
        )]
    );
}

#[test]
fn a_shared_volume_that_is_gone_becomes_the_instances_own() {
    let pinned = pinned();
    let set = set(&[]);
    let gone = ["thoric-first-chrome-mcp-browser-profile".to_owned()];
    let planned = plan(&set, Some(&pinned), 4, "c4", &BTreeMap::new(), &gone);
    assert_eq!(
        planned.values.as_ref().unwrap()["chrome-mcp.BROWSER_PROFILE"],
        "thoric-c4-chrome-mcp-browser-profile"
    );
    assert_eq!(volume_at(&planned, "/home/desktop/analysis"), "analysis");
}

#[test]
fn folders_whose_names_clean_to_one_get_volumes_apart() {
    let spec: crate::model::ContainerSpec = serde_json::from_value(json!({
        "name": "re",
        "image": "thoric/custom:re",
        "mounts": [
            {"volume": "a", "target": "/data/a-b"},
            {"volume": "b", "target": "/data/a/b"},
        ],
    }))
    .unwrap();
    let mut set = crate::scale::new_set("re", &spec, &crate::scale::Scaling::default());
    set.volumes = BTreeMap::from([
        ("/data/a-b".to_owned(), VolumePolicy::New),
        ("/data/a/b".to_owned(), VolumePolicy::New),
    ]);
    let planned = plan(&set, None, 1, "c1", &BTreeMap::new(), &[]);
    let names = planned
        .own
        .iter()
        .map(|(_, name)| name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        ["thoric-c1-mount-data-a-b", "thoric-c1-mount-data-a-b-2"]
    );
}

#[test]
fn only_the_templates_folders_can_be_chosen() {
    let set = set(&[]);
    let slots = slots(&set, Some(&pinned()));
    let known = BTreeMap::from([
        ("/opt/tools".to_owned(), Override::New),
        (
            "/home/desktop/.config/chrome-agent".to_owned(),
            Override::New,
        ),
    ]);
    check_targets(&slots, &known).unwrap();
    let unknown = BTreeMap::from([("/home/desktop/work".to_owned(), Override::New)]);
    let error = check_targets(&slots, &unknown).unwrap_err().to_string();
    assert!(error.contains("/home/desktop/work"), "{error}");
    assert!(error.contains("/home/desktop/analysis"), "{error}");
}
