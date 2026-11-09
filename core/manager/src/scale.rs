//! Scale sets: a container made scalable, whose settings and compose values are kept as a template
//! that new instances are created from on demand. Each instance is an ordinary container, created
//! as the first one was, from the same image with the same values: nothing is committed or copied,
//! so an instance starts from the image and its compose setup, never from another instance's
//! files. What instances share are volumes: each volume of the template is mounted by every
//! instance (`shared`), or each instance gets an empty one of its own (`new`), or the first keeps
//! the template's and the others get their own (`first`), or every instance mounts the template's
//! and only the first may write there (`first_writes`). Each desktop MCP server of the template
//! runs in every instance (`each`), or in instance 1 alone for all of them (`first`), for a server
//! that holds what only one copy may hold, such as a project on a shared volume with a lock file.
//!
//! The set's record is the one place that says which containers are its instances, by number,
//! and it keeps a number from the moment an instance's creation starts, so two creations never
//! take one number and an agent can follow the job of an instance whose record is not saved yet.
//! The compose values, secret ones too, and the password of the template's own proxy live in a
//! record of their own (`TEMPLATES`) that no route returns, as a container's do.
pub(crate) mod instances;
pub(crate) mod volumes;

use crate::{
    App,
    compose::Pinned,
    model::{CONTAINERS, Container, ContainerSpec, now},
};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The database kind of scale sets, by name.
pub(crate) const SETS: &str = "scale-set";
/// The database kind of each set's compose values and proxy password, by the set's name.
pub(crate) const TEMPLATES: &str = "scale-set-template";
/// The most instances a set may have.
pub(crate) const MAX_INSTANCES: u32 = 16;
/// The instances a set may have when its request names no maximum.
const DEFAULT_MAX: u32 = 4;
/// A set's name is part of its instances' names and its endpoint's path.
const NAME_LIMIT: usize = 40;

/// What an instance mounts for a volume of the template.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VolumePolicy {
    /// The template's volume, which every instance mounts.
    #[default]
    Shared,
    /// An empty volume of its own, which goes with it when it is deleted.
    New,
    /// The template's volume for instance 1, an empty one of its own for the others.
    First,
    /// The template's volume in every instance, which instance 1 mounts to read and write and
    /// the others read-only, so they see what it keeps there and cannot change it. Docker mounts
    /// it read-only, which root in a desktop without `SYS_ADMIN` cannot undo.
    FirstWrites,
}

/// Where the instances run a desktop MCP server of the template.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ServerPolicy {
    /// Every instance runs its own.
    #[default]
    Each,
    /// Instance 1 alone runs it, in one process, and its tools serve every instance: for a server
    /// that holds what only one copy may hold, such as a project a shared volume keeps with a lock
    /// file. The other instances never start it.
    First,
}

/// The most desktop servers a set names a policy for.
const SERVER_POLICY_LIMIT: usize = 64;

/// A scale set as it is saved: the template, its limits and its instances.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ScaleSet {
    pub name: String,
    pub created_at: u64,
    /// The settings every instance is created with, under a name and an identity of its own.
    pub template: ContainerSpec,
    /// The instances that stay: the lowest numbers, which neither an agent nor the idle delete
    /// removes.
    pub min: u32,
    pub max: u32,
    /// Delete an instance beyond `min` that nothing used for this many minutes; 0 never.
    #[serde(default)]
    pub idle_delete_minutes: u32,
    /// What each volume of the template becomes in an instance, by the folder it is mounted at.
    /// A folder left out is shared.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub volumes: BTreeMap<String, VolumePolicy>,
    /// Where each desktop MCP server runs, by its name. A server left out runs in every instance.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub servers: BTreeMap<String, ServerPolicy>,
    /// The instances by number, also those still being created.
    #[serde(default)]
    pub instances: BTreeMap<u32, Instance>,
    /// The MCP endpoint made for the set, whose tools take `instance`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// The set is being deleted with its instances: no instance is created, deleted on its own
    /// or changed meanwhile, and the record goes once the last instance is gone, so the name
    /// stays taken and a deletion that fails halfway leaves a set that is deleted again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleting: bool,
}

/// One instance of a set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Instance {
    pub container_id: String,
    /// What the instance is for, as the request that created it said, such as the agent that
    /// works in it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// The longest note an instance keeps.
pub(crate) const NOTE_LIMIT: usize = 200;

impl ScaleSet {
    /// What an instance mounts for the template's volume at `target`.
    pub fn policy(&self, target: &str) -> VolumePolicy {
        self.volumes.get(target).copied().unwrap_or_default()
    }
    /// Where the instances run the desktop server `server`.
    pub fn server_policy(&self, server: &str) -> ServerPolicy {
        self.servers.get(server).copied().unwrap_or_default()
    }
    /// The instance that alone runs the desktop server `server` for every instance, or `None`
    /// when each runs its own.
    pub fn runs_in(&self, server: &str) -> Option<u32> {
        (self.server_policy(server) == ServerPolicy::First).then_some(1)
    }
    /// The number of the instance that is the container `id`.
    pub fn number_of(&self, id: &str) -> Option<u32> {
        self.instances
            .iter()
            .find(|(_, instance)| instance.container_id == id)
            .map(|(number, _)| *number)
    }
    /// The lowest number no instance has, within the maximum.
    pub fn free_number(&self) -> Option<u32> {
        (1..=self.max).find(|number| !self.instances.contains_key(number))
    }
    /// The numbers that stay: the lowest `min` of those the set has.
    pub fn kept(&self) -> Vec<u32> {
        self.instances
            .keys()
            .take(self.min as usize)
            .copied()
            .collect()
    }
    /// The name instance `number` is created with.
    pub fn instance_name(&self, number: u32) -> String {
        format!("{}-{number}", self.name)
    }
}

/// What the set keeps apart from its record: the values its compose was given, secret ones too,
/// the compose and values its instances are created with once one has pinned them, and the
/// password of the template's own proxy.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Template {
    /// The compose values as the request gave them.
    #[serde(default)]
    pub parameters: BTreeMap<String, String>,
    /// The image record whose compose the instances run, and that compose with the values it
    /// was given for the first instance created from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<(String, Pinned)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_password: Option<String>,
}

/// How a container is made scalable, in a creation request or for an existing container.
#[derive(Clone, Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scaling {
    /// The set's name: lowercase letters, digits and dashes. Its instances are named
    /// `<set>-<number>` and its MCP endpoint `/mcp/<set>`. The container's name made into one when
    /// left out.
    #[serde(default)]
    pub set: Option<String>,
    /// How many instances stay, the lowest numbers: created with the set, and never deleted by
    /// an agent or for being idle. 1 when left out.
    #[serde(default = "one")]
    pub min: u32,
    /// The most instances the set may have, at most 16. 4 when left out.
    #[serde(default = "default_max")]
    pub max: u32,
    /// Delete an instance beyond `min` that nothing used for this many minutes; 0, the default,
    /// never.
    #[serde(default)]
    pub idle_delete_minutes: u32,
    /// What each volume becomes in an instance, by the folder it is mounted at: `shared`, the
    /// default, `new`, `first` or `first_writes`.
    #[serde(default)]
    pub volumes: BTreeMap<String, VolumePolicy>,
    /// Where each desktop MCP server runs, by its name: `each`, the default, in every instance,
    /// or `first`, in instance 1 alone, whose tools then serve every instance. `first` is for a
    /// server that locks what the instances share, such as a project on a shared volume.
    #[serde(default)]
    pub servers: BTreeMap<String, ServerPolicy>,
}
fn one() -> u32 {
    1
}
fn default_max() -> u32 {
    DEFAULT_MAX
}

impl Scaling {
    pub fn validate(&self) -> Result<()> {
        limits(self.min, self.max, self.idle_delete_minutes)?;
        if let Some(name) = &self.set {
            set_name(name)?;
        }
        check_policies(&self.volumes)?;
        check_servers(&self.servers)
    }
}

/// Volume policies a request may give: each for a folder a volume can be mounted at, and never
/// `first_writes`, which mounts the volume read-only after instance 1, for the desktop home or a
/// folder of its configuration, which the session writes; a container's own volume may not be
/// read-only there either.
pub(crate) fn check_policies(volumes: &BTreeMap<String, VolumePolicy>) -> Result<()> {
    for (target, policy) in volumes {
        crate::model::mount_target(target)?;
        if *policy == VolumePolicy::FirstWrites && crate::model::must_stay_writable(target) {
            bail!(
                "{target} must stay writable in every instance, so only instance 1 writing there cannot be chosen; the desktop home and its configuration folders are written by the session"
            );
        }
    }
    Ok(())
}

/// Server policies a request may give: at most [`SERVER_POLICY_LIMIT`], each for a valid server
/// name. A server the instances do not declare yet is kept, as one the desktop user may add later.
pub(crate) fn check_servers(servers: &BTreeMap<String, ServerPolicy>) -> Result<()> {
    if servers.len() > SERVER_POLICY_LIMIT {
        bail!("A scale set names at most {SERVER_POLICY_LIMIT} desktop MCP servers");
    }
    for name in servers.keys() {
        crate::desktop_mcp::server_name(name)?;
    }
    Ok(())
}

/// The server policies a set keeps of a request's: those that are not the default.
pub(crate) fn server_policies(
    servers: &BTreeMap<String, ServerPolicy>,
) -> BTreeMap<String, ServerPolicy> {
    servers
        .iter()
        .filter(|(_, policy)| **policy != ServerPolicy::Each)
        .map(|(name, policy)| (name.clone(), *policy))
        .collect()
}

/// A set's limits: at most [`MAX_INSTANCES`], a minimum that fits under the maximum and an idle
/// delete of at most a week.
pub(crate) fn limits(min: u32, max: u32, idle_delete_minutes: u32) -> Result<()> {
    if !(1..=MAX_INSTANCES).contains(&max) {
        bail!("A scale set has 1 to {MAX_INSTANCES} instances at most");
    }
    if min > max {
        bail!("A scale set's minimum cannot exceed its maximum");
    }
    if idle_delete_minutes > crate::lifecycle::MAX_IDLE_MINUTES {
        bail!(
            "idle_delete_minutes is at most {}, a week",
            crate::lifecycle::MAX_IDLE_MINUTES
        );
    }
    Ok(())
}

/// A set's name: lowercase letters, digits and dashes, at most 40, not starting or ending with a
/// dash.
pub(crate) fn set_name(name: &str) -> Result<()> {
    if !crate::desktop_mcp::slug(name, NAME_LIMIT) || name.ends_with('-') {
        bail!(
            "A scale set's name is lowercase letters, digits and dashes, at most {NAME_LIMIT}, not starting or ending with a dash"
        );
    }
    Ok(())
}

/// The name a set made of the container `name` gets when its request names none.
pub(crate) fn name_of(name: &str) -> String {
    crate::desktop_mcp::slug_of(name, NAME_LIMIT, "set")
}

/// The template a set keeps of `spec`: everything but the name, which each instance gets of its
/// own, the machine identity, which each is given anew, and the expiry, a moment the first
/// container's task ends.
pub(crate) fn template_of(spec: &ContainerSpec, set: &str) -> ContainerSpec {
    crate::duplicate::copied_spec(spec, set)
}

/// Every set, sorted by name.
pub(crate) fn all(app: &App) -> Result<Vec<ScaleSet>> {
    let mut sets: Vec<ScaleSet> = app.db.list(SETS)?;
    sets.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(sets)
}

/// The set the container `id` is an instance of, with its number.
pub(crate) fn instance_of<'a>(sets: &'a [ScaleSet], id: &str) -> Option<(&'a ScaleSet, u32)> {
    sets.iter()
        .find_map(|set| set.number_of(id).map(|number| (set, number)))
}

/// What a container's entry in the lists says of its set: its name and number, `null` for a
/// container of none.
pub(crate) fn membership(sets: &[ScaleSet], id: &str) -> Value {
    match instance_of(sets, id) {
        Some((set, number)) => json!({"set": set.name, "number": number}),
        None => Value::Null,
    }
}

/// The lock a set's record is changed under.
pub(crate) fn lock(app: &App, name: &str) -> crate::jobs::ResourceLock {
    app.resource_lock(&format!("scale-set:{name}"))
}

/// Take the container `id` out of the set it is an instance of, once it is deleted or its
/// creation ended without a record.
pub(crate) async fn forget_container(app: &App, id: &str) -> Result<()> {
    for set in app.db.list::<ScaleSet>(SETS)? {
        if set.number_of(id).is_none() {
            continue;
        }
        let lock = lock(app, &set.name);
        let _held = lock.lock().await;
        // Read again under the lock, which every change of the record holds.
        let Ok(mut set) = app.db.get::<ScaleSet>(SETS, &set.name) else {
            continue;
        };
        set.instances
            .retain(|_, instance| instance.container_id != id);
        app.db.put(SETS, &set.name.clone(), &set)?;
        crate::desktop_mcp::announce_set(app, &set.name);
    }
    Ok(())
}

/// Forget the numbers of instances whose creation a restart interrupted before their record was
/// saved: no job outlives the manager, so nothing will save them.
pub(crate) fn prune(db: &crate::db::Db) -> Result<()> {
    let containers = db.ids(CONTAINERS)?;
    for mut set in db.list::<ScaleSet>(SETS)? {
        let before = set.instances.len();
        set.instances
            .retain(|_, instance| containers.contains(&instance.container_id));
        if set.instances.len() != before {
            db.put(SETS, &set.name.clone(), &set)?;
        }
    }
    Ok(())
}

/// The set whose instances are created from an image `names` names: a reference, an image ID or
/// the image record whose compose the set pinned.
pub(crate) fn starts_from<'a>(
    sets: &'a [ScaleSet],
    templates: &BTreeMap<String, Template>,
    names: &[&str],
) -> Option<&'a ScaleSet> {
    sets.iter().find(|set| {
        names.contains(&set.template.image.as_str())
            || templates
                .get(&set.name)
                .and_then(|template| template.pinned.as_ref())
                .is_some_and(|(id, _)| names.contains(&id.as_str()))
    })
}

/// Every set's template, by the set's name.
pub(crate) fn templates(app: &App) -> Result<BTreeMap<String, Template>> {
    Ok(app
        .db
        .entries_complete::<Template>(TEMPLATES)?
        .into_iter()
        .collect())
}

/// The set that mounts `volume` in the instances it creates, by its template.
pub(crate) fn mounting<'a>(
    sets: &'a [ScaleSet],
    templates: &BTreeMap<String, Template>,
    volume: &str,
) -> Option<&'a ScaleSet> {
    sets.iter().find(|set| {
        set.template
            .mounts
            .iter()
            .any(|mount| mount.volume == volume && set.policy(&mount.target) != VolumePolicy::New)
            || templates
                .get(&set.name)
                .and_then(|template| template.pinned.as_ref())
                .is_some_and(|(_, pinned)| {
                    pinned.mounts().iter().any(|mount| {
                        mount.volume == volume && set.policy(&mount.target) != VolumePolicy::New
                    })
                })
    })
}

/// Whether the instance `number` of `set` is deleted for being idle: one beyond the minimum that
/// went unused for the set's idle delete, whether its desktop runs or not.
pub(crate) fn idle_delete_due(
    set: &ScaleSet,
    number: u32,
    idle: Option<std::time::Duration>,
) -> Option<u32> {
    let minutes = set.idle_delete_minutes;
    let limit = std::time::Duration::from_secs(u64::from(minutes) * 60);
    (minutes > 0
        && !set.deleting
        && !set.kept().contains(&number)
        && idle.is_some_and(|idle| idle >= limit))
    .then_some(minutes)
}

/// A set as the dashboard and the tools show it, with its instances' containers when they are
/// known: never its compose values or its proxy password.
pub(crate) fn summary(set: &ScaleSet, containers: &[Container]) -> Value {
    let instances = set
        .instances
        .iter()
        .map(|(number, instance)| {
            let record = containers
                .iter()
                .find(|container| container.id == instance.container_id);
            json!({
                "number": number,
                "container_id": instance.container_id,
                "note": instance.note,
                "name": record.map(|record| record.spec.name.clone()),
                "creating": record.is_none(),
                "kept": set.kept().contains(number),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "name": set.name,
        "created_at": set.created_at,
        "image": set.template.image,
        "min": set.min,
        "max": set.max,
        "idle_delete_minutes": set.idle_delete_minutes,
        "volumes": set.volumes,
        "servers": set.servers,
        "instances": instances,
        "endpoint": set.endpoint,
        "deleting": set.deleting,
        "endpoint_path": set.endpoint.as_ref().map(|name| format!("/mcp/{name}")),
    })
}

/// A new set for the template `spec`, with no instance yet.
pub(crate) fn new_set(name: &str, spec: &ContainerSpec, scaling: &Scaling) -> ScaleSet {
    ScaleSet {
        name: name.into(),
        created_at: now(),
        template: template_of(spec, name),
        min: scaling.min,
        max: scaling.max,
        idle_delete_minutes: scaling.idle_delete_minutes,
        volumes: scaling
            .volumes
            .iter()
            .filter(|(_, policy)| **policy != VolumePolicy::Shared)
            .map(|(target, policy)| (target.clone(), *policy))
            .collect(),
        servers: server_policies(&scaling.servers),
        instances: BTreeMap::new(),
        endpoint: None,
        deleting: false,
    }
}

#[cfg(test)]
#[path = "scale_tests.rs"]
mod tests;
