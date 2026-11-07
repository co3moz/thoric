//! Pins down what a restore refuses before it creates anything: a manifest or records that are
//! not a backup this release wrote, a scale set or an endpoint that reaches beyond the backup, and
//! any name this installation already uses, whether in its records or in Docker. A volume that is
//! merely missing is reported, not refused.
use super::*;
use crate::backup::{
    manifest::{Included, Installation, VolumeFacts, plan},
    tests::{container, endpoint, image, pinned, scale_set, set_endpoint, set_template},
};
use std::collections::BTreeMap;

fn backup() -> (Manifest, Records) {
    let installation = Installation {
        containers: vec![
            container("c1", "Desk", "thoric/custom:i1", &["projects", "absent"]),
            container("c2", "re-1", "thoric/custom:i1", &["projects"]),
        ],
        images: vec![image("i1", "Tools", "build")],
        composes: vec![("c1".into(), pinned())],
        endpoints: vec![endpoint("desk", &["c1"]), set_endpoint("re", "re")],
        proxy_passwords: Vec::new(),
        scale_sets: vec![scale_set("re", "thoric/custom:i1", &[(1, "c2")])],
        scale_set_templates: vec![("re".into(), set_template())],
        available_images: HashSet::from(["i1".into()]),
        with_files: HashSet::from(["c1".into()]),
        volumes: BTreeMap::from([
            ("projects".into(), VolumeFacts::default()),
            ("shared".into(), VolumeFacts::default()),
        ]),
    };
    let included = Included {
        images: true,
        volumes: true,
        files: true,
        secrets: false,
    };
    plan(installation, included, "0.7.0", 1)
}

fn refused(manifest: &Manifest) -> String {
    format!("{:#}", check_manifest(manifest).unwrap_err())
}

#[test]
fn only_a_backup_this_release_reads_is_accepted() {
    let (manifest, records) = backup();
    check_manifest(&manifest).unwrap();
    check_records(&manifest, &records).unwrap();

    let mut other = manifest.clone();
    other.kind = "something-else".into();
    assert_eq!(refused(&other), "The file is not a Thoric backup");
    let mut newer = manifest.clone();
    newer.format = FORMAT + 1;
    assert!(refused(&newer).contains("a layout this release does not read"));
    assert!(parse_manifest(b"{\"name\":\"an image archive\"}").is_err());
}

#[test]
fn names_that_reach_docker_must_be_ones_the_manager_gives() {
    let (manifest, _) = backup();
    for id in ["", "UPPER", "-dash", "a/b", "a b", &"x".repeat(65)] {
        let mut bad = manifest.clone();
        bad.containers[0].id = id.into();
        assert!(refused(&bad).contains("invalid container id"), "{id:?}");
    }
    let mut bad = manifest.clone();
    bad.volumes[0].name = "../etc".into();
    assert!(check_manifest(&bad).is_err());
    let mut bad = manifest.clone();
    bad.endpoints[0] = "Not An Endpoint".into();
    assert!(check_manifest(&bad).is_err());
    let mut bad = manifest.clone();
    bad.volumes[0].owner = Some("Other Owner".into());
    assert!(refused(&bad).contains("invalid owner labels"));
}

#[test]
fn the_parts_must_be_exactly_the_ones_the_manifest_says_it_carries() {
    let (manifest, _) = backup();
    let mut missing = manifest.clone();
    missing.parts.pop();
    assert!(refused(&missing).contains("parts do not match"));
    let mut twice = manifest.clone();
    twice.parts.push(twice.parts[0].clone());
    assert!(refused(&twice).contains("parts do not match"));
    let mut uncarried = manifest.clone();
    uncarried.images[0].image = false;
    assert!(refused(&uncarried).contains("parts do not match"));
}

#[test]
fn the_records_must_match_the_manifest_and_the_rules_of_their_routes() {
    let (manifest, records) = backup();
    let mut extra = records.clone();
    extra.containers.push(container("c9", "Stray", "x", &[]));
    assert!(
        check_records(&manifest, &extra)
            .unwrap_err()
            .to_string()
            .contains("containers do not match")
    );
    let mut stray = records.clone();
    stray.endpoints[0].grants[0].container_id = "elsewhere".into();
    assert!(
        check_records(&manifest, &stray)
            .unwrap_err()
            .to_string()
            .contains("names a container the backup does not hold")
    );
    let mut invalid = records.clone();
    invalid.containers[0].spec.resolution = "huge".into();
    assert!(check_records(&manifest, &invalid).is_err());
    let mut hash = records.clone();
    hash.endpoints[0].token_hash = Some("not-a-hash".into());
    assert!(check_records(&manifest, &hash).is_err());
    let mut orphan = records;
    orphan.composes[0].container_id = "elsewhere".into();
    assert!(check_records(&manifest, &orphan).is_err());
}

#[test]
fn a_scale_set_holds_containers_of_the_backup_and_its_endpoint_covers_it_alone() {
    let (manifest, records) = backup();
    check_records(&manifest, &records).unwrap();
    let refusal =
        |records: &Records| format!("{:#}", check_records(&manifest, records).unwrap_err());

    // Its instances are containers the backup holds, each of one set under one number.
    let mut stray = records.clone();
    stray.scale_sets[0]
        .instances
        .get_mut(&1)
        .unwrap()
        .container_id = "elsewhere".into();
    assert!(refusal(&stray).contains("names an instance the backup does not hold"));
    let mut twice = records.clone();
    let again = twice.scale_sets[0].instances[&1].clone();
    twice.scale_sets[0].instances.insert(2, again);
    assert!(refusal(&twice).contains("names an instance the backup does not hold"));
    let mut beyond = records.clone();
    let instance = beyond.scale_sets[0].instances.remove(&1).unwrap();
    beyond.scale_sets[0].instances.insert(9, instance);
    assert!(refusal(&beyond).contains("names an instance the backup does not hold"));
    let mut note = records.clone();
    note.scale_sets[0].instances.get_mut(&1).unwrap().note = "two\nlines".into();
    assert!(refusal(&note).contains("invalid instance note"));

    // Its limits and template follow the rules of the routes that make a set.
    let mut limits = records.clone();
    limits.scale_sets[0].min = 5;
    assert!(refusal(&limits).contains("minimum cannot exceed its maximum"));
    let mut template = records.clone();
    template.scale_sets[0].template.resolution = "huge".into();
    assert!(refusal(&template).starts_with("Scale set re"));
    let mut target = records.clone();
    target.scale_sets[0]
        .volumes
        .insert("../etc".into(), crate::scale::VolumePolicy::New);
    assert!(check_records(&manifest, &target).is_err());
    let mut password = records.clone();
    password.scale_set_templates[0].template.proxy_password = Some("two\nlines".into());
    assert!(check_records(&manifest, &password).is_err());
    let mut unknown = records.clone();
    unknown.scale_set_templates[0].set = "elsewhere".into();
    assert!(refusal(&unknown).contains("template of an unknown scale set"));

    // A grant of a set covers a set of the backup, and is its endpoint's only one.
    let set = records
        .endpoints
        .iter()
        .position(|e| e.name == "re")
        .unwrap();
    let mut other_set = records.clone();
    other_set.endpoints[set].grants[0].set = Some("other".into());
    assert!(refusal(&other_set).contains("names a scale set the backup does not hold"));
    let mut both = records.clone();
    both.endpoints[set].grants[0].container_id = "c1".into();
    assert!(refusal(&both).contains("names a scale set the backup does not hold"));
    let mut beside = records.clone();
    let desk = beside.endpoints[1 - set].grants[0].clone();
    beside.endpoints[set].grants.push(desk);
    assert!(refusal(&beside).contains("names a scale set the backup does not hold"));
    // Only a set's grant has the scale scope.
    let mut scaling = records.clone();
    scaling.endpoints[1 - set].grants[0].scopes = records.endpoints[set].grants[0].scopes.clone();
    assert!(refusal(&scaling).contains("gives a container the scale scope"));

    // The records and the manifest name the same sets.
    let mut missing = records;
    missing.scale_sets.clear();
    assert!(refusal(&missing).contains("scale sets do not match"));
}

#[test]
fn the_manifest_names_scale_sets_the_manager_would_name() {
    let (manifest, _) = backup();
    let mut bad = manifest.clone();
    bad.scale_sets[0] = "Not A Set".into();
    assert!(check_manifest(&bad).is_err());
    let mut twice = manifest.clone();
    twice.scale_sets.push("re".into());
    assert!(refused(&twice).contains("names the scale set re twice"));
    let mut unknown = manifest;
    unknown.scale_sets_secrets_left_out = vec!["other".into()];
    assert!(refused(&unknown).contains("unknown scale set"));
}

#[test]
fn a_restore_takes_only_names_nobody_uses() {
    let (manifest, _) = backup();
    let clear = review(&manifest, &Present::default());
    assert!(clear.conflicts.is_empty());
    assert_eq!(clear.missing_volumes, ["absent"]);

    let conflict = |present: Present| review(&manifest, &present).conflicts;
    let container = Conflict {
        kind: "container",
        name: "Desk".into(),
    };
    // Its record, its pinned compose, its Docker container or the one set aside, its networks and
    // the tag of its files all belong to the container.
    for present in [
        Present {
            containers: HashSet::from(["c1".into()]),
            ..Default::default()
        },
        Present {
            composes: HashSet::from(["c1".into()]),
            ..Default::default()
        },
        Present {
            docker_containers: HashSet::from(["thoric-c1".into()]),
            ..Default::default()
        },
        Present {
            docker_containers: HashSet::from(["thoric-c1-previous".into()]),
            ..Default::default()
        },
        Present {
            networks: HashSet::from(["thoric-c1-egress".into()]),
            ..Default::default()
        },
        Present {
            tags: HashSet::from(["thoric/container:restored-c1".into()]),
            ..Default::default()
        },
    ] {
        assert_eq!(conflict(present), std::slice::from_ref(&container));
    }
    let image = Conflict {
        kind: "image",
        name: "Tools".into(),
    };
    assert_eq!(
        conflict(Present {
            images: HashSet::from(["i1".into()]),
            ..Default::default()
        }),
        std::slice::from_ref(&image)
    );
    assert_eq!(
        conflict(Present {
            tags: HashSet::from(["thoric/custom:i1".into()]),
            ..Default::default()
        }),
        [image]
    );
    assert_eq!(
        conflict(Present {
            endpoints: HashSet::from(["desk".into()]),
            ..Default::default()
        }),
        [Conflict {
            kind: "endpoint",
            name: "desk".into()
        }]
    );
    // A set's record or its template is the set's.
    let set = Conflict {
        kind: "scale-set",
        name: "re".into(),
    };
    assert_eq!(
        conflict(Present {
            scale_sets: HashSet::from(["re".into()]),
            ..Default::default()
        }),
        std::slice::from_ref(&set)
    );
    assert!(conflict_message(&[set]).contains("would create: scale set re."));
    // A carried volume must be new here; one it only names may already be here.
    let present = Present {
        volumes: HashSet::from(["projects".into(), "absent".into()]),
        ..Default::default()
    };
    let reviewed = review(&manifest, &present);
    assert_eq!(
        reviewed.conflicts,
        [Conflict {
            kind: "volume",
            name: "projects".into()
        }]
    );
    assert!(reviewed.missing_volumes.is_empty());
    assert!(conflict_message(&reviewed.conflicts).contains("volume projects"));
}

/// The restore dialog labels every kind of conflict a review names, so a kind added here without
/// its label there fails here, on the side that adds it.
#[test]
fn the_dashboard_labels_every_kind_of_conflict() {
    let (manifest, _) = backup();
    let taken = |names: &[&str]| names.iter().map(|name| name.to_string()).collect();
    let everything = Present {
        containers: taken(&["c1"]),
        images: taken(&["i1"]),
        endpoints: taken(&["desk"]),
        scale_sets: taken(&["re"]),
        volumes: taken(&["projects"]),
        ..Default::default()
    };
    let kinds = review(&manifest, &everything)
        .conflicts
        .iter()
        .map(|conflict| conflict.kind)
        .collect::<BTreeSet<_>>();
    assert_eq!(kinds.len(), 5, "{kinds:?}");
    let dialog = include_str!("../../../web/src/components/dialogs/restore-dialog.tsx");
    for kind in kinds {
        let key = if kind.contains('-') {
            format!("\"{kind}\": \"backup.conflict.")
        } else {
            format!("{kind}: \"backup.conflict.")
        };
        assert!(
            dialog.contains(&key),
            "core/web/src/components/dialogs/restore-dialog.tsx must label the conflict kind {kind}"
        );
    }
}

#[test]
fn restored_images_go_by_thorics_own_tags() {
    assert_eq!(image_reference("build", "i1"), "thoric/custom:i1");
    assert_eq!(image_reference("snapshot", "i1"), "thoric/snapshot:i1");
    assert_eq!(image_reference("archive", "i1"), "thoric/imported:i1");
    // One registered from the other host's own tag gets no name from that host.
    assert_eq!(image_reference("import", "i1"), "thoric/imported:i1");
    assert_eq!(image_kind("import"), "archive");
    assert_eq!(image_kind("snapshot"), "snapshot");
    assert_eq!(files_reference("c1"), "thoric/container:restored-c1");
    assert!(record_id("0123abcd-ef"));
}
