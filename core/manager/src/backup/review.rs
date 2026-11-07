//! Checking a backup before anything is restored. A backup is an upload like any other, so its
//! manifest and records are held to the rules the routes that create the same things apply; and
//! what it would create is compared with what this installation already has. A restore adds to
//! an installation and never replaces anything in it: one name it would take that is already
//! taken refuses the whole restore before anything is created, so no backup is half restored
//! over items of the same name.
use super::manifest::{FORMAT, KIND, Manifest, PartKind, Records};
use crate::{
    desktop_mcp::endpoint_name,
    model::{build_scripts, image_ref, mount_target, template_id, volume_name},
};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::collections::{BTreeSet, HashSet};

/// Record ids name Docker containers, networks and tags, so they keep to lowercase letters,
/// digits and dashes, as the ids the manager gives them do.
pub(crate) fn record_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id.as_bytes()[0].is_ascii_alphanumeric()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The tag a restored saved image gets: Thoric's own, after its record, whatever tag it had.
/// No other name is ever applied, so a backup cannot put an image under a catalog reference or
/// any other name this installation uses.
pub(crate) fn image_reference(kind: &str, id: &str) -> String {
    let repository = match kind {
        "build" => "custom",
        "snapshot" => "snapshot",
        _ => "imported",
    };
    format!("thoric/{repository}:{id}")
}

/// What an image record's kind becomes: an image registered from the host it came from was there
/// before Thoric, but here the restore put it, so it goes like an imported archive.
pub(crate) fn image_kind(kind: &str) -> &str {
    if kind == "import" { "archive" } else { kind }
}

/// The tag of the commit that carries a restored container's files.
pub(crate) fn files_reference(container: &str) -> String {
    format!("thoric/container:restored-{container}")
}

/// The Docker names a container takes: its container, the one a settings change sets aside, and
/// its two networks.
pub(crate) fn docker_names(container: &str) -> [String; 4] {
    [
        format!("thoric-{container}"),
        format!("thoric-{container}-previous"),
        format!("thoric-{container}-private"),
        format!("thoric-{container}-egress"),
    ]
}

/// Check the manifest on its own: that it is a backup this release reads, that its names are
/// ones the manager would give, and that its parts are the ones it says it carries.
pub(crate) fn check_manifest(manifest: &Manifest) -> Result<()> {
    if manifest.kind != KIND {
        bail!("The file is not a Thoric backup");
    }
    if manifest.format != FORMAT {
        bail!(
            "The backup was written by Thoric {} in a layout this release does not read; restore it with that release or a newer one",
            manifest.version
        );
    }
    if manifest.parts.len() > super::format::PART_LIMIT {
        bail!("The backup has too many parts");
    }
    let mut containers = HashSet::new();
    for container in &manifest.containers {
        if !record_id(&container.id) || !containers.insert(container.id.as_str()) {
            bail!(
                "The backup names an invalid container id {:?}",
                container.id
            );
        }
    }
    let mut images = HashSet::new();
    for image in &manifest.images {
        if !record_id(&image.id) || !images.insert(image.id.as_str()) {
            bail!("The backup names an invalid image id {:?}", image.id);
        }
    }
    let mut endpoints = HashSet::new();
    for name in &manifest.endpoints {
        endpoint_name(name)?;
        if !endpoints.insert(name.as_str()) {
            bail!("The backup names the endpoint {name} twice");
        }
    }
    let mut sets = HashSet::new();
    for name in &manifest.scale_sets {
        crate::scale::set_name(name)?;
        if !sets.insert(name.as_str()) {
            bail!("The backup names the scale set {name} twice");
        }
    }
    let mut volumes = HashSet::new();
    for volume in &manifest.volumes {
        volume_name(&volume.name)?;
        if !volumes.insert(volume.name.as_str()) {
            bail!("The backup names the volume {} twice", volume.name);
        }
        if volume.owner.as_ref().is_some_and(|owner| !record_id(owner))
            || volume
                .parameter
                .as_ref()
                .is_some_and(|p| p.is_empty() || p.len() > 128 || p.contains([',', '\n', '\0']))
        {
            bail!("The backup names invalid owner labels for {}", volume.name);
        }
    }
    let mut parts = HashSet::new();
    for part in &manifest.parts {
        let carried = match part.kind {
            PartKind::Image => manifest
                .images
                .iter()
                .any(|i| i.id == part.target && i.image),
            PartKind::Files => manifest
                .containers
                .iter()
                .any(|c| c.id == part.target && c.files),
            PartKind::Volume => manifest
                .volumes
                .iter()
                .any(|v| v.name == part.target && v.included),
        };
        if !carried || !parts.insert((part.kind, part.target.as_str())) {
            bail!("The backup's parts do not match its manifest");
        }
    }
    let expected = manifest.images.iter().filter(|i| i.image).count()
        + manifest.containers.iter().filter(|c| c.files).count()
        + manifest.volumes.iter().filter(|v| v.included).count();
    if expected != manifest.parts.len() {
        bail!("The backup's parts do not match its manifest");
    }
    if manifest
        .secrets_left_out
        .iter()
        .any(|id| !containers.contains(id.as_str()))
    {
        bail!("The backup's manifest names an unknown container");
    }
    if manifest
        .scale_sets_secrets_left_out
        .iter()
        .any(|name| !sets.contains(name.as_str()))
    {
        bail!("The backup's manifest names an unknown scale set");
    }
    Ok(())
}

/// Check the records against the manifest and against what the routes that make the same records
/// accept. Nothing in them is trusted for a Docker name: those come from the ids again.
pub(crate) fn check_records(manifest: &Manifest, records: &Records) -> Result<()> {
    let same = |a: BTreeSet<&str>, b: BTreeSet<&str>, what: &str| {
        if a != b {
            bail!("The backup's {what} do not match its manifest");
        }
        Ok(())
    };
    same(
        records.containers.iter().map(|c| c.id.as_str()).collect(),
        manifest.containers.iter().map(|c| c.id.as_str()).collect(),
        "containers",
    )?;
    same(
        records.images.iter().map(|i| i.id.as_str()).collect(),
        manifest.images.iter().map(|i| i.id.as_str()).collect(),
        "images",
    )?;
    same(
        records.endpoints.iter().map(|e| e.name.as_str()).collect(),
        manifest.endpoints.iter().map(String::as_str).collect(),
        "endpoints",
    )?;
    same(
        records.scale_sets.iter().map(|s| s.name.as_str()).collect(),
        manifest.scale_sets.iter().map(String::as_str).collect(),
        "scale sets",
    )?;
    if records.containers.len() != manifest.containers.len()
        || records.images.len() != manifest.images.len()
        || records.endpoints.len() != manifest.endpoints.len()
        || records.scale_sets.len() != manifest.scale_sets.len()
    {
        bail!("The backup holds a record twice");
    }
    for container in &records.containers {
        let named =
            |error: anyhow::Error| error.context(format!("Container {}", container.spec.name));
        container.spec.validate().map_err(named)?;
        for mount in container.compose_mounts() {
            volume_name(&mount.volume)
                .and_then(|_| mount_target(&mount.target))
                .map_err(named)?;
        }
    }
    for image in &records.images {
        let named = |error: anyhow::Error| error.context(format!("Image {}", image.name));
        if image.name.trim().is_empty()
            || image.name.chars().count() > 100
            || image.description.len() > 4096
        {
            bail!("The backup holds an image with an invalid name or description");
        }
        if let Some(compose) = &image.compose {
            compose.validate().map_err(named)?;
        }
        if let Some(icon) = &image.icon {
            icon.validate().map_err(named)?;
        }
        if let Some(build) = &image.build {
            image_ref(&build.base_image)
                .and_then(|_| build_scripts(&build.script, &build.parts))
                .and_then(|_| crate::desktop_mcp::check_declarations(&build.mcp))
                .map_err(named)?;
        }
        for id in &image.template_ids {
            template_id(id).map_err(named)?;
        }
    }
    let mut pinned = HashSet::new();
    for compose in &records.composes {
        if !pinned.insert(compose.container_id.as_str())
            || !manifest
                .containers
                .iter()
                .any(|c| c.id == compose.container_id)
        {
            bail!("The backup holds a compose of an unknown container");
        }
        compose.pinned.compose.validate()?;
    }
    let mut passwords = HashSet::new();
    for entry in &records.proxy_passwords {
        if !passwords.insert(entry.container_id.as_str())
            || !manifest
                .containers
                .iter()
                .any(|c| c.id == entry.container_id)
        {
            bail!("The backup holds a proxy password of an unknown container");
        }
        crate::outbound::password(&entry.credentials.password)?;
    }
    check_scale_sets(manifest, records)?;
    for endpoint in &records.endpoints {
        let mut aliases = HashSet::new();
        for grant in &endpoint.grants {
            let held = match &grant.set {
                // A set's grant is its endpoint's only one, as the endpoint routes make it.
                Some(set) => {
                    if !grant.container_id.is_empty()
                        || endpoint.grants.len() != 1
                        || !manifest.scale_sets.contains(set)
                    {
                        bail!(
                            "The endpoint {} names a scale set the backup does not hold",
                            endpoint.name
                        );
                    }
                    true
                }
                None => {
                    if grant.scales() {
                        bail!(
                            "The endpoint {} gives a container the scale scope, which is for a scale set",
                            endpoint.name
                        );
                    }
                    manifest
                        .containers
                        .iter()
                        .any(|c| c.id == grant.container_id)
                }
            };
            if !held
                || !crate::desktop_mcp::slug(&grant.alias, 24)
                || !aliases.insert(grant.alias.as_str())
            {
                bail!(
                    "The endpoint {} names a container the backup does not hold",
                    endpoint.name
                );
            }
        }
        if endpoint
            .token_hash
            .as_ref()
            .is_some_and(|hash| hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            bail!("The endpoint {} has an invalid token hash", endpoint.name);
        }
    }
    Ok(())
}

/// Check the scale sets and their templates: the limits and settings the set routes accept, and
/// instances that are containers of the backup, each of one set under one number.
fn check_scale_sets(manifest: &Manifest, records: &Records) -> Result<()> {
    let mut instances = HashSet::new();
    for set in &records.scale_sets {
        let named = |error: anyhow::Error| error.context(format!("Scale set {}", set.name));
        crate::scale::limits(set.min, set.max, set.idle_delete_minutes).map_err(named)?;
        set.template.validate().map_err(named)?;
        for target in set.volumes.keys() {
            mount_target(target).map_err(named)?;
        }
        if let Some(endpoint) = &set.endpoint {
            endpoint_name(endpoint).map_err(named)?;
        }
        for (number, instance) in &set.instances {
            if !(1..=set.max).contains(number)
                || !instances.insert(instance.container_id.as_str())
                || !manifest
                    .containers
                    .iter()
                    .any(|c| c.id == instance.container_id)
            {
                bail!(
                    "The scale set {} names an instance the backup does not hold",
                    set.name
                );
            }
            if instance.note.chars().count() > crate::scale::NOTE_LIMIT
                || instance.note.chars().any(char::is_control)
            {
                bail!("The scale set {} has an invalid instance note", set.name);
            }
        }
    }
    let mut templates = HashSet::new();
    for entry in &records.scale_set_templates {
        if !templates.insert(entry.set.as_str()) || !manifest.scale_sets.contains(&entry.set) {
            bail!("The backup holds a template of an unknown scale set");
        }
        let named = |error: anyhow::Error| error.context(format!("Scale set {}", entry.set));
        if let Some((_, pinned)) = &entry.template.pinned {
            pinned.compose.validate().map_err(named)?;
            for mount in pinned.mounts() {
                volume_name(&mount.volume)
                    .and_then(|_| mount_target(&mount.target))
                    .map_err(named)?;
            }
        }
        if let Some(password) = &entry.template.proxy_password {
            crate::outbound::password(password).map_err(named)?;
        }
    }
    Ok(())
}

/// What this installation has that a restore would collide with.
#[derive(Debug, Default)]
pub(crate) struct Present {
    /// Record ids and endpoint names in the database.
    pub containers: HashSet<String>,
    pub images: HashSet<String>,
    pub composes: HashSet<String>,
    pub endpoints: HashSet<String>,
    /// Scale sets, by the name of their record or their template's.
    pub scale_sets: HashSet<String>,
    /// Names Docker has.
    pub docker_containers: HashSet<String>,
    pub networks: HashSet<String>,
    pub volumes: HashSet<String>,
    /// Image tags as `repository:tag`.
    pub tags: HashSet<String>,
}

/// One name a restore would take that is taken already.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct Conflict {
    /// `container`, `image`, `endpoint`, `volume` or `scale-set`.
    pub kind: &'static str,
    /// What the backup calls it.
    pub name: String,
}

/// A backup compared with this installation.
#[derive(Debug, Default, PartialEq, Serialize)]
pub(crate) struct Review {
    pub conflicts: Vec<Conflict>,
    /// Volumes the containers mount that the backup does not carry and this installation lacks.
    pub missing_volumes: Vec<String>,
}

pub(crate) fn review(manifest: &Manifest, present: &Present) -> Review {
    let mut conflicts = BTreeSet::new();
    for container in &manifest.containers {
        let taken = present.containers.contains(&container.id)
            || present.composes.contains(&container.id)
            || present.tags.contains(&files_reference(&container.id))
            || docker_names(&container.id).iter().any(|name| {
                present.docker_containers.contains(name) || present.networks.contains(name)
            });
        if taken {
            conflicts.insert(Conflict {
                kind: "container",
                name: container.name.clone(),
            });
        }
    }
    for image in &manifest.images {
        // The kind decides the tag; any of them taken is a conflict, whichever it would be.
        let taken = present.images.contains(&image.id)
            || ["build", "snapshot", "archive"].iter().any(|kind| {
                image.image && present.tags.contains(&image_reference(kind, &image.id))
            });
        if taken {
            conflicts.insert(Conflict {
                kind: "image",
                name: image.name.clone(),
            });
        }
    }
    for name in &manifest.endpoints {
        if present.endpoints.contains(name) {
            conflicts.insert(Conflict {
                kind: "endpoint",
                name: name.clone(),
            });
        }
    }
    for name in &manifest.scale_sets {
        if present.scale_sets.contains(name) {
            conflicts.insert(Conflict {
                kind: "scale-set",
                name: name.clone(),
            });
        }
    }
    let mut missing_volumes = Vec::new();
    for volume in &manifest.volumes {
        let here = present.volumes.contains(&volume.name);
        if volume.included && here {
            conflicts.insert(Conflict {
                kind: "volume",
                name: volume.name.clone(),
            });
        } else if !volume.included && !here {
            missing_volumes.push(volume.name.clone());
        }
    }
    Review {
        conflicts: conflicts.into_iter().collect(),
        missing_volumes,
    }
}

/// The refusal a restore answers with when names are taken.
pub(crate) fn conflict_message(conflicts: &[Conflict]) -> String {
    let names = conflicts
        .iter()
        .map(|c| format!("{} {}", c.kind.replace('-', " "), c.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "This installation already has what the backup would create: {names}. Delete or rename them first, or restore into a new installation"
    )
}

/// The manifest a request or an upload sent, read with its limit.
pub(crate) fn parse_manifest(data: &[u8]) -> Result<Manifest> {
    let manifest: Manifest =
        serde_json::from_slice(data).context("The file is not a Thoric backup")?;
    check_manifest(&manifest)?;
    Ok(manifest)
}

#[cfg(test)]
#[path = "review_tests.rs"]
mod tests;
