//! Pins down what a backup carries: every record of the included kinds and none of the others,
//! the parts that were asked for and that Docker still has, and the secret values only when they
//! were asked for. Every kind of record the manager writes has to be on one of the two lists.
use super::*;
use crate::backup::tests::{
    SECRET, SET_PROXY_SECRET, container, endpoint, image, pinned, scale_set, set_endpoint,
    set_template,
};

/// The database kinds a backup carries, one for each field of `Records`.
const INCLUDED_KINDS: [&str; 7] = [
    crate::model::CONTAINERS,
    "image",
    crate::compose::PINNED,
    crate::desktop_mcp::ENDPOINTS,
    crate::outbound::CREDENTIALS,
    crate::scale::SETS,
    crate::scale::TEMPLATES,
];
/// The kinds a backup leaves out on purpose: history with the requests kept to retry it, the
/// processes of running desktops, and the tool lists the manager reads again from a desktop once
/// it runs.
const EXCLUDED_KINDS: [&str; 6] = [
    "job",
    crate::jobs::retry::REQUESTS,
    "application",
    "terminal",
    "recording",
    crate::desktop_mcp::SERVERS,
];

/// The password of a container's own proxy.
const PROXY_SECRET: &str = "proxy-password-of-c1";

fn proxy_password() -> crate::outbound::Credentials {
    crate::outbound::Credentials {
        password: PROXY_SECRET.into(),
    }
}

fn installation() -> Installation {
    Installation {
        containers: vec![
            container("c2", "Second", "thoric/snapshot:i1", &["shared"]),
            container(
                "c1",
                "First",
                "ghcr.io/co3moz/thoric/ubuntu-xfce:24.04",
                &["projects", "gone"],
            ),
        ],
        images: vec![
            image("i1", "Snapshot", "snapshot"),
            image("i2", "Lost", "build"),
        ],
        composes: vec![
            ("c1".into(), pinned()),
            // A compose whose container is gone is not the installation's any more.
            ("orphan".into(), pinned()),
        ],
        endpoints: vec![endpoint("desk", &["c1"]), set_endpoint("re", "re")],
        proxy_passwords: vec![
            ("c1".into(), proxy_password()),
            // A password whose container is gone is not the installation's any more.
            ("orphan".into(), proxy_password()),
        ],
        // Instance 2 is still being created: its number is taken, its record not saved yet.
        scale_sets: vec![scale_set(
            "re",
            "thoric/snapshot:i1",
            &[(1, "c1"), (2, "creating")],
        )],
        scale_set_templates: vec![
            ("re".into(), set_template()),
            // A template whose set is gone is not the installation's any more.
            ("gone".into(), set_template()),
        ],
        available_images: HashSet::from(["i1".into()]),
        with_files: HashSet::from(["c1".into()]),
        volumes: BTreeMap::from([
            ("projects".into(), VolumeFacts::default()),
            ("shared".into(), VolumeFacts::default()),
            (
                "thoric-c1-cache".into(),
                VolumeFacts {
                    owner: Some("c1".into()),
                    parameter: Some("CACHE".into()),
                },
            ),
        ]),
    }
}

const EVERYTHING: Included = Included {
    images: true,
    volumes: true,
    files: true,
    secrets: false,
};

#[test]
fn a_backup_carries_every_record_and_the_parts_docker_still_has() {
    let (manifest, records) = plan(installation(), EVERYTHING, "0.7.0", 99);
    assert_eq!((manifest.kind.as_str(), manifest.format), (KIND, FORMAT));
    assert_eq!(
        (manifest.version.as_str(), manifest.created_at),
        ("0.7.0", 99)
    );
    // Oldest first, as they were made.
    let ids = |records: &Records| {
        records
            .containers
            .iter()
            .map(|c| c.id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&records), ["c1", "c2"]);
    assert_eq!(records.images.len(), 2);
    assert_eq!(records.endpoints.len(), 2);
    assert_eq!(
        records
            .composes
            .iter()
            .map(|c| c.container_id.as_str())
            .collect::<Vec<_>>(),
        ["c1"]
    );
    // An image Docker lost is carried as a record alone, a container without files likewise.
    assert_eq!(
        manifest
            .images
            .iter()
            .map(|i| (i.id.as_str(), i.image))
            .collect::<Vec<_>>(),
        [("i1", true), ("i2", false)]
    );
    assert_eq!(
        manifest
            .containers
            .iter()
            .map(|c| (c.id.as_str(), c.files))
            .collect::<Vec<_>>(),
        [("c1", true), ("c2", false)]
    );
    assert_eq!(
        manifest.parts,
        [
            Part {
                kind: PartKind::Image,
                target: "i1".into()
            },
            Part {
                kind: PartKind::Files,
                target: "c1".into()
            },
            Part {
                kind: PartKind::Volume,
                target: "projects".into()
            },
            Part {
                kind: PartKind::Volume,
                target: "shared".into()
            },
            Part {
                kind: PartKind::Volume,
                target: "thoric-c1-cache".into()
            },
        ]
    );
    // A volume a container mounts that Docker no longer has is named, not carried.
    let volumes = manifest
        .volumes
        .iter()
        .map(|v| (v.name.as_str(), v.included, v.owner.as_deref()))
        .collect::<Vec<_>>();
    assert_eq!(
        volumes,
        [
            ("gone", false, None),
            ("projects", true, None),
            ("shared", true, None),
            ("thoric-c1-cache", true, Some("c1")),
        ]
    );
    assert_eq!(manifest.endpoints, ["desk", "re"]);
    assert_eq!(manifest.scale_sets, ["re"]);
    crate::backup::review::check_manifest(&manifest).unwrap();
    crate::backup::review::check_records(&manifest, &records).unwrap();
}

#[test]
fn a_backup_carries_the_scale_sets_with_their_templates_and_endpoints() {
    let (manifest, records) = plan(
        installation(),
        Included {
            secrets: true,
            ..EVERYTHING
        },
        "0.9.3",
        99,
    );
    let [set] = &records.scale_sets[..] else {
        panic!("{:?}", records.scale_sets);
    };
    // The instance still being created has no record to restore; its number is given up.
    assert_eq!(set.instances.keys().copied().collect::<Vec<_>>(), [1]);
    assert_eq!(set.instances[&1].container_id, "c1");
    assert_eq!(set.instances[&1].note, "an agent's");
    assert_eq!(set.template.image, "thoric/snapshot:i1");
    assert_eq!(set.endpoint.as_deref(), Some("re"));
    // Where its desktop servers run comes along.
    assert_eq!(set.runs_in("ghidra"), Some(1));
    // Only the template of a set the backup holds, with everything it keeps when asked for.
    let [template] = &records.scale_set_templates[..] else {
        panic!("{:?}", records.scale_set_templates);
    };
    assert_eq!(template.set, "re");
    assert_eq!(template.template.parameters["PASSWORD"], SECRET);
    assert_eq!(
        template.template.pinned.as_ref().unwrap().1.values["PASSWORD"],
        SECRET
    );
    assert_eq!(
        template.template.proxy_password.as_deref(),
        Some(SET_PROXY_SECRET)
    );
    assert!(manifest.scale_sets_secrets_left_out.is_empty());
    // The set's endpoint comes along with its grant and token hash.
    let re = records.endpoints.iter().find(|e| e.name == "re").unwrap();
    assert_eq!(re.set().map(|(_, set)| set), Some("re"));
    assert_eq!(re.token_hash.as_deref(), Some("cd".repeat(32).as_str()));
    crate::backup::review::check_records(&manifest, &records).unwrap();
}

#[test]
fn a_volume_only_a_sets_template_mounts_is_named_and_carried() {
    let mut installation = installation();
    // No instance is left, but the next one mounts the template's volume.
    let mut idle = scale_set("idle", "thoric/snapshot:i1", &[]);
    idle.template.mounts[0].volume = "set-only".into();
    // A volume each instance gets anew is no volume of the template's.
    let mut fresh = scale_set("fresh", "thoric/snapshot:i1", &[]);
    fresh.template.mounts[0].volume = "never-mounted".into();
    fresh.volumes = BTreeMap::from([(
        "/home/desktop/shared".into(),
        crate::scale::VolumePolicy::New,
    )]);
    installation.scale_sets.extend([idle, fresh]);
    installation
        .volumes
        .insert("set-only".into(), VolumeFacts::default());
    let (manifest, _) = plan(installation, EVERYTHING, "0.9.3", 99);
    let named = manifest
        .volumes
        .iter()
        .map(|v| (v.name.as_str(), v.included))
        .collect::<Vec<_>>();
    assert!(named.contains(&("set-only", true)), "{named:?}");
    assert!(!named.iter().any(|(name, _)| *name == "never-mounted"));
}

#[test]
fn an_endpoint_keeps_no_grant_of_a_set_the_backup_does_not_hold() {
    let mut installation = installation();
    // Grants of a set that is gone: its own endpoint goes, another one stays without the grant.
    installation.endpoints.push(set_endpoint("old", "old"));
    let mut kept = set_endpoint("kept", "old");
    kept.automatic = false;
    installation.endpoints.push(kept);
    let (manifest, records) = plan(installation, EVERYTHING, "0.9.3", 99);
    assert_eq!(manifest.endpoints, ["desk", "kept", "re"]);
    let kept = records.endpoints.iter().find(|e| e.name == "kept").unwrap();
    assert!(kept.grants.is_empty());
    crate::backup::review::check_records(&manifest, &records).unwrap();
}

#[test]
fn a_sets_secret_values_leave_only_when_asked_for() {
    let (manifest, records) = plan(installation(), EVERYTHING, "0.9.3", 1);
    assert_eq!(manifest.scale_sets_secrets_left_out, ["re"]);
    let template = &records.scale_set_templates[0].template;
    assert_eq!(
        template.parameters,
        BTreeMap::from([("EMAIL".into(), "a@example.com".into())])
    );
    let pinned = &template.pinned.as_ref().unwrap().1;
    assert!(!pinned.values.contains_key("PASSWORD"));
    assert_eq!(template.proxy_password, None);
    let written = serde_json::to_string(&(&manifest, &records)).unwrap();
    assert!(!written.contains(SECRET) && !written.contains(SET_PROXY_SECRET));

    // A template no instance pinned yet is judged by the compose of its image's record, which
    // goes with the images part or not, and also takes a parameter of a step of several by its
    // name alone.
    let mut unpinned = installation();
    let mut tools = image("i1", "Tools", "build");
    tools.reference = "thoric/snapshot:i1".into();
    tools.compose = Some(
        serde_json::from_value(serde_json::json!({"steps": [
            {"id": "mail", "parameters": [{"name": "EMAIL"}], "script": "echo mail"},
            {"id": "vault", "parameters": [{"name": "TOKEN", "secret": true}], "script": "echo vault"},
        ]}))
        .unwrap(),
    );
    unpinned.images = vec![tools];
    unpinned.scale_set_templates = vec![(
        "re".into(),
        crate::scale::Template {
            parameters: BTreeMap::from([
                ("mail.EMAIL".into(), "a@example.com".into()),
                ("TOKEN".into(), SECRET.into()),
            ]),
            pinned: None,
            proxy_password: None,
        },
    )];
    let (manifest, records) = plan(unpinned, Included::default(), "0.9.3", 1);
    assert!(records.images.is_empty());
    assert_eq!(manifest.scale_sets_secrets_left_out, ["re"]);
    assert_eq!(
        records.scale_set_templates[0].template.parameters,
        BTreeMap::from([("mail.EMAIL".into(), "a@example.com".into())])
    );

    // Values for a compose neither the template nor its image names cannot be told apart.
    let mut unknown = installation();
    unknown.images.clear();
    unknown.scale_set_templates[0].1.pinned = None;
    let (manifest, records) = plan(unknown, EVERYTHING, "0.9.3", 1);
    assert_eq!(manifest.scale_sets_secrets_left_out, ["re"]);
    assert!(
        records.scale_set_templates[0]
            .template
            .parameters
            .is_empty()
    );
}

#[test]
fn the_optional_parts_stay_out_unless_asked_for() {
    let (manifest, records) = plan(installation(), Included::default(), "0.7.0", 1);
    assert!(manifest.parts.is_empty());
    // Without the images part, the saved images stay out altogether.
    assert!(records.images.is_empty() && manifest.images.is_empty());
    assert_eq!(records.containers.len(), 2);
    assert!(manifest.containers.iter().all(|c| !c.files));
    // The volumes the containers mount are still named, so a restore can say what is missing.
    let named = manifest
        .volumes
        .iter()
        .map(|v| (v.name.as_str(), v.included))
        .collect::<Vec<_>>();
    assert_eq!(
        named,
        [("gone", false), ("projects", false), ("shared", false)]
    );
    crate::backup::review::check_manifest(&manifest).unwrap();
}

#[test]
fn secret_values_leave_only_when_asked_for() {
    let (manifest, records) = plan(installation(), EVERYTHING, "0.7.0", 1);
    let values = &records.composes[0].pinned.values;
    assert_eq!(
        values.get("EMAIL").map(String::as_str),
        Some("a@example.com")
    );
    assert!(!values.contains_key("PASSWORD"));
    assert_eq!(manifest.secrets_left_out, ["c1"]);
    let written = serde_json::to_string(&(&manifest, &records)).unwrap();
    assert!(!written.contains(SECRET));
    assert!(!written.contains(PROXY_SECRET));
    assert!(records.proxy_passwords.is_empty());
    // The names of secret parameters stay on the container, as they do in every list.
    assert_eq!(
        records.containers[0].setup.as_ref().unwrap().secrets,
        ["PASSWORD"]
    );

    let (manifest, records) = plan(
        installation(),
        Included {
            secrets: true,
            ..EVERYTHING
        },
        "0.7.0",
        1,
    );
    assert!(manifest.secrets_left_out.is_empty());
    assert_eq!(records.composes[0].pinned.values["PASSWORD"], SECRET);
    // The orphan's password stays behind with its container.
    assert_eq!(
        records.proxy_passwords,
        [super::ProxyPassword {
            container_id: "c1".into(),
            credentials: proxy_password(),
        }]
    );
    // The manifest, which the browser reads to preview a restore, never holds one.
    let manifest_text = serde_json::to_string(&manifest).unwrap();
    assert!(!manifest_text.contains(SECRET) && !manifest_text.contains(PROXY_SECRET));
    // Token hashes always come along, so the endpoints' tokens keep working.
    assert_eq!(
        records.endpoints[0].token_hash.as_deref(),
        Some("ab".repeat(32).as_str())
    );
}

#[test]
fn the_records_round_trip_through_their_json() {
    let (manifest, records) = plan(
        installation(),
        Included {
            secrets: true,
            ..EVERYTHING
        },
        "0.7.0",
        1,
    );
    let read: Manifest = serde_json::from_slice(&serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert_eq!(read, manifest);
    let read: Records = serde_json::from_slice(&serde_json::to_vec(&records).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(&read).unwrap(),
        serde_json::to_value(&records).unwrap()
    );
    assert_eq!(read.composes[0].pinned, records.composes[0].pinned);
}

/// The records file has one list for each included kind, so a kind added to one side without the
/// other shows here.
#[test]
fn the_included_kinds_are_the_fields_of_the_records() {
    let fields = serde_json::to_value(Records::default()).unwrap();
    let mut fields = fields
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    fields.sort();
    assert_eq!(
        fields,
        [
            "composes",
            "containers",
            "endpoints",
            "images",
            "proxy_passwords",
            "scale_set_templates",
            "scale_sets"
        ]
    );
    assert_eq!(INCLUDED_KINDS.len(), fields.len());
}

/// Every kind of record the manager writes was decided on: carried by a backup, or left out on
/// purpose. A new kind, such as a record of agents' activity, fails here until someone decides.
#[test]
fn every_record_kind_is_either_carried_or_left_out() {
    let constants = [
        ("CONTAINERS", crate::model::CONTAINERS),
        ("PINNED", crate::compose::PINNED),
        ("ENDPOINTS", crate::desktop_mcp::ENDPOINTS),
        ("SERVERS", crate::desktop_mcp::SERVERS),
        ("REQUESTS", crate::jobs::retry::REQUESTS),
        ("CREDENTIALS", crate::outbound::CREDENTIALS),
        ("SETS", crate::scale::SETS),
        ("TEMPLATES", crate::scale::TEMPLATES),
    ];
    let put = regex::Regex::new(r"\bdb\s*\.\s*put(?:::<[^>]*>)?\s*\(\s*([^,]+?)\s*,").unwrap();
    let mut found = BTreeSet::new();
    let mut stack = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(path) = stack.pop() {
        if path.is_dir() {
            stack.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".rs") || name.ends_with("_tests.rs") || name == "tests.rs" {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        for capture in put.captures_iter(&source) {
            let argument = capture[1].trim();
            let kind = match argument.strip_prefix('"') {
                Some(literal) => literal.trim_end_matches('"').to_owned(),
                None => {
                    let constant = argument.rsplit("::").next().unwrap();
                    constants
                        .iter()
                        .find(|(name, _)| *name == constant)
                        .unwrap_or_else(|| {
                            panic!(
                                "{}: decide whether backups carry {argument}",
                                path.display()
                            )
                        })
                        .1
                        .to_owned()
                }
            };
            found.insert(kind);
        }
    }
    assert!(
        found.contains("environment") && found.contains("job"),
        "{found:?}"
    );
    for kind in &found {
        assert!(
            INCLUDED_KINDS.contains(&kind.as_str()) || EXCLUDED_KINDS.contains(&kind.as_str()),
            "the manager writes {kind} records; decide whether a backup carries them"
        );
    }
}
