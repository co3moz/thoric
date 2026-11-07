//! Deleting a named volume by itself, such as one a compose created for a container that was
//! deleted without it. Never the manager's own volume, and never one a container record mounts,
//! through its volume settings or its compose: a reset, retry or settings change of that container
//! would stop at the missing volume. One `VOLUME_FILTER` leaves out is answered as missing, as
//! opening it is. Docker refuses one that any other container still uses, so nothing is forced
//! and nothing is pruned.
use super::manager_volume;
use crate::{
    App,
    error::{Error, Result, missing},
    model::{CONTAINERS, Container, volume_name},
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use std::sync::Arc;

/// `DELETE /api/volumes/{name}`. It answers at once, like creating a volume: Docker removes a
/// volume in one quick call, and a refusal reaches the caller as its status instead of a job log.
pub async fn remove(State(app): State<Arc<App>>, Path(name): Path<String>) -> Result<Json<Value>> {
    volume_name(&name)?;
    let _guard = app.hold(&format!("volume:{name}")).await?;
    app.delete_volume(&name).await?;
    Ok(Json(json!({"name": name, "deleted": true})))
}

/// The first container record that mounts the volume `name`, through its volume settings or its
/// compose.
pub(crate) fn mounted_by<'a>(name: &str, containers: &'a [Container]) -> Option<&'a Container> {
    containers
        .iter()
        .find(|container| container.mounts().iter().any(|mount| mount.volume == name))
}

impl App {
    /// Delete the volume `name` and its files. The caller holds the volume's lock, which a
    /// creation holds too from its check of the volume until its record is saved, so the records
    /// read here name every container that mounts it.
    async fn delete_volume(&self, name: &str) -> Result<()> {
        // Without MANAGER_CONTAINER the manager runs outside Docker and mounts no volume; one that
        // names another container is refused rather than taken to mean the same.
        if self.config.manager.is_some() && manager_volume(&self.manager_container().await?, name) {
            return Err(Error(
                StatusCode::FORBIDDEN,
                "This is the manager's own volume; it cannot be deleted".into(),
            ));
        }
        self.optional_inspect("volume", name)
            .await?
            .filter(|info| self.lists_volume(info))
            .ok_or_else(missing)?;
        // A container record that does not read may mount it, so none may be left out.
        let containers = self.db.list_complete::<Container>(CONTAINERS)?;
        if let Some(container) = mounted_by(name, &containers) {
            return Err(Error(StatusCode::CONFLICT, mounted(name, container)));
        }
        // A scale set mounts it in the instances it creates from now on.
        let sets = self
            .db
            .list_complete::<crate::scale::ScaleSet>(crate::scale::SETS)?;
        if let Some(set) = crate::scale::mounting(&sets, &crate::scale::templates(self)?, name) {
            return Err(Error(
                StatusCode::CONFLICT,
                format!(
                    "The scale set {} mounts this volume in its instances; delete that set first",
                    set.name
                ),
            ));
        }
        self.docker(&["volume", "rm", name])
            .await
            .map_err(refused)?;
        Ok(())
    }
}

/// Why a volume a container record mounts stays, and what frees it: a volume of its settings can
/// be taken out there, one its compose was given only goes with the container.
fn mounted(name: &str, container: &Container) -> String {
    let owner = &container.spec.name;
    if container
        .spec
        .mounts
        .iter()
        .any(|mount| mount.volume == name)
    {
        format!(
            "\"{owner}\" mounts this volume; remove it from that container's volumes, or delete the container, first"
        )
    } else {
        format!("\"{owner}\" mounts this volume for its compose; delete that container first")
    }
}

/// Docker's refusal with a status: a volume a container uses is a conflict, one removed meanwhile
/// is gone.
fn refused(error: anyhow::Error) -> Error {
    let message = format!("{error:#}");
    if message.contains("volume is in use") {
        Error(StatusCode::CONFLICT, message)
    } else if message.contains("no such volume") {
        missing()
    } else {
        error.into()
    }
}

#[cfg(test)]
#[path = "remove_tests.rs"]
mod tests;
