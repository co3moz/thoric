//! Pins down how a backup's records are moved to this installation: image IDs to the ones Docker
//! gave the loaded images here, tags to Thoric's own, Docker names made from the ids again,
//! the machine identity kept, and a reset still finding the image a container started from.
use super::*;
use crate::backup::{
    manifest::Records,
    tests::{container, image},
};

const CATALOG: &str = "ghcr.io/co3moz/thoric/ubuntu-xfce:24.04";

fn records() -> Records {
    let mut built = image("i1", "Tools", "build");
    built.reference = "thoric/custom:i1".into();
    let mut snapshot = image("i2", "Snap", "snapshot");
    snapshot.reference = "thoric/snapshot:i2".into();
    snapshot.parent_image_id = Some(built.image_id.clone());
    let mut registered = image("i3", "Host tag", "import");
    registered.reference = "registry.example/team/tool:1".into();
    registered.parent_image_id = Some("sha256:not-restored".into());

    let mut from_snapshot = container("c1", "Desk", "thoric/snapshot:i2", &[]);
    from_snapshot.source_image_id = snapshot.image_id.clone();
    // A record whose stored names point elsewhere gets them from its id again.
    from_snapshot.container = "thoric-manager".into();
    from_snapshot.private_network = "host".into();
    let mut from_catalog = container("c2", "Catalog", CATALOG, &[]);
    from_catalog.source_image_id = "sha256:catalog-elsewhere".into();
    let mut same_image = container("c3", "Same", CATALOG, &[]);
    same_image.source_image_id = "sha256:same-here".into();
    let mut by_id = container("c4", "By id", "sha256:image-i1", &[]);
    by_id.source_image_id = "sha256:image-i1".into();
    Records {
        containers: vec![from_snapshot, from_catalog, same_image, by_id],
        images: vec![built, snapshot, registered],
        composes: Vec::new(),
        endpoints: Vec::new(),
        proxy_passwords: Vec::new(),
    }
}

fn loaded() -> Loaded {
    Loaded {
        images: HashMap::from([
            ("i1".into(), "sha256:new-i1".into()),
            ("i2".into(), "sha256:new-i2".into()),
            ("i3".into(), "sha256:new-i3".into()),
        ]),
        files: HashMap::from([("c1".into(), "sha256:files-c1".into())]),
        resolved: HashMap::from([
            (CATALOG.into(), "sha256:catalog-here".into()),
            ("sha256:same-here".into(), "sha256:same-here".into()),
        ]),
    }
}

#[test]
fn restored_images_take_the_ids_docker_gave_them_here_and_their_own_tags() {
    let records = remap(records(), &loaded());
    let images = records
        .images
        .iter()
        .map(|i| (i.image_id.as_str(), i.reference.as_str(), i.kind.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        images,
        [
            ("sha256:new-i1", "thoric/custom:i1", "build"),
            ("sha256:new-i2", "thoric/snapshot:i2", "snapshot"),
            // The other host's tag is not applied here, and the image is the restore's own.
            ("sha256:new-i3", "thoric/imported:i3", "archive"),
        ]
    );
    // A parent that came along is found again; one that did not keeps its old ID.
    assert_eq!(
        records.images[1].parent_image_id.as_deref(),
        Some("sha256:new-i1")
    );
    assert_eq!(
        records.images[2].parent_image_id.as_deref(),
        Some("sha256:not-restored")
    );
}

#[test]
fn restored_containers_keep_their_identity_and_find_their_images_here() {
    let before = records();
    let records = remap(records(), &loaded());
    let [snapshot, catalog, same, by_id] = &records.containers[..] else {
        panic!("{:?}", records.containers);
    };
    assert_eq!(snapshot.container, "thoric-c1");
    assert_eq!(snapshot.private_network, "thoric-c1-private");
    assert_eq!(snapshot.egress_network, "thoric-c1-egress");
    assert_eq!(snapshot.spec.hostname, before.containers[0].spec.hostname);
    assert_eq!(
        snapshot.spec.mac_address,
        before.containers[0].spec.mac_address
    );
    assert_eq!(
        snapshot.spec.machine_id,
        before.containers[0].spec.machine_id
    );
    // It runs its files, and a reset starts from the restored snapshot by its new ID.
    assert_eq!(
        snapshot.working_image,
        Some(crate::model::WorkingImage {
            image_id: "sha256:files-c1".into(),
            reference: "thoric/container:restored-c1".into(),
        })
    );
    assert_eq!(snapshot.source_image_id, "sha256:new-i2");
    assert_eq!(snapshot.spec.image, "thoric/snapshot:i2");
    assert_eq!(snapshot.setup.as_ref().unwrap().status, "completed");
    // The catalog image it names, as this installation has it; the same image ID when it is here.
    assert_eq!(catalog.source_image_id, "sha256:catalog-here");
    assert_eq!(catalog.spec.image, CATALOG);
    assert_eq!(same.source_image_id, "sha256:same-here");
    assert_eq!(by_id.spec.image, "sha256:new-i1");
    assert_eq!(by_id.source_image_id, "sha256:new-i1");
    // Without its files a container starts over from its image, so its setup runs again.
    assert_eq!(catalog.working_image, None);
    assert_eq!(catalog.setup.as_ref().unwrap().status, "pending");
}

#[test]
fn a_container_waits_without_its_files_or_a_volume_it_mounts() {
    let records = remap(records(), &loaded());
    let volumes = HashSet::from(["projects".to_owned()]);
    assert_eq!(not_created(&records.containers[0], &volumes), None);
    assert!(
        not_created(&records.containers[1], &volumes)
            .unwrap()
            .starts_with("Restored without its files. Retry setup creates it")
    );
    let mut mounting = records.containers[0].clone();
    mounting.spec.mounts = vec![crate::model::Mount {
        volume: "elsewhere".into(),
        target: "/home/desktop/work".into(),
        readonly: false,
    }];
    assert!(
        not_created(&mounting, &volumes)
            .unwrap()
            .contains("the volume elsewhere is not on this installation")
    );
}

#[test]
fn restored_endpoints_come_without_their_tokens() {
    let mut records = records();
    records.endpoints = vec![crate::backup::tests::endpoint("desk", &["c1"])];
    assert!(records.endpoints[0].token_hash.is_some());
    let records = remap(records, &loaded());
    assert_eq!(records.endpoints[0].token_hash, None);
}

#[test]
fn what_this_installation_disables_is_switched_off() {
    let mut config = crate::config::Config::for_tests();
    let mut spec = container("c1", "Desk", CATALOG, &[]).spec;
    spec.internet = true;
    spec.sudo = true;
    spec.gamepad = true;
    spec.vpn = true;
    spec.debugger = true;
    spec.realtime = true;
    spec.profiler = true;
    spec.kvm = true;
    spec.audio = true;
    spec.microphone = true;
    spec.gpu = true;
    spec.proxy = Some(ProxyChoice::Installation);
    let allowed = |config: &mut crate::config::Config| {
        config.internet = true;
        config.sudo = true;
        config.gamepad = true;
        config.vpn = true;
        config.audio = true;
        config.microphone = true;
    };
    allowed(&mut config);
    config.proxy = crate::outbound::installation(
        &|name| (name == "PROXY_URL").then(|| "http://proxy.example:3128".to_owned()),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(config.proxy.is_some());
    let mut kept = spec.clone();
    assert!(switch_off_disabled(&config, &mut kept).is_empty());
    assert!(kept.gamepad && kept.vpn && kept.kvm && kept.proxy.is_some());

    config.internet = false;
    config.sudo = false;
    config.gamepad = false;
    config.vpn = false;
    config.debugger = false;
    config.realtime = false;
    config.profiler = false;
    config.kvm = false;
    config.audio = false;
    config.proxy = None;
    let off = switch_off_disabled(&config, &mut spec);
    assert_eq!(
        off,
        [
            "internet",
            "sudo",
            "gamepad",
            "vpn",
            "debugger",
            "realtime",
            "profiler",
            "kvm",
            "audio",
            "microphone",
            "proxy"
        ]
    );
    assert!(!spec.internet && !spec.sudo && !spec.gamepad && !spec.vpn);
    assert!(!spec.debugger && !spec.realtime && !spec.profiler && !spec.kvm);
    assert!(!spec.audio && !spec.microphone && spec.proxy.is_none());
    // The GPU stays asked for: the container then says why it was not created.
    assert!(spec.gpu);
    spec.validate().unwrap();
}

#[test]
fn volumes_volume_filter_hides_are_left_out_and_missing_ones_waited_for() {
    let mut desk = container(
        "c1",
        "Desk",
        CATALOG,
        &["hidden", "listed", "missing", "hidden"],
    );
    desk.working_image = Some(WorkingImage {
        image_id: "sha256:files".into(),
        reference: "thoric/container:restored-c1".into(),
    });
    desk.setup.as_mut().unwrap().mounts = vec![crate::model::Mount {
        volume: "hidden".into(),
        target: "/home/desktop/compose".into(),
        readonly: false,
    }];
    let present = HashSet::from(["listed".to_owned(), "hidden".to_owned()]);
    let listed = HashSet::from(["listed".to_owned()]);
    assert_eq!(leave_out_unlisted(&mut desk, &present, &listed), ["hidden"]);
    let volumes = desk
        .mounts()
        .into_iter()
        .map(|mount| mount.volume)
        .collect::<Vec<_>>();
    assert_eq!(volumes, ["listed", "missing"]);
    assert!(not_created(&desk, &present).is_some_and(|reason| reason.contains("missing")));
}
