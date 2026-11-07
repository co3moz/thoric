//! What a backup holds: the manifest that describes it, the records it carries, and the plan of
//! the parts that follow them, chosen from what the installation has.
//!
//! The records are an allowlist of database kinds, the fields of [`Records`]. Containers, saved
//! images with their icons and composes, each container's pinned compose with its values, the
//! passwords of the containers' own proxies and MCP endpoints with their token hashes are the
//! installation's state. Jobs with their logs, which are in a table of their own, the processes
//! of running desktops (applications, terminals, recordings), the tool lists of desktop MCP servers, which
//! the manager reads again from a running desktop, and any kind added later are history or what a
//! running manager rebuilds, and stay out until someone decides otherwise here. Scale sets stay out
//! too, with the endpoints of a set: a set's instances come back as containers of their own, which
//! can be made scalable again. The tests list
//! both sides and fail on a kind the code writes that is on neither.
use crate::{
    compose::Pinned,
    desktop_mcp::Endpoint,
    model::{Container, ImageRecord, Mount},
    outbound::Credentials,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// The manifest's own kind, so another JSON file is not taken for a backup.
pub(crate) const KIND: &str = "thoric-backup";
/// The layout this release writes and reads.
pub(crate) const FORMAT: u32 = 1;

/// The optional parts of a backup. The records are always in it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Included {
    /// The saved images: their records with their Docker images. Catalog images are left out;
    /// the installation they return to downloads them when it needs them.
    #[serde(default)]
    pub images: bool,
    /// The named volumes the containers mount and those Thoric created.
    #[serde(default)]
    pub volumes: bool,
    /// Each container's files: a commit of its container layer.
    #[serde(default)]
    pub files: bool,
    /// The values of the composes' secret parameters and the passwords of the containers' own
    /// proxies.
    #[serde(default)]
    pub secrets: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ContainerEntry {
    pub id: String,
    pub name: String,
    /// Whether the backup carries its files.
    pub files: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ImageEntry {
    pub id: String,
    pub name: String,
    /// Whether the backup carries its Docker image; one Docker no longer had is a record only.
    pub image: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct VolumeEntry {
    pub name: String,
    /// Whether the backup carries its files; one that is not is only named by a container.
    pub included: bool,
    /// The container it was created for, and the compose parameter, from its labels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PartKind {
    /// A saved image's `docker save` stream; `target` is its record id.
    Image,
    /// A `docker save` stream of a commit of a container's files; `target` is the container id.
    Files,
    /// A volume's files as a plain tar; `target` is the volume's name.
    Volume,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Part {
    pub kind: PartKind,
    pub target: String,
}

/// The backup's first entry: what it holds, by name, without a secret.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub kind: String,
    pub format: u32,
    /// The release that wrote it.
    pub version: String,
    pub created_at: u64,
    pub included: Included,
    pub containers: Vec<ContainerEntry>,
    pub images: Vec<ImageEntry>,
    pub volumes: Vec<VolumeEntry>,
    pub endpoints: Vec<String>,
    /// The parts after the records, in order.
    pub parts: Vec<Part>,
    /// The containers whose secret values were left out.
    #[serde(default)]
    pub secrets_left_out: Vec<String>,
}

/// A container's pinned compose with the container it belongs to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PinnedCompose {
    pub container_id: String,
    #[serde(flatten)]
    pub pinned: Pinned,
}

/// The password of a container's own proxy with the container it belongs to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ProxyPassword {
    pub container_id: String,
    #[serde(flatten)]
    pub credentials: Credentials,
}

/// The backup's second entry: the records of the included kinds.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Records {
    #[serde(default)]
    pub containers: Vec<Container>,
    #[serde(default)]
    pub images: Vec<ImageRecord>,
    #[serde(default)]
    pub composes: Vec<PinnedCompose>,
    #[serde(default)]
    pub endpoints: Vec<Endpoint>,
    #[serde(default)]
    pub proxy_passwords: Vec<ProxyPassword>,
}

/// The last entry, so a backup cut short is told from a whole one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Ending {
    pub parts: usize,
}

/// A part's last entry: how long its stream was and its BLAKE2s-256 digest in hex, checked before
/// what it carried is kept.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PartEnd {
    pub bytes: u64,
    pub blake2s: String,
}

/// A volume the backup may carry, with the labels that name the container it was created for.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct VolumeFacts {
    pub owner: Option<String>,
    pub parameter: Option<String>,
}

/// What the installation has, as a backup starts: its records and what Docker still holds.
#[derive(Default)]
pub(crate) struct Installation {
    pub containers: Vec<Container>,
    pub images: Vec<ImageRecord>,
    pub composes: Vec<(String, Pinned)>,
    pub endpoints: Vec<Endpoint>,
    pub proxy_passwords: Vec<(String, Credentials)>,
    /// Saved images whose Docker image exists, by record id.
    pub available_images: HashSet<String>,
    /// Containers whose files can be saved: their Docker container, or the commit a settings
    /// change left them, exists.
    pub with_files: HashSet<String>,
    /// The volumes a backup may carry: those the containers mount and those Thoric created
    /// that the volume list shows, never the manager's own.
    pub volumes: BTreeMap<String, VolumeFacts>,
}

/// Choose what a backup carries: every record of the included kinds, the parts `included` asks
/// for, and the secret values only when it asks for them too.
pub(crate) fn plan(
    installation: Installation,
    included: Included,
    version: &str,
    created_at: u64,
) -> (Manifest, Records) {
    let Installation {
        mut containers,
        mut images,
        composes,
        endpoints,
        proxy_passwords,
        available_images,
        with_files,
        volumes,
    } = installation;
    containers.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    images.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    if !included.images {
        images.clear();
    }
    let ids: HashSet<&str> = containers.iter().map(|c| c.id.as_str()).collect();
    let mut secrets_left_out = Vec::new();
    let mut kept_composes = Vec::new();
    for (container_id, mut pinned) in composes {
        if !ids.contains(container_id.as_str()) {
            continue;
        }
        if !included.secrets {
            let secret = pinned.setup("").secrets;
            let before = pinned.values.len();
            pinned.values.retain(|key, _| !secret.contains(key));
            if pinned.values.len() != before {
                secrets_left_out.push(container_id.clone());
            }
        }
        kept_composes.push(PinnedCompose {
            container_id,
            pinned,
        });
    }
    kept_composes.sort_by(|a, b| a.container_id.cmp(&b.container_id));
    let mut kept_passwords = Vec::new();
    for (container_id, credentials) in proxy_passwords {
        if !ids.contains(container_id.as_str()) {
            continue;
        }
        if included.secrets {
            kept_passwords.push(ProxyPassword {
                container_id,
                credentials,
            });
        } else if !secrets_left_out.contains(&container_id) {
            secrets_left_out.push(container_id);
        }
    }
    kept_passwords.sort_by(|a, b| a.container_id.cmp(&b.container_id));
    secrets_left_out.sort();
    // An endpoint of a scale set covers a set the backup does not carry.
    let mut endpoints = endpoints
        .into_iter()
        .filter(|endpoint| endpoint.grants.iter().all(|grant| grant.set.is_none()))
        .collect::<Vec<_>>();
    endpoints.sort_by(|a, b| a.name.cmp(&b.name));

    let mut parts = Vec::new();
    let image_entries = images
        .iter()
        .map(|record| {
            let image = available_images.contains(&record.id);
            if image {
                parts.push(Part {
                    kind: PartKind::Image,
                    target: record.id.clone(),
                });
            }
            ImageEntry {
                id: record.id.clone(),
                name: record.name.clone(),
                image,
            }
        })
        .collect();
    let container_entries = containers
        .iter()
        .map(|container| {
            let files = included.files && with_files.contains(&container.id);
            if files {
                parts.push(Part {
                    kind: PartKind::Files,
                    target: container.id.clone(),
                });
            }
            ContainerEntry {
                id: container.id.clone(),
                name: container.spec.name.clone(),
                files,
            }
        })
        .collect();
    // Every volume a container mounts is named, carried or not, so a restore can tell what is
    // missing; the others only when they are carried.
    let mounted: BTreeSet<String> = containers
        .iter()
        .flat_map(Container::mounts)
        .map(|mount: Mount| mount.volume)
        .collect();
    let mut volume_entries = Vec::new();
    for name in volumes.keys().chain(&mounted).collect::<BTreeSet<_>>() {
        let facts = volumes.get(name);
        let carried = included.volumes && facts.is_some();
        if !carried && !mounted.contains(name) {
            continue;
        }
        if carried {
            parts.push(Part {
                kind: PartKind::Volume,
                target: name.clone(),
            });
        }
        let facts = facts.cloned().unwrap_or_default();
        volume_entries.push(VolumeEntry {
            name: name.clone(),
            included: carried,
            owner: facts.owner,
            parameter: facts.parameter,
        });
    }
    let manifest = Manifest {
        kind: KIND.into(),
        format: FORMAT,
        version: version.into(),
        created_at,
        included,
        containers: container_entries,
        images: image_entries,
        volumes: volume_entries,
        endpoints: endpoints.iter().map(|e| e.name.clone()).collect(),
        parts,
        secrets_left_out,
    };
    let records = Records {
        containers,
        images,
        composes: kept_composes,
        endpoints,
        proxy_passwords: kept_passwords,
    };
    (manifest, records)
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
