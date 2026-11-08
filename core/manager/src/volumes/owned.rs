//! Volumes created for one container: a compose's volume parameter with `new_volume`, left
//! empty, gets `thoric-<container>-<parameter>`, which the container's creation job creates and
//! labels with its owner. A listing names that owner, or says it was deleted, and deleting the
//! container removes them too when asked, but never one that another container mounts.
use crate::{
    App,
    model::{Container, LABEL},
};
use anyhow::bail;
use serde_json::{Value, json};
use std::collections::HashMap;

/// The label naming the container a volume was created for, as on its containers and networks.
pub(crate) const OWNER: &str = "thoric.environment";
/// The label naming the compose parameter a volume was created for.
pub(crate) const PARAMETER: &str = "thoric.parameter";

/// The labels a volume for `container`'s `parameter` gets beside those of every volume the
/// manager creates.
pub(crate) fn owner_labels(container: &str, parameter: &str) -> Vec<String> {
    vec![
        format!("{OWNER}={container}"),
        format!("{PARAMETER}={parameter}"),
    ]
}

/// Whether `docker volume inspect` shows a volume the manager created for `container`.
pub(crate) fn owned_by(info: &Value, container: &str) -> bool {
    info["Labels"][LABEL] == "true" && info["Labels"][OWNER] == container
}

impl App {
    /// Create the volume `name` for `container`'s `parameter`, and say whether it was created: a
    /// volume of that name that is already the container's own is kept, any other one refused.
    /// The caller holds the volume's lock.
    pub(crate) async fn container_volume(
        &self,
        container: &str,
        parameter: &str,
        name: &str,
    ) -> anyhow::Result<bool> {
        if let Some(info) = self.optional_inspect("volume", name).await? {
            if owned_by(&info, container) {
                return Ok(false);
            }
            bail!(
                "A volume named {name} already exists and was not created for this container; choose a volume for {parameter} instead"
            );
        }
        let created = self
            .make_volume(name, &owner_labels(container, parameter))
            .await?;
        if !owned_by(&created, container) {
            bail!("Volume was created by another operation; ownership does not match");
        }
        Ok(true)
    }

    /// The volumes created for `container`, found by their labels and each checked again.
    pub(crate) async fn owned_volumes(&self, container: &str) -> anyhow::Result<Vec<String>> {
        let listed = self
            .docker(&[
                "volume",
                "ls",
                "--filter",
                &format!("label={LABEL}=true"),
                "--filter",
                &format!("label={OWNER}={container}"),
                "--format",
                "{{.Name}}",
            ])
            .await?;
        let mut names = Vec::new();
        for name in String::from_utf8_lossy(&listed).lines() {
            if !name.is_empty() && owned_by(&self.inspect("volume", name).await?, container) {
                names.push(name.to_owned());
            }
        }
        names.sort();
        Ok(names)
    }
}

/// What deleting a container did with the volumes created for it, when asked to delete them.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct RemovedVolumes {
    pub deleted: Vec<String>,
    /// Those another container mounts, or that Docker refused to remove.
    pub kept: Vec<String>,
}

/// Of the volumes created for a container, those that can go with it, and those another
/// container mounts, each with the first such container, which keep them.
pub(crate) fn removable(
    owned: Vec<String>,
    others: &[Container],
) -> (Vec<String>, Vec<(String, &Container)>) {
    let mut free = Vec::new();
    let mut shared = Vec::new();
    for name in owned {
        match super::mounted_by(&name, others) {
            Some(other) => shared.push((name, other)),
            None => free.push(name),
        }
    }
    (free, shared)
}

impl App {
    /// Of `volumes`, those a scale set shares in the instances it creates from now on, each
    /// with the set's name: they stay when the container that made them goes, also as its last
    /// instance. Every set and template is read, since a record that does not read may be the
    /// one that shares a volume.
    pub(crate) fn shared_by_sets(
        &self,
        volumes: &[String],
    ) -> anyhow::Result<std::collections::HashMap<String, String>> {
        if volumes.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let sets = self
            .db
            .list_complete::<crate::scale::ScaleSet>(crate::scale::SETS)?;
        let templates = crate::scale::templates(self)?;
        Ok(volumes
            .iter()
            .filter_map(|name| {
                crate::scale::mounting(&sets, &templates, name)
                    .map(|set| (name.clone(), set.name.clone()))
            })
            .collect())
    }
}

/// Add `owner` to each line of `docker volume ls`: for a volume created for a container, its
/// `container_id`, the `parameter` it was created for, and the container's `name` while its
/// record exists, or `deleted`; `null` for any other volume.
pub(crate) fn mark_owners(volumes: &mut [Value], containers: &[Container]) {
    for volume in volumes {
        let owner = volume["Labels"].as_str().and_then(|text| {
            let labels = labels(text);
            if labels.get(LABEL) != Some(&"true") {
                return None;
            }
            let id = *labels.get(OWNER)?;
            let record = containers.iter().find(|container| container.id == id);
            Some(json!({
                "container_id": id,
                "parameter": labels.get(PARAMETER),
                "name": record.map(|container| &container.spec.name),
                "deleted": record.is_none(),
            }))
        });
        if let Value::Object(fields) = volume {
            fields.insert("owner".into(), owner.unwrap_or(Value::Null));
        }
    }
}

/// The labels of a `docker volume ls` line, which joins them as `key=value` with commas.
fn labels(text: &str) -> HashMap<&str, &str> {
    text.split(',')
        .filter_map(|pair| pair.split_once('='))
        .collect()
}

#[cfg(test)]
#[path = "owned_tests.rs"]
mod tests;
