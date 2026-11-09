//! Tool profiles: every tool an endpoint can offer is of exactly one kind, so a new tool cannot
//! land in or out of `headless` or `desktop` unnoticed, and the interface names every profile.
use super::*;
use crate::desktop_mcp::scopes::{LIST_INSTANCES, Scope};
use std::collections::BTreeSet;

/// Every container tool an endpoint can offer: the scopes' tools, `get_job` and `list_instances`.
fn offered() -> BTreeSet<&'static str> {
    Scope::ALL
        .iter()
        .flat_map(|scope| scope.tools().iter().copied())
        .chain(["get_job", LIST_INSTANCES])
        .collect()
}

#[test]
fn every_tool_of_an_endpoint_is_of_exactly_one_kind() {
    let offered = offered();
    for name in &offered {
        assert!(
            kind(name).is_some(),
            "{name} has no kind in desktop_mcp/profiles.rs: say whether it needs the screen and whether it is everyday, so every profile knows whether it offers it"
        );
    }
    let mut seen = BTreeSet::new();
    for (_, names) in KINDS {
        for name in names {
            assert!(seen.insert(*name), "{name} is of two kinds");
            assert!(
                offered.contains(name),
                "{name} has a kind but no scope offers it"
            );
        }
    }
}

#[test]
fn full_offers_everything_and_the_others_what_their_rule_says() {
    let offered = offered();
    assert_eq!(
        Profile::Full.tools().into_iter().collect::<BTreeSet<_>>(),
        offered
    );
    for name in &offered {
        assert!(Profile::Full.includes(name));
    }
    // Headless: scripts, terminals, files, applications, processes and jobs, nothing of the
    // screen, sound or gamepad.
    for name in [
        "exec_container",
        "open_terminal",
        "terminal_output",
        "read_file",
        "search_files",
        "write_file",
        "launch_application",
        "list_processes",
        "kill_process",
        "get_job",
        "container_action",
        "list_instances",
        "create_instance",
    ] {
        assert!(Profile::Headless.includes(name), "{name}");
    }
    for name in [
        "screenshot",
        "save_screenshot",
        "desktop_input",
        "act_and_observe",
        "list_windows",
        "focus_window",
        "set_window_geometry",
        "inspect_ui",
        "find_text",
        "get_clipboard",
        "set_clipboard",
        "gamepad_input",
        "start_recording",
        "capture_audio",
        "inject_audio",
        "export_artifacts",
    ] {
        assert!(!Profile::Headless.includes(name), "{name}");
    }
    // Desktop: the screen and input, programs, files and scripts, without the deeper tools.
    for name in [
        "screenshot",
        "desktop_input",
        "act_and_observe",
        "find_text",
        "launch_application",
        "read_file",
        "search_files",
        "exec_container",
        "get_job",
        "request_human",
    ] {
        assert!(Profile::Desktop.includes(name), "{name}");
    }
    for name in [
        "open_terminal",
        "container_diagnostics",
        "sample_metrics",
        "gamepad_input",
        "start_recording",
        "inspect_ui",
        "get_changes",
    ] {
        assert!(!Profile::Desktop.includes(name), "{name}");
    }
    // Both narrow profiles are smaller than full and differ from each other.
    let headless = Profile::Headless.tools();
    let desktop = Profile::Desktop.tools();
    assert!(headless.len() < offered.len() && desktop.len() < offered.len());
    assert_ne!(headless, desktop);
}

#[test]
fn a_name_no_rule_knows_is_offered_by_full_alone() {
    assert!(Profile::Full.includes("a_tool_of_tomorrow"));
    assert!(!Profile::Headless.includes("a_tool_of_tomorrow"));
    assert!(!Profile::Desktop.includes("a_tool_of_tomorrow"));
}

#[test]
fn a_profile_reads_and_writes_as_its_id() {
    for profile in Profile::ALL {
        assert_eq!(
            serde_json::to_value(profile).unwrap(),
            serde_json::json!(profile.id())
        );
        assert_eq!(
            serde_json::from_value::<Profile>(serde_json::json!(profile.id())).unwrap(),
            profile
        );
    }
    assert!(serde_json::from_value::<Profile>(serde_json::json!("everything")).is_err());
    assert_eq!(Profile::default(), Profile::Full);
}

/// The dashboard names and explains every profile, in the catalog every other one follows, and
/// lists them in the forms' order.
#[test]
fn the_interface_names_every_profile() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src");
    let catalog = std::fs::read_to_string(format!("{root}/locales/en.ts")).unwrap();
    let helpers = std::fs::read_to_string(format!("{root}/lib/mcp.ts")).unwrap();
    for profile in Profile::ALL {
        let id = profile.id();
        for key in [
            format!("\"mcp.profile.{id}\""),
            format!("\"mcp.profile.{id}.description\""),
        ] {
            assert!(catalog.contains(&key), "{key} is missing in locales/en.ts");
        }
    }
    let listed = Profile::ALL
        .iter()
        .map(|profile| format!("\"{}\"", profile.id()))
        .collect::<Vec<_>>()
        .join(", ");
    assert!(
        helpers.contains(&format!("PROFILES: McpProfile[] = [{listed}]")),
        "PROFILES in lib/mcp.ts does not list {listed}"
    );
}
