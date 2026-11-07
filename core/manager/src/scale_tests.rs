//! Pins down a scale set's own rules: the names it accepts, its limits, the number a new instance
//! takes, which instances it keeps and so which an agent or the idle delete may remove, what its
//! template leaves out of the first container's settings, and what it shows without its secrets.
use super::*;
use std::time::Duration;

fn spec() -> ContainerSpec {
    serde_json::from_value(json!({
        "name": "Reverse engineering",
        "image": "thoric/custom:re",
        "hostname": "re1",
        "mac_address": "02:42:ac:11:00:02",
        "machine_id": "0123456789abcdef0123456789abcdef",
        "mounts": [{"volume": "analysis", "target": "/home/desktop/analysis"}],
        "lifecycle": {"idle_stop_minutes": 30, "expires_at": 4_000_000_000u64},
    }))
    .unwrap()
}

fn set_with(numbers: &[u32], min: u32, max: u32) -> ScaleSet {
    let mut set = new_set(
        "re",
        &spec(),
        &Scaling {
            min,
            max,
            ..Scaling::default()
        },
    );
    for number in numbers {
        set.instances.insert(
            *number,
            Instance {
                container_id: format!("c{number}"),
                note: String::new(),
            },
        );
    }
    set
}

#[test]
fn a_set_name_is_a_short_slug() {
    for good in ["re", "unity-dev", "a1"] {
        set_name(good).unwrap();
    }
    for bad in ["", "-re", "re-", "Re", "re_1", &"a".repeat(41)] {
        assert!(set_name(bad).is_err(), "{bad:?}");
    }
    assert_eq!(name_of("Reverse engineering"), "reverse-engineering");
    assert_eq!(name_of("!!!"), "set");
}

#[test]
fn the_limits_hold_the_minimum_under_the_maximum() {
    limits(0, 1, 0).unwrap();
    limits(16, 16, crate::lifecycle::MAX_IDLE_MINUTES).unwrap();
    assert!(limits(1, 0, 0).is_err());
    assert!(limits(1, MAX_INSTANCES + 1, 0).is_err());
    assert!(limits(3, 2, 0).is_err());
    assert!(limits(1, 2, crate::lifecycle::MAX_IDLE_MINUTES + 1).is_err());
    // A request without them gets one instance kept and four at most.
    let scaling: Scaling = serde_json::from_value(json!({})).unwrap();
    assert_eq!((scaling.min, scaling.max), (1, 4));
    scaling.validate().unwrap();
    assert!(serde_json::from_value::<Scaling>(json!({"replicas": 2})).is_err());
}

#[test]
fn a_new_instance_takes_the_lowest_free_number_within_the_maximum() {
    assert_eq!(set_with(&[], 1, 4).free_number(), Some(1));
    assert_eq!(set_with(&[1, 2, 4], 1, 4).free_number(), Some(3));
    assert_eq!(set_with(&[1, 2, 3, 4], 1, 4).free_number(), None);
    // A lowered maximum leaves no number above it.
    assert_eq!(set_with(&[1], 1, 1).free_number(), None);
}

#[test]
fn the_set_keeps_its_lowest_numbers_up_to_its_minimum() {
    assert_eq!(set_with(&[1, 2, 3], 1, 4).kept(), [1]);
    assert_eq!(set_with(&[2, 3, 5], 2, 6).kept(), [2, 3]);
    assert!(set_with(&[1, 2], 0, 4).kept().is_empty());
}

#[test]
fn an_agent_deletes_neither_a_kept_instance_nor_below_the_minimum() {
    use crate::scale::instances::refusal;
    let set = set_with(&[1, 2, 3], 1, 4);
    assert!(refusal(&set, 2, true).is_none());
    assert!(refusal(&set, 1, true).unwrap().contains("keeps"));
    // The dashboard may delete instance 1 while the set has more than its minimum.
    assert!(refusal(&set, 1, false).is_none());
    assert!(refusal(&set, 9, false).unwrap().contains("no instance 9"));
    let at_minimum = set_with(&[1, 2], 2, 4);
    assert!(
        refusal(&at_minimum, 2, false)
            .unwrap()
            .contains("lower its minimum")
    );
}

#[test]
fn the_idle_delete_leaves_kept_instances_and_waits_for_its_minutes() {
    let mut set = set_with(&[1, 2], 1, 4);
    let long = Some(Duration::from_secs(3600));
    assert_eq!(idle_delete_due(&set, 2, long), None, "no idle delete");
    set.idle_delete_minutes = 30;
    assert_eq!(idle_delete_due(&set, 2, long), Some(30));
    assert_eq!(idle_delete_due(&set, 1, long), None, "kept");
    assert_eq!(
        idle_delete_due(&set, 2, Some(Duration::from_secs(60))),
        None
    );
    assert_eq!(idle_delete_due(&set, 2, None), None, "in use");
}

#[test]
fn the_template_leaves_out_the_name_identity_and_expiry() {
    let set = new_set("re", &spec(), &Scaling::default());
    let template = &set.template;
    assert_eq!(template.name, "re");
    assert_eq!(template.image, "thoric/custom:re");
    assert_eq!(
        (
            &template.hostname,
            &template.mac_address,
            &template.machine_id
        ),
        (&None, &None, &None)
    );
    assert_eq!(template.lifecycle.expires_at, None);
    assert_eq!(template.lifecycle.idle_stop_minutes, 30);
    assert_eq!(template.mounts, spec().mounts);
    assert_eq!(set.instance_name(3), "re-3");
}

#[test]
fn only_policies_other_than_shared_are_kept() {
    let scaling: Scaling = serde_json::from_value(json!({"volumes": {
        "/home/desktop/analysis": "shared",
        "/home/desktop/.config/chrome-agent": "new",
        "/opt/x": "first",
    }}))
    .unwrap();
    let set = new_set("re", &spec(), &scaling);
    assert_eq!(set.volumes.len(), 2);
    assert_eq!(set.policy("/home/desktop/analysis"), VolumePolicy::Shared);
    assert_eq!(set.policy("/opt/x"), VolumePolicy::First);
    assert_eq!(set.policy("/elsewhere"), VolumePolicy::Shared);
}

#[test]
fn a_container_knows_its_set_and_number() {
    let sets = [set_with(&[1, 3], 1, 4)];
    assert_eq!(membership(&sets, "c3"), json!({"set": "re", "number": 3}));
    assert_eq!(membership(&sets, "c2"), Value::Null);
    assert_eq!(instance_of(&sets, "c1").map(|(_, n)| n), Some(1));
}

#[test]
fn a_volume_the_template_shares_is_in_use_by_the_set() {
    let mut set = set_with(&[1], 1, 4);
    let sets = vec![set.clone()];
    assert!(mounting(&sets, &BTreeMap::new(), "analysis").is_some());
    assert!(mounting(&sets, &BTreeMap::new(), "other").is_none());
    // A folder whose instances get volumes of their own does not keep the template's.
    set.volumes
        .insert("/home/desktop/analysis".into(), VolumePolicy::New);
    assert!(mounting(&[set], &BTreeMap::new(), "analysis").is_none());
}

#[test]
fn the_summary_names_instances_without_the_templates_secrets() {
    let mut set = set_with(&[1, 2], 1, 4);
    set.instances.get_mut(&2).unwrap().note = "packet decoder".into();
    let records: Vec<Container> = Vec::new();
    let summary = summary(&set, &records);
    assert_eq!(summary["instances"][1]["note"], "packet decoder");
    assert_eq!(summary["instances"][0]["kept"], true);
    assert_eq!(summary["instances"][1]["creating"], true);
    assert!(summary.get("template").is_none());
    let template = Template {
        parameters: BTreeMap::from([("TOKEN".into(), "secret-value".into())]),
        pinned: None,
        proxy_password: Some("proxy-secret".into()),
    };
    let saved = serde_json::to_string(&set).unwrap();
    assert!(!saved.contains("secret-value") && !saved.contains("proxy-secret"));
    assert!(
        serde_json::to_string(&template)
            .unwrap()
            .contains("secret-value")
    );
}
