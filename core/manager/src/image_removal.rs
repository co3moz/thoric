//! Deleting an image. A saved image goes with its record: Docker removes the tag Thoric gave
//! it, and the image itself once no other tag names it and no other image or container uses it.
//! A catalog image loses only its download and stays listed, to be downloaded again. An image
//! registered from the Docker host (kind `import`) was there before Thoric, so only its record
//! goes. Never while a container record starts from it, unless another saved image's tag keeps
//! the same Docker image: a reset, a retry and a recreation create the container from its source
//! image again. The same rules decide whether the image a catalog download replaced goes. Nothing
//! is forced and nothing is pruned.
use crate::{
    App,
    catalog::CatalogImage,
    error::{Error, Result},
    model::{CONTAINERS, Container, ImageRecord, Job},
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc};

/// What the tags of script builds start with, which another manager on the same Docker makes too.
const BUILT: &str = "thoric/custom:";

/// `DELETE /api/images/{id}`, where `id` is a saved image's record id or a catalog id. It answers
/// at once, like deleting a volume: Docker removes an image in one call, and a refusal reaches the
/// caller as its status instead of a job log.
pub async fn remove(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<Json<Value>> {
    let lock = app.resource_lock(&format!("image:{id}"));
    // A download or a build of a catalog image holds its lock for minutes, and an agent's call
    // would time out behind it; the image is busy instead.
    let Ok(_guard) = lock.try_lock() else {
        return Err(Error(
            StatusCode::CONFLICT,
            "Another operation on this image is running; delete it once that finishes".into(),
        ));
    };
    let removed = match crate::catalog::find(&id) {
        Some(image) => app.delete_catalog_image(&image).await?,
        None => app.delete_saved_image(&id).await?,
    };
    Ok(Json(removed))
}

/// The container records that start from the Docker image `image_id`.
pub(crate) fn started_from<'a>(image_id: &str, containers: &'a [Container]) -> Vec<&'a Container> {
    containers
        .iter()
        .filter(|container| container.source_image_id == image_id)
        .collect()
}

impl App {
    /// Remove a catalog image's download. The caller holds the image's lock.
    async fn delete_catalog_image(&self, image: &CatalogImage) -> Result<Value> {
        if !self.image_present(&image.reference).await? {
            return Err(Error(
                StatusCode::NOT_FOUND,
                "This catalog image is not downloaded".into(),
            ));
        }
        let info = self.inspect("image", &image.reference).await?;
        let image_id = info["Id"].as_str().unwrap_or_default();
        self.refuse_used(image_id)?;
        self.refuse_set(&[&image.reference, image_id])?;
        let removed = self.remove_docker_image(&image.reference, image_id).await?;
        Ok(json!({"id": image.id, "name": image.name, "deleted": true, "image_removed": removed}))
    }

    /// Delete a saved image's record and, unless it was registered from the host, its Docker
    /// image. The caller holds the image's lock.
    async fn delete_saved_image(&self, id: &str) -> Result<Value> {
        let record: ImageRecord = self.db.get("image", id)?;
        // A scale set creates its instances from the image by the name it was given, whatever
        // keeps the Docker image.
        self.refuse_set(&[&record.id, &record.reference, &record.image_id])?;
        let removed = if record.kind == "import" {
            false
        } else {
            // Another record's tag keeps the same Docker image, such as an archive imported on
            // the host it came from, so the containers keep their source.
            let records = self.db.list::<ImageRecord>("image")?;
            if !shares_image(&record, &records) {
                self.refuse_used(&record.image_id)?;
            }
            // Thoric's own tag names the image. Once it is gone or names another image, the
            // image ID does, which Docker refuses while another tag still names it.
            let tagged = self
                .inspect("image", &record.reference)
                .await
                .is_ok_and(|info| info["Id"] == record.image_id.as_str());
            let target = if tagged {
                &record.reference
            } else {
                &record.image_id
            };
            self.remove_docker_image(target, &record.image_id).await?
        };
        self.db.remove("image", id)?;
        Ok(json!({"id": record.id, "name": record.name, "deleted": true, "image_removed": removed}))
    }

    /// Drop the tags of builds, snapshots and imports that got no record. Each tags its image
    /// with its job's ID before it saves the record of that ID, so a manager stopped in between
    /// leaves a tag that no record names. Run at startup. A tag stays while a record names it,
    /// while the job it is named after is queued or running, since that job's record is still to
    /// come, and while a container record starts from it, by name or by image, since a reset
    /// creates the container from it again. A build's tag also goes only when this database ran
    /// that build and saw it end, such as one an older manager left behind when it was cancelled:
    /// another manager on the same Docker, with a database of its own, builds into the same
    /// repository. The tags are listed first and the jobs read before the records, so a job that
    /// saves its record and ends meanwhile is seen running or has its record seen. Only Thoric's
    /// own repositories are looked at, and the image goes with its last tag, which is this
    /// orphan's own.
    pub(crate) async fn drop_orphan_tags(&self) -> anyhow::Result<Vec<String>> {
        let mut tags = Vec::new();
        for repository in ["thoric/custom", "thoric/snapshot", "thoric/imported"] {
            let listed = self
                .docker(&[
                    "image",
                    "ls",
                    "--no-trunc",
                    "--format",
                    "{{.Repository}}:{{.Tag}} {{.ID}}",
                    repository,
                ])
                .await?;
            tags.extend(String::from_utf8_lossy(&listed).lines().filter_map(|line| {
                let (tag, image) = line.trim().split_once(' ')?;
                (tag.starts_with(repository) && !tag.ends_with(":<none>"))
                    .then(|| (tag.to_owned(), image.to_owned()))
            }));
        }
        let running: HashSet<String> = self
            .db
            .active_jobs()?
            .into_iter()
            .map(|job| job.id)
            .collect();
        let ended_builds: HashSet<String> = tags
            .iter()
            .filter_map(|(tag, _)| tag.strip_prefix(BUILT))
            .filter(|id| {
                self.db.get::<Job>("job", id).is_ok_and(|job| {
                    job.kind == "build" && !matches!(job.status.as_str(), "queued" | "running")
                })
            })
            .map(str::to_owned)
            .collect();
        // A record that does not read may name a tag; nothing is dropped until all of them do.
        let records = self.db.list_complete::<ImageRecord>("image")?;
        let containers = self.db.list_complete::<Container>(CONTAINERS)?;
        let mut dropped = Vec::new();
        for (tag, image) in tags {
            let job = tag.rsplit_once(':').map_or("", |(_, job)| job);
            if (tag.starts_with(BUILT) && !ended_builds.contains(job))
                || records.iter().any(|record| record.reference == tag)
                || running.contains(job)
                || containers.iter().any(|container| {
                    container.spec.image == tag || container.source_image_id == image
                })
            {
                continue;
            }
            match self.docker(&["image", "rm", &tag]).await {
                Ok(_) => dropped.push(tag),
                Err(error) => tracing::debug!("Orphan tag {tag}: {error:#}"),
            }
        }
        Ok(dropped)
    }

    /// Remove the Docker image `replaced`, which a catalog reference named until a download moved
    /// the reference to a newer image, when nothing needs it any more; Docker would otherwise keep
    /// it without a name, 1 to 2 GB for each release. It stays while a container record starts from
    /// it, since a reset creates the container from it again, while a saved image's record names
    /// it, while any record does not read, since it may, and while a tag still names it, such as a
    /// release tag that was here before. Docker itself refuses it while a container uses it, since
    /// nothing is forced. Answers what happened, for the download's log; it never fails the download.
    pub(crate) async fn remove_replaced(&self, replaced: &str, reference: &str) -> String {
        let short = replaced.trim_start_matches("sha256:");
        let short = &short[..short.len().min(12)];
        let info = match self.inspect("image", replaced).await {
            Ok(info) => info,
            Err(error) if format!("{error:#}").contains("No such image") => {
                return format!("The image {reference} named before, {short}, is gone already.");
            }
            Err(error) => {
                tracing::debug!("Kept the replaced image {replaced}: {error:#}");
                return format!(
                    "The image {reference} named before, {short}, stays, since Docker did not describe it."
                );
            }
        };
        if let Some(tag) = info["RepoTags"].as_array().and_then(|tags| tags.first()) {
            return format!(
                "The image {reference} named before, {short}, stays, since {} names it too.",
                tag.as_str().unwrap_or_default()
            );
        }
        let containers = self.db.list_complete::<Container>(CONTAINERS);
        let records = self.db.list_complete::<ImageRecord>("image");
        let needed = match (&containers, &records) {
            (Ok(containers), Ok(records)) => {
                !started_from(replaced, containers).is_empty()
                    || records.iter().any(|record| record.image_id == replaced)
            }
            _ => true,
        };
        if needed {
            return format!(
                "The image {reference} named before, {short}, stays, since a container or a saved image was made from it."
            );
        }
        match self.remove_docker_image(replaced, replaced).await {
            Ok(true) => format!("Removed the image {reference} named before, {short}."),
            Ok(false) => format!(
                "Docker keeps the image {reference} named before, {short}, for the images built on it."
            ),
            Err(error) => {
                tracing::debug!("Kept the replaced image {replaced}: {}", error.1);
                format!(
                    "The image {reference} named before, {short}, stays, since a container or an image built on it uses it."
                )
            }
        }
    }

    /// Refuse while a container record starts from the Docker image `image_id`, and while one
    /// does not read, since it may.
    fn refuse_used(&self, image_id: &str) -> Result<()> {
        let containers = self.db.list_complete::<Container>(CONTAINERS)?;
        let users = started_from(image_id, &containers);
        if users.is_empty() {
            Ok(())
        } else {
            Err(Error(StatusCode::CONFLICT, in_use(&users)))
        }
    }

    /// Refuse while a scale set creates its instances from an image `names` names.
    fn refuse_set(&self, names: &[&str]) -> Result<()> {
        let sets = self
            .db
            .list_complete::<crate::scale::ScaleSet>(crate::scale::SETS)?;
        match crate::scale::starts_from(&sets, &crate::scale::templates(self)?, names) {
            Some(set) => Err(Error(
                StatusCode::CONFLICT,
                format!(
                    "The scale set {} creates its instances from this image; delete that set first",
                    set.name
                ),
            )),
            None => Ok(()),
        }
    }

    /// Remove `target`, a tag or an image ID, and tell whether Docker deleted the image `image_id`
    /// itself rather than only the tag. The containerd image store reports a `Deleted:` line for
    /// an image that a committed child, such as a snapshot, still names, and keeps it until the
    /// child goes; so the image is gone only once it no longer inspects either. An image that is
    /// gone already has nothing left to remove.
    async fn remove_docker_image(&self, target: &str, image_id: &str) -> Result<bool> {
        match self.docker(&["image", "rm", target]).await {
            Ok(output) => Ok(deleted_image(&String::from_utf8_lossy(&output))
                && self.inspect("image", image_id).await.is_err()),
            Err(error) if format!("{error:#}").contains("No such image") => Ok(false),
            Err(error) => Err(refused(error)),
        }
    }
}

/// Whether another saved image names the same Docker image as `record`.
fn shares_image(record: &ImageRecord, records: &[ImageRecord]) -> bool {
    records
        .iter()
        .any(|other| other.id != record.id && other.image_id == record.image_id)
}

/// Why an image stays while containers start from it, and what frees it.
fn in_use(users: &[&Container]) -> String {
    let names = users
        .iter()
        .map(|container| format!("\"{}\"", container.spec.name))
        .collect::<Vec<_>>()
        .join(", ");
    if users.len() == 1 {
        format!(
            "{names} was created from this image and needs it to reset; delete that container first"
        )
    } else {
        format!(
            "{names} were created from this image and need it to reset; delete those containers first"
        )
    }
}

/// Whether `docker image rm` reports deleting an image with a `Deleted:` line, rather than only
/// untagging it because another tag still names it or, on the classic store, a child needs it.
fn deleted_image(output: &str) -> bool {
    output.lines().any(|line| line.starts_with("Deleted:"))
}

/// Docker's refusal with a status: an image a container or another tag still holds is a conflict.
fn refused(error: anyhow::Error) -> Error {
    let message = format!("{error:#}");
    if message.contains("conflict") {
        Error(StatusCode::CONFLICT, message)
    } else {
        error.into()
    }
}

#[cfg(test)]
#[path = "image_removal_tests.rs"]
mod tests;
