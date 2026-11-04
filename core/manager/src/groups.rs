//! Container groups. Containers that share a group name share an internal Docker network,
//! `thoric-group-<name>`, that nothing else is on: neither the manager nor the internet. There
//! each member is reached by its hostname, by its name as a DNS label and by the aliases its
//! settings list. A group is only a name in its members' settings: its network is created when
//! the first member joins and removed once no container record lists the group.
//!
//! A group's network has no address on the Docker host where the engine can leave it without one
//! (see `docker::isolation`).
//!
//! A desktop serves VNC without a password, so its VNC server listens only in its private network
//! (`desktop-private-address`). An image older than that carries no `thoric.vnc=private` label,
//! and a desktop from it shows its screen to the members of its groups; the container list says
//! so with `vnc_private`.
use crate::{
    App,
    docker::{GROUP_PREFIX, Reach, isolated},
    model::{CONTAINERS, Container, ContainerSpec, LABEL},
};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::collections::BTreeSet;

/// What every group network's name starts with, before the group's name.
const PREFIX: &str = "thoric-group-";
/// The label that names the group a network belongs to.
const GROUP_LABEL: &str = "thoric.group";
/// The label a desktop image carries once its VNC server listens only in the private network,
/// and the value that says so.
pub(crate) const VNC_LABEL: &str = "thoric.vnc";
pub(crate) const VNC_PRIVATE: &str = "private";
/// At most this many groups, and this many aliases, per container.
const MOST: usize = 8;

/// The Docker network of a group.
pub(crate) fn network(group: &str) -> String {
    format!("{PREFIX}{group}")
}

/// A group name or an alias: one DNS label of lowercase letters, digits and hyphens.
fn label(value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 63
        || bytes.starts_with(b"-")
        || bytes.ends_with(b"-")
        || !bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
    {
        bail!(
            "Group names and aliases are 1 to 63 lowercase letters, digits and hyphens, without a hyphen at either end: {value}"
        );
    }
    Ok(())
}

/// Check a container's groups and aliases.
pub(crate) fn validate(spec: &ContainerSpec) -> Result<()> {
    for (kind, list) in [("groups", &spec.groups), ("aliases", &spec.aliases)] {
        if list.len() > MOST {
            bail!("A container has at most {MOST} {kind}");
        }
        let mut seen = BTreeSet::new();
        for value in list {
            label(value)?;
            if !seen.insert(value) {
                bail!("{value} is listed twice in the {kind}");
            }
        }
    }
    Ok(())
}

/// A container's name as a DNS label: lowercase letters and digits, Latin letters without their
/// marks, and a hyphen for everything else; `None` when nothing is left. The dashboard's
/// `groups.ts` does the same, and both are held to `core/web/tests/name-labels.json`.
pub(crate) fn name_label(name: &str) -> Option<String> {
    let mut label = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        let c = match c {
            // What `İ` lowercases to, and the marks of letters written apart from them.
            '\u{300}'..='\u{36f}' => continue,
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ğ' => 'g',
            'ı' | 'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
            'ş' | 'ß' => 's',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            label.push(c);
        } else if !label.is_empty() && !label.ends_with('-') {
            label.push('-');
        }
    }
    label.truncate(63);
    let label = label.trim_end_matches('-');
    (!label.is_empty()).then(|| label.to_owned())
}

/// The names the members of a container's groups reach it by, each once: its hostname, its name
/// as a DNS label and its aliases.
pub(crate) fn names(spec: &ContainerSpec) -> Vec<String> {
    let hostname = spec.hostname.as_deref().map(str::to_ascii_lowercase);
    let mut names = Vec::new();
    for name in hostname
        .into_iter()
        .chain(name_label(&spec.name))
        .chain(spec.aliases.iter().cloned())
    {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// Refuse a name that another member of one of the container's groups is reached by already,
/// which the group's DNS would answer with either container.
pub(crate) fn names_free(id: &str, spec: &ContainerSpec, others: &[Container]) -> Result<()> {
    let own = names(spec);
    for other in others.iter().filter(|other| other.id != id) {
        let Some(group) = spec.groups.iter().find(|g| other.spec.groups.contains(g)) else {
            continue;
        };
        if let Some(name) = names(&other.spec).into_iter().find(|n| own.contains(n)) {
            bail!(
                "\"{}\" in the group {group} is reached as {name} already; choose another name, hostname or alias",
                other.spec.name
            );
        }
    }
    Ok(())
}

/// The names of every other container that shares a group with `id`, which its desktop's proxy
/// lets through straight to them.
pub(crate) fn peers(id: &str, spec: &ContainerSpec, others: &[Container]) -> Vec<String> {
    let mut peers = Vec::new();
    for other in others.iter().filter(|other| other.id != id) {
        if spec.groups.iter().any(|g| other.spec.groups.contains(g)) {
            for name in names(&other.spec) {
                if !peers.contains(&name) {
                    peers.push(name);
                }
            }
        }
    }
    peers
}

/// Whether an endpoint in `docker inspect` of a container answers to exactly `wanted`. Docker
/// before API 1.45 listed the container's short ID among the aliases too.
fn same_aliases(endpoint: &Value, wanted: &[String], container_id: &str) -> bool {
    let short = container_id.get(..12).unwrap_or(container_id);
    let have = endpoint["Aliases"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|alias| *alias != short)
        .collect::<BTreeSet<_>>();
    have == wanted.iter().map(String::as_str).collect()
}

/// Whether the container behind an entry of `docker ps --format '{{json .}}'` keeps VNC to its
/// private network: its image says so and it was not created without the private network's subnet.
pub(crate) fn vnc_private(ps: &Value) -> bool {
    let wanted = format!("{VNC_LABEL}={VNC_PRIVATE}");
    ps["Labels"]
        .as_str()
        .is_some_and(|labels| labels.split(',').any(|label| label == wanted))
}

impl App {
    /// [`names_free`] against every other container record that reads. One that does not is
    /// left out rather than refusing every change of every container's names.
    pub(crate) fn group_names_free(&self, id: &str, spec: &ContainerSpec) -> Result<()> {
        names_free(id, spec, &self.db.list::<Container>(CONTAINERS)?)
    }

    /// Bring a Docker container's group networks in line with its record: join each group's
    /// network under the names it is reached by, creating the network for the group's first
    /// member, and leave the groups it no longer lists, removing a network no record lists any
    /// more. `info` is `docker inspect` of the container, which may be stopped.
    pub(crate) async fn join_groups(&self, env: &Container, info: &Value) -> Result<()> {
        let container_id = info["Id"].as_str().context("Missing container ID")?;
        let connected = &info["NetworkSettings"]["Networks"];
        let wanted = names(&env.spec);
        for group in &env.spec.groups {
            let name = network(group);
            let lock = self.resource_lock(&format!("group:{group}"));
            let _guard = lock.lock().await;
            if let Some(endpoint) = connected.get(&name) {
                if same_aliases(endpoint, &wanted, container_id) {
                    continue;
                }
                // An endpoint keeps the aliases it was connected with.
                self.docker(&["network", "disconnect", &name, container_id])
                    .await?;
            }
            let id = self.group_network(group).await?;
            self.connect_network_as(env, &id, container_id, &wanted)
                .await?;
        }
        for name in connected.as_object().into_iter().flat_map(|n| n.keys()) {
            let Some(group) = name.strip_prefix(PREFIX) else {
                continue;
            };
            if env.spec.groups.iter().any(|g| g == group) {
                continue;
            }
            let lock = self.resource_lock(&format!("group:{group}"));
            let _guard = lock.lock().await;
            self.docker(&["network", "disconnect", name, container_id])
                .await?;
            self.drop_group_network(group, &env.id).await?;
        }
        Ok(())
    }

    /// Remove the networks of the groups a deleted container was the last member of, while its
    /// record, which no longer counts, is still there.
    pub(crate) async fn leave_groups(&self, env: &Container) -> Result<()> {
        for group in &env.spec.groups {
            let lock = self.resource_lock(&format!("group:{group}"));
            let _guard = lock.lock().await;
            self.drop_group_network(group, &env.id).await?;
        }
        Ok(())
    }

    /// The ID of a group's network, created when it is missing, without an address on the Docker
    /// host where the engine can leave it without one. One made with an address is made again
    /// without while no container is on it. A network of that name Thoric did not create for the
    /// group is refused.
    async fn group_network(&self, group: &str) -> Result<String> {
        let name = network(group);
        let info = match self.optional_inspect("network", &name).await? {
            Some(info) => info,
            None => self.create_group_network(group).await?,
        };
        if info["Labels"][LABEL] != "true"
            || info["Labels"][GROUP_LABEL] != group
            || info["Internal"] != true
            || info["Driver"] != "bridge"
        {
            bail!("The network {name} exists, but Thoric did not create it for the group {group}");
        }
        let id = info["Id"].as_str().context("Missing network ID")?;
        if !isolated(&info)
            && self.network_users(id).await?.is_empty()
            && self.internal_reach().await == Reach::Nothing
        {
            self.docker(&["network", "rm", id]).await?;
            let made = self.create_group_network(group).await?;
            return Ok(made["Id"]
                .as_str()
                .context("Missing network ID")?
                .to_owned());
        }
        Ok(id.to_owned())
    }

    /// A group's network, in a /24 of `DESKTOP_ADDRESS_POOL` for its members.
    async fn create_group_network(&self, group: &str) -> Result<Value> {
        let labels = [format!("{LABEL}=true"), format!("{GROUP_LABEL}={group}")];
        let reach = self.internal_reach().await;
        self.make_network(&network(group), &labels, reach, None, GROUP_PREFIX)
            .await
    }

    /// Remove a group's network once no container record but `leaving` lists the group. A stopped
    /// member has no endpoint Docker would show, so the records decide, not the network, and the
    /// network stays while one does not read.
    async fn drop_group_network(&self, group: &str, leaving: &str) -> Result<()> {
        let members = match self.db.list_complete::<Container>(CONTAINERS) {
            Ok(members) => members,
            Err(error) => {
                tracing::warn!("Keeping the network of the group {group}: {error:#}");
                return Ok(());
            }
        };
        if members
            .iter()
            .any(|other| other.id != leaving && other.spec.groups.iter().any(|g| g == group))
        {
            return Ok(());
        }
        let name = network(group);
        if let Some(info) = self.optional_inspect("network", &name).await?
            && info["Labels"][LABEL] == "true"
            && info["Labels"][GROUP_LABEL] == group
        {
            self.docker(&["network", "rm", &name]).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "groups_tests.rs"]
mod tests;
