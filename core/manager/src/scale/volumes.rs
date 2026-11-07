//! What an instance of a scale set mounts: for each volume of the template, the template's volume
//! or an empty one of its own, by the set's policy and what the request for that instance asks.
//! A volume of its own is named after the instance's container, as a compose's `new_volume`
//! parameter names one, and carries its owner's labels, so deleting the instance with its volumes
//! takes it too while the shared ones stay.
use super::{ScaleSet, VolumePolicy};
use crate::{compose::Pinned, model::Mount};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What a request for one instance asks for a volume of the template, by its folder, over the
/// set's policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Override {
    /// The template's volume.
    Shared,
    /// An empty volume of the instance's own.
    New,
}

/// A volume of the template: the folder it is mounted at, the template's volume there, empty for
/// a compose parameter given none, and the compose value it is when a compose parameter mounts it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Slot {
    pub target: String,
    pub volume: String,
    pub key: Option<String>,
    pub readonly: bool,
}

/// The template's volumes: those of its settings, then those of its compose's volume parameters.
pub(crate) fn slots(set: &ScaleSet, pinned: Option<&Pinned>) -> Vec<Slot> {
    let mut slots = set
        .template
        .mounts
        .iter()
        .map(|mount| Slot {
            target: mount.target.clone(),
            volume: mount.volume.clone(),
            key: None,
            readonly: mount.readonly,
        })
        .collect::<Vec<_>>();
    if let Some(pinned) = pinned {
        for (key, path) in pinned.volume_slots() {
            slots.push(Slot {
                target: path,
                volume: pinned.values.get(&key).cloned().unwrap_or_default(),
                key: Some(key),
                readonly: false,
            });
        }
    }
    slots
}

/// Check that every folder `chosen` names is one of the template's volumes.
pub(crate) fn check_targets<T>(slots: &[Slot], chosen: &BTreeMap<String, T>) -> Result<()> {
    for target in chosen.keys() {
        if !slots.iter().any(|slot| slot.target == *target) {
            let known = slots
                .iter()
                .map(|slot| slot.target.as_str())
                .collect::<Vec<_>>();
            if known.is_empty() {
                bail!("The set mounts no volume, so {target} cannot be chosen");
            }
            bail!(
                "The set mounts no volume at {target}; its volumes are at {}",
                known.join(", ")
            );
        }
    }
    Ok(())
}

/// Whether instance `number` gets a volume of its own for a slot of this policy, unless the
/// request for it asks otherwise.
pub(crate) fn own(policy: VolumePolicy, number: u32, asked: Option<Override>) -> bool {
    match asked {
        Some(Override::Shared) => false,
        Some(Override::New) => true,
        None => match policy {
            VolumePolicy::Shared => false,
            VolumePolicy::New => true,
            VolumePolicy::First => number > 1,
        },
    }
}

/// The name of the volume the container `container` gets of its own for `slot`, which is
/// `part`: `thoric-<container>-<part>`, in the characters volume names have and at most 128.
fn own_name(container: &str, part: &str) -> String {
    let part: String = part
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let part = part.trim_matches('-');
    let mut name = format!("thoric-{container}-{part}");
    // Volume names are ASCII here, so any length is a character boundary.
    name.truncate(128);
    name.trim_end_matches('-').to_owned()
}

/// What a slot's own volume is named after: a compose parameter's value name, as its
/// `new_volume` would name one, or `mount` and the folder for a volume of the settings.
fn part(slot: &Slot) -> String {
    match &slot.key {
        Some(key) => key.replace(['.', '_'], "-"),
        None => format!("mount{}", slot.target.replace('/', "-")),
    }
}

/// What instance `number`, the container `container`, mounts and is given.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Planned {
    /// Its settings' volumes.
    pub mounts: Vec<Mount>,
    /// Its compose values, with the volumes of its volume parameters, when the set has pinned a
    /// compose.
    pub values: Option<BTreeMap<String, String>>,
    /// The volumes made for it, each with what it was made for: a compose value's name, or the
    /// folder of a volume of the settings.
    pub own: Vec<(String, String)>,
    /// The folders whose volume it shares with the template.
    pub shared: Vec<String>,
}

/// Plan the volumes of instance `number`. A slot whose template volume is one of `gone`, such as
/// one the first instance got of its own and that went with it, gets a volume of its own too.
pub(crate) fn plan(
    set: &ScaleSet,
    pinned: Option<&Pinned>,
    number: u32,
    container: &str,
    asked: &BTreeMap<String, Override>,
    gone: &[String],
) -> Planned {
    let mut planned = Planned {
        values: pinned.map(|pinned| pinned.values.clone()),
        ..Planned::default()
    };
    for slot in slots(set, pinned) {
        let mine = own(
            set.policy(&slot.target),
            number,
            asked.get(&slot.target).copied(),
        ) || gone.contains(&slot.volume);
        let volume = if mine {
            let mut name = own_name(container, &part(&slot));
            // Two folders whose names clean to one part keep volumes apart.
            let mut n = 2;
            while planned.own.iter().any(|(_, own)| *own == name) {
                name = own_name(container, &format!("{}-{n}", part(&slot)));
                n += 1;
            }
            let made_for = slot.key.clone().unwrap_or_else(|| slot.target.clone());
            planned.own.push((made_for, name.clone()));
            name
        } else {
            if !slot.volume.is_empty() {
                planned.shared.push(slot.target.clone());
            }
            slot.volume.clone()
        };
        match (&slot.key, planned.values.as_mut()) {
            (Some(key), Some(values)) => {
                values.insert(key.clone(), volume);
            }
            _ => planned.mounts.push(Mount {
                volume,
                target: slot.target.clone(),
                readonly: slot.readonly,
            }),
        }
    }
    planned
}

#[cfg(test)]
#[path = "volumes_tests.rs"]
mod tests;
