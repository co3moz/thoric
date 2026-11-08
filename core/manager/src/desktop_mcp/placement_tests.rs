//! Where a server of a scale set's instance runs: in every instance by default, in instance 1
//! alone under `first`, and through one connection every endpoint shares there.
use super::*;
use crate::scale::{Instance, Scaling, new_set};

fn set(servers: &[(&str, ServerPolicy)]) -> ScaleSet {
    let spec: crate::model::ContainerSpec = serde_json::from_value(serde_json::json!({
        "name": "re-1",
        "image": "thoric/ubuntu-xfce:24.04",
    }))
    .unwrap();
    let mut set = new_set(
        "re",
        &spec,
        &Scaling {
            servers: servers
                .iter()
                .map(|(name, policy)| ((*name).to_owned(), *policy))
                .collect(),
            ..Scaling::default()
        },
    );
    for number in [1, 2] {
        set.instances.insert(
            number,
            Instance {
                container_id: format!("c{number}"),
                note: String::new(),
            },
        );
    }
    set
}

#[test]
fn a_server_runs_in_every_instance_unless_the_set_runs_it_in_the_first() {
    let sets = [set(&[("ghidra", ServerPolicy::First)])];
    assert_eq!(Placement::of(&sets, "c2", "browser"), Placement::Own);
    assert_eq!(Placement::of(&sets, "elsewhere", "ghidra"), Placement::Own);
    assert_eq!(
        Placement::of(&sets, "c1", "ghidra"),
        Placement::Home { set: "re".into() }
    );
    assert_eq!(
        Placement::of(&sets, "c2", "ghidra"),
        Placement::Elsewhere {
            set: "re".into(),
            home: 1
        }
    );
}

#[test]
fn every_endpoint_shares_the_connection_to_a_server_that_runs_once() {
    let home = Placement::Home { set: "re".into() };
    assert_eq!(home.connection("re"), SHARED);
    assert_eq!(home.connection("/mcp"), SHARED);
    assert_eq!(Placement::Own.connection("re"), "re");
    // No endpoint can be called what the shared connection is kept under.
    assert!(!crate::desktop_mcp::slug(SHARED, 64));
}

#[test]
fn another_instance_is_told_which_one_runs_the_server() {
    let elsewhere = Placement::Elsewhere {
        set: "re".into(),
        home: 1,
    };
    assert_eq!(elsewhere.home(), Some(1));
    let refusal = elsewhere.refusal("ghidra").unwrap();
    assert_eq!(crate::error::Error::from(refusal).0, StatusCode::CONFLICT);
    assert!(
        elsewhere
            .refusal("ghidra")
            .unwrap()
            .to_string()
            .contains("in instance 1 alone")
    );
    assert!(Placement::Own.refusal("ghidra").is_none());
    assert!(
        Placement::Home { set: "re".into() }
            .refusal("ghidra")
            .is_none()
    );
}

#[test]
fn a_set_keeps_only_the_servers_that_do_not_run_everywhere() {
    let pinned = set(&[
        ("ghidra", ServerPolicy::First),
        ("browser", ServerPolicy::Each),
    ]);
    assert_eq!(
        pinned.servers,
        BTreeMap::from([("ghidra".to_owned(), ServerPolicy::First)])
    );
    assert_eq!(pinned.runs_in("ghidra"), Some(1));
    assert_eq!(pinned.runs_in("browser"), None);
    let saved = serde_json::to_value(&pinned).unwrap();
    assert_eq!(saved["servers"], serde_json::json!({"ghidra": "first"}));
    let none = serde_json::to_value(set(&[])).unwrap();
    assert!(none.get("servers").is_none());
}

#[test]
fn only_the_servers_whose_policy_changed_are_moved() {
    let now = set(&[("ghidra", ServerPolicy::First)]);
    assert!(moved(&now.servers, &now).is_empty());
    assert_eq!(
        moved(&BTreeMap::new(), &now),
        BTreeSet::from(["ghidra".to_owned()])
    );
    let before = BTreeMap::from([
        ("ghidra".to_owned(), ServerPolicy::First),
        ("db".to_owned(), ServerPolicy::First),
    ]);
    assert_eq!(moved(&before, &now), BTreeSet::from(["db".to_owned()]));
}

#[test]
fn a_server_another_instance_runs_shows_as_elsewhere_unless_its_declaration_fails() {
    let sets = [set(&[("ghidra", ServerPolicy::First)])];
    let mut record = ServerRecord {
        container_id: "c2".into(),
        name: "ghidra".into(),
        source: super::super::registry::Source::Image,
        spec: None,
        tools: Vec::new(),
        tools_at: None,
        error: None,
    };
    assert_eq!(shown(&record, true, &sets), ("elsewhere", Some(1)));
    assert_eq!(shown(&record, false, &sets), ("elsewhere", Some(1)));
    record.error = Some("not JSON".into());
    assert_eq!(shown(&record, true, &sets), ("error", Some(1)));
    record.container_id = "c1".into();
    record.error = None;
    assert_eq!(shown(&record, true, &sets), ("pending", None));
}
