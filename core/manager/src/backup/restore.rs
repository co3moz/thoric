//! The restore job: a backup read as it is uploaded (see `upload`), checked before anything is
//! created, its parts loaded one by one, and its records saved once the whole upload has arrived.
mod upload;

use super::{
    RESOURCE,
    format::{MANIFEST_LIMIT, Name, RECORDS_LIMIT, SMALL_LIMIT},
    installation,
    manifest::{Ending, Manifest, PartKind, Records},
    remap::{Loaded, leave_out_unlisted, not_created, remap, switch_off_disabled},
    review::{
        Present, check_manifest, check_records, conflict_message, files_reference, image_reference,
        parse_manifest, review,
    },
};
use crate::{
    App,
    archives::{arriving, check_upload, import_progress, transfer_permit},
    auth,
    compose::PINNED,
    desktop_mcp::ENDPOINTS,
    error::{Error, Result},
    jobs,
    model::{CONTAINERS, Container},
    outbound::{CREDENTIALS, ProxyChoice},
};
use anyhow::{Context, anyhow, bail};
use axum::{
    Json,
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::oneshot;
use tracing::Instrument;
use upload::{Input, Load, Sink};

/// `POST /api/backup/check`: compare a backup's manifest with this installation before its upload,
/// so the dashboard can name what a restore would create and what is in the way.
pub async fn check(
    State(app): State<Arc<App>>,
    Json(manifest): Json<Value>,
) -> Result<Json<Value>> {
    let manifest: Manifest =
        serde_json::from_value(manifest).map_err(|_| anyhow!("The file is not a Thoric backup"))?;
    check_manifest(&manifest)?;
    let present = app.present().await?;
    Ok(Json(json!(review(&manifest, &present))))
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Upload {
    /// The backup's exact size, or none for one streamed as it is made.
    #[serde(default)]
    pub size: Option<u64>,
}

/// `POST /api/backup/restore`: receive a backup and restore it. The answer comes once the whole
/// upload has arrived: the job's ID while it saves the records and creates the containers, or why
/// the backup was refused.
pub async fn restore(
    State(app): State<Arc<App>>,
    Query(upload): Query<Upload>,
    headers: HeaderMap,
    token: auth::Session,
    body: Body,
) -> Result<Json<Value>> {
    check_upload(&app, upload.size, &headers)?;
    if app.config.manager.is_none() {
        return Err(Error(
            StatusCode::CONFLICT,
            "Set MANAGER_CONTAINER; a restore creates volumes and containers through the manager's image and networks".into(),
        ));
    }
    let guard = installation(&app)?;
    let permit = transfer_permit(&app)?;
    let session = token.clone();
    let (accepted, was_accepted) = oneshot::channel();
    let span = tracing::Span::current();
    let (job_id, outcome) = jobs::start(
        app.clone(),
        "restore",
        RESOURCE.into(),
        false,
        move |app, job_id| async move {
            let _guard = guard;
            let _permit = permit;
            app.restore_backup(&job_id, upload.size, body, token, accepted)
                .instrument(span)
                .await
        },
    )?;
    tokio::select! {
        result = was_accepted => match result {
            Ok(Ok(())) => Ok(Json(json!({"job_id": job_id}))),
            Ok(Err(refused)) => Err(refused),
            // The job ended before it took the whole backup; its error says why.
            Err(_) => match outcome.await {
                Ok(Err(message)) => Err(Error(StatusCode::BAD_REQUEST, message)),
                _ => Err(Error(StatusCode::INTERNAL_SERVER_ERROR, "The restore ended unexpectedly".into())),
            },
        },
        _ = app.session_revoked(session) => {
            Err(Error(StatusCode::UNAUTHORIZED, "Session expired. Sign in again.".into()))
        }
    }
}

/// Tells the upload request that the whole backup arrived, or why it was refused before.
type Accepted = oneshot::Sender<Result<()>>;

/// What a restore created before its records were saved, removed again when it fails.
#[derive(Default)]
struct Created {
    tags: Vec<String>,
    volumes: Vec<String>,
}

impl App {
    /// What this installation has that a restore could collide with.
    pub(crate) async fn present(&self) -> anyhow::Result<Present> {
        let names = |bytes: Vec<u8>| -> HashSet<String> {
            String::from_utf8_lossy(&bytes)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.ends_with(":<none>"))
                .map(str::to_owned)
                .collect()
        };
        // By id, so a record this release cannot read still counts as taken. A container, an
        // image and a pinned compose are saved under their id, an endpoint under its name.
        Ok(Present {
            containers: self.db.ids(CONTAINERS)?.into_iter().collect(),
            images: self.db.ids("image")?.into_iter().collect(),
            composes: self.db.ids(PINNED)?.into_iter().collect(),
            endpoints: self.db.ids(ENDPOINTS)?.into_iter().collect(),
            docker_containers: names(self.docker(&["ps", "-a", "--format", "{{.Names}}"]).await?),
            networks: names(
                self.docker(&["network", "ls", "--format", "{{.Name}}"])
                    .await?,
            ),
            volumes: names(
                self.docker(&["volume", "ls", "--format", "{{.Name}}"])
                    .await?,
            ),
            tags: names(
                self.docker(&["image", "ls", "--format", "{{.Repository}}:{{.Tag}}"])
                    .await?,
            ),
        })
    }

    pub(super) async fn restore_backup(
        &self,
        job_id: &str,
        size: Option<u64>,
        body: Body,
        token: auth::Session,
        accepted: Accepted,
    ) -> anyhow::Result<Value> {
        self.plan_stages(job_id, &crate::stages::RESTORE_STAGES)?;
        self.begin_stage(job_id, "verify")?;
        self.append_log(job_id, &format!("Receiving a backup {}.", arriving(size)))?;
        self.set_progress(job_id, Some(import_progress(0, size)))?;
        let mut input = Input::new(self, job_id, size, body, token);
        let manifest = parse_manifest(
            &input
                .expect(Name::Manifest, MANIFEST_LIMIT, "manifest")
                .await?,
        )?;
        let records: Records = serde_json::from_slice(
            &input
                .expect(Name::Records, RECORDS_LIMIT, "records")
                .await?,
        )
        .context("The backup's records cannot be read")?;
        check_records(&manifest, &records)?;
        let review = review(&manifest, &self.present().await?);
        if !review.conflicts.is_empty() {
            let message = conflict_message(&review.conflicts);
            let _ = accepted.send(Err(Error(StatusCode::CONFLICT, message.clone())));
            bail!(message);
        }
        self.append_log(job_id, &describe(&manifest, &review.missing_volumes))?;
        let mut created = Created::default();
        let mut loaded = Loaded::default();
        let received = self
            .receive_parts(&mut input, &manifest, &records, &mut created, &mut loaded)
            .await;
        if let Err(error) = received {
            self.roll_back(job_id, &created).await;
            return Err(error);
        }
        // The whole backup is here; the upload request can end.
        let _ = accepted.send(Ok(()));
        self.set_progress(job_id, None)?;

        self.begin_stage(job_id, "records")?;
        let tokens = records
            .endpoints
            .iter()
            .filter(|endpoint| endpoint.token_hash.is_some())
            .map(|endpoint| endpoint.name.clone())
            .collect::<Vec<_>>();
        let saved = self.save_records(job_id, records, &loaded).await;
        let (records, skipped) = match saved {
            Ok(saved) => saved,
            Err(error) => {
                self.roll_back(job_id, &created).await;
                return Err(error);
            }
        };
        self.begin_stage(job_id, "containers")?;
        let manager = self
            .config
            .manager
            .clone()
            .context("MANAGER_CONTAINER is required")?;
        let mut made = Vec::new();
        let mut waiting = Vec::new();
        for container in &records.containers {
            let name = &container.spec.name;
            if let Some(reason) = skipped.get(&container.id) {
                self.append_log(job_id, &format!("\"{name}\": {reason}"))?;
                waiting.push(json!({"id": container.id, "name": name, "reason": reason}));
                continue;
            }
            let lock = self.resource_lock(&format!("container:{}", container.id));
            let _guard = lock.lock().await;
            match self.create_stopped(container, &manager).await {
                Ok(()) => {
                    self.append_log(
                        job_id,
                        &format!("Created \"{name}\" from its files. It is stopped."),
                    )?;
                    made.push(container.id.clone());
                }
                Err(error) => {
                    let reason = format!("{error:#}");
                    let mut failed = container.clone();
                    failed.last_error = Some(reason.clone());
                    self.db.put(CONTAINERS, &failed.id, &failed)?;
                    self.append_log(job_id, &format!("Could not create \"{name}\": {reason}"))?;
                    waiting.push(json!({"id": container.id, "name": name, "reason": reason}));
                }
            }
        }
        if !tokens.is_empty() {
            self.append_log(
                job_id,
                &format!(
                    "The MCP endpoints {} come without their tokens, so a token from the backup's installation opens nothing here. Give their agents new ones with Create token on the Endpoints page.",
                    tokens.join(", ")
                ),
            )?;
        }
        Ok(json!({
            "containers": records.containers.len(),
            "created": made,
            "not_created": waiting,
            "images": loaded.images.len(),
            "volumes": created.volumes,
            "endpoints": records.endpoints.iter().map(|e| &e.name).collect::<Vec<_>>(),
            "endpoints_without_tokens": tokens,
            "secrets_left_out": manifest.secrets_left_out,
            "missing_volumes": review.missing_volumes,
        }))
    }

    /// Load each part as it arrives, and read the backup's end.
    async fn receive_parts(
        &self,
        input: &mut Input<'_>,
        manifest: &Manifest,
        records: &Records,
        created: &mut Created,
        loaded: &mut Loaded,
    ) -> anyhow::Result<()> {
        for (index, part) in manifest.parts.iter().enumerate() {
            match part.kind {
                PartKind::Image => {
                    self.begin_stage(input.job_id, "images")?;
                    let record = records
                        .images
                        .iter()
                        .find(|record| record.id == part.target)
                        .context("Missing image record")?;
                    let lock = self.resource_lock(&format!("image:{}", record.id));
                    let _guard = lock.lock().await;
                    self.append_log(
                        input.job_id,
                        &format!("Loading the image \"{}\".", record.name),
                    )?;
                    let image_id = self.load_part(input, index).await?;
                    let reference = image_reference(&record.kind, &record.id);
                    self.docker(&["image", "tag", &image_id, &reference])
                        .await?;
                    created.tags.push(reference);
                    loaded.images.insert(record.id.clone(), image_id);
                }
                PartKind::Files => {
                    self.begin_stage(input.job_id, "files")?;
                    let container = records
                        .containers
                        .iter()
                        .find(|container| container.id == part.target)
                        .context("Missing container record")?;
                    let lock = self.resource_lock(&format!("container:{}", container.id));
                    let _guard = lock.lock().await;
                    self.append_log(
                        input.job_id,
                        &format!("Loading the files of \"{}\".", container.spec.name),
                    )?;
                    let image_id = self.load_part(input, index).await?;
                    let reference = files_reference(&container.id);
                    self.docker(&["image", "tag", &image_id, &reference])
                        .await?;
                    created.tags.push(reference);
                    loaded.files.insert(container.id.clone(), image_id);
                }
                PartKind::Volume => {
                    self.begin_stage(input.job_id, "volumes")?;
                    let name = &part.target;
                    let entry = manifest
                        .volumes
                        .iter()
                        .find(|volume| &volume.name == name)
                        .context("Missing volume")?;
                    let mut labels = Vec::new();
                    if let Some(owner) = &entry.owner {
                        labels.push(format!("{}={owner}", crate::volumes::OWNER));
                    }
                    if let Some(parameter) = &entry.parameter {
                        labels.push(format!("{}={parameter}", crate::volumes::PARAMETER));
                    }
                    let lock = self.resource_lock(&format!("volume:{name}"));
                    let _guard = lock.lock().await;
                    let mut writer = self
                        .write_volume_files(name, &labels)
                        .await
                        .map_err(|error| anyhow!(error.1))?;
                    created.volumes.push(name.clone());
                    self.append_log(input.job_id, &format!("Restoring the volume {name}."))?;
                    let streamed = input.part(index, Sink::Volume(&mut writer)).await;
                    match streamed {
                        Ok(_) => writer.finish(self).await?,
                        Err(error) => return Err(writer.fail(self, error).await),
                    }
                }
            }
        }
        let ending: Ending =
            serde_json::from_slice(&input.expect(Name::End, SMALL_LIMIT, "end").await?)
                .context("The backup's end cannot be read")?;
        if ending.parts != manifest.parts.len() || input.next().await?.is_some() {
            bail!("The backup's end does not match its manifest");
        }
        Ok(())
    }

    /// Load part `index`, a `docker save` stream, and return the image ID Docker gave it here.
    async fn load_part(&self, input: &mut Input<'_>, index: usize) -> anyhow::Result<String> {
        let mut load = Load::start(self)?;
        let streamed = input.part(index, Sink::Load(&mut load)).await;
        if let Err(error) = streamed {
            return Err(load.fail(error).await);
        }
        let loaded = load.finish().await?;
        let info = self.inspect("image", &loaded).await?;
        let image_id = info["Id"].as_str().context("Missing image ID")?.to_owned();
        if let Err(error) = crate::catalog::check_desktop(&info["Config"]["Labels"]) {
            // An untagged image is this restore's own; a tagged one was already on the host.
            if info["RepoTags"].as_array().is_none_or(Vec::is_empty) {
                let _ = self.docker(&["image", "rm", &image_id]).await;
            }
            return Err(error.context(format!("Part {index} of the backup")));
        }
        Ok(image_id)
    }

    /// Save the records as this installation keeps them, all together, and return them with the
    /// reason each container that is not created now waits.
    async fn save_records(
        &self,
        job_id: &str,
        records: Records,
        loaded: &Loaded,
    ) -> anyhow::Result<(Records, HashMap<String, String>)> {
        let catalog = crate::catalog::catalog();
        let mut loaded_here = Loaded {
            images: loaded.images.clone(),
            files: loaded.files.clone(),
            resolved: HashMap::new(),
        };
        // The same image ID, or the catalog image the container names, when Docker has it here.
        for container in &records.containers {
            let mut names = vec![&container.source_image_id];
            if catalog
                .iter()
                .any(|image| image.reference == container.spec.image)
            {
                names.push(&container.spec.image);
            }
            for name in names {
                if loaded_here.resolved.contains_key(name) {
                    continue;
                }
                if let Ok(info) = self.inspect("image", name).await
                    && let Some(id) = info["Id"].as_str()
                {
                    loaded_here.resolved.insert(name.clone(), id.to_owned());
                }
            }
        }
        let mut records = remap(records, &loaded_here);
        let volumes: HashSet<String> = String::from_utf8_lossy(
            &self
                .docker(&["volume", "ls", "--format", "{{.Name}}"])
                .await?,
        )
        .lines()
        .map(str::to_owned)
        .collect();
        // The volumes a request may mount here, as the volume list shows them.
        let listed: HashSet<String> = match self.config.volume_filter.as_deref() {
            None => volumes.clone(),
            Some(filter) => String::from_utf8_lossy(
                &self
                    .docker_args(&crate::volumes::list_args(Some(filter)))
                    .await?,
            )
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter_map(|volume| volume["Name"].as_str().map(str::to_owned))
            .collect(),
        };
        let passwords: HashSet<String> = records
            .proxy_passwords
            .iter()
            .map(|entry| entry.container_id.clone())
            .collect();
        let mut skipped = HashMap::new();
        for container in &mut records.containers {
            let name = container.spec.name.clone();
            let off = switch_off_disabled(&self.config, &mut container.spec);
            if !off.is_empty() {
                self.append_log(
                    job_id,
                    &format!(
                        "\"{name}\": switched off {}, which this installation disables.",
                        off.join(", ")
                    ),
                )?;
            }
            let left_out = leave_out_unlisted(container, &volumes, &listed);
            if !left_out.is_empty() {
                self.append_log(
                    job_id,
                    &format!(
                        "\"{name}\": left out the volumes {}, which VOLUME_FILTER does not list here.",
                        left_out.join(", ")
                    ),
                )?;
            }
            if crate::lifecycle::restored(&mut container.spec.lifecycle, crate::model::now()) {
                self.append_log(
                    job_id,
                    &format!(
                        "\"{name}\": its expiry passed while it was in the backup, so it was cleared and the container stays."
                    ),
                )?;
            }
            if let Some(reason) = not_created(container, &volumes) {
                container.last_error = Some(reason.clone());
                skipped.insert(container.id.clone(), reason);
            }
            // A password the backup left out is asked for again in the container's settings.
            if let Some(ProxyChoice::Custom { password, .. }) = &mut container.spec.proxy
                && !passwords.contains(&container.id)
            {
                *password = false;
            }
        }
        let mut rows: Vec<(&str, &str, Value)> = Vec::new();
        for image in &records.images {
            rows.push(("image", &image.id, serde_json::to_value(image)?));
        }
        for compose in &records.composes {
            rows.push((
                PINNED,
                &compose.container_id,
                serde_json::to_value(&compose.pinned)?,
            ));
        }
        for container in &records.containers {
            rows.push((CONTAINERS, &container.id, serde_json::to_value(container)?));
        }
        for endpoint in &records.endpoints {
            rows.push((ENDPOINTS, &endpoint.name, serde_json::to_value(endpoint)?));
        }
        for entry in &records.proxy_passwords {
            rows.push((
                CREDENTIALS,
                &entry.container_id,
                serde_json::to_value(&entry.credentials)?,
            ));
        }
        self.db
            .insert_all(&rows)
            .context("The records could not be saved; this installation gained one of their names during the restore")?;
        self.append_log(
            job_id,
            &format!(
                "Saved {} containers, {} saved images, {} composes and {} MCP endpoints.",
                records.containers.len(),
                records.images.len(),
                records.composes.len(),
                records.endpoints.len()
            ),
        )?;
        Ok((records, skipped))
    }

    /// Remove what a failed restore created: the tags it gave, which takes the image along when no
    /// other tag names it, and the volumes it made.
    async fn roll_back(&self, job_id: &str, created: &Created) {
        for reference in created.tags.iter().rev() {
            if self.docker(&["image", "rm", reference]).await.is_ok() {
                let _ = self.append_log(job_id, &format!("Removed {reference} again."));
            }
        }
        for volume in created.volumes.iter().rev() {
            if self.docker(&["volume", "rm", volume]).await.is_ok() {
                let _ = self.append_log(job_id, &format!("Removed the volume {volume} again."));
            }
        }
    }

    /// The image a reset, or a retry that creates a missing container again, starts it from: the
    /// source image ID its record keeps, or, when Docker no longer has it, as on an installation
    /// restored from a backup of another, the catalog image its settings name, downloaded when
    /// needed. Any other missing image is an error, and so is one that is not a Thoric desktop
    /// image of a level this manager handles; the container is left as it is.
    pub(crate) async fn source_image(
        &self,
        env: &mut Container,
        job_id: &str,
    ) -> anyhow::Result<()> {
        if let Ok(info) = self.inspect("image", &env.source_image_id).await {
            // A backup names the image by ID, which can be any image this host has.
            return crate::catalog::check_desktop(&info["Config"]["Labels"]).with_context(|| {
                format!(
                    "The image the container starts from, {}",
                    env.source_image_id
                )
            });
        }
        let Some(image) = crate::catalog::catalog()
            .into_iter()
            .find(|image| image.reference == env.spec.image)
        else {
            bail!("Docker has no image {}", env.source_image_id);
        };
        let _ = self.append_log(
            job_id,
            &format!(
                "The image the container started from is not on this installation; using {}, the catalog image it names.",
                image.name
            ),
        );
        self.ensure_catalog_image(&image.reference, job_id).await?;
        let info = self.inspect("image", &image.reference).await?;
        env.source_image_id = info["Id"].as_str().context("Missing image ID")?.to_owned();
        Ok(())
    }
}

/// What a restore brings, for its job log: counts and names, never a value.
fn describe(manifest: &Manifest, missing: &[String]) -> String {
    let mut text = format!(
        "The backup from Thoric {} holds {} containers, the files of {} of them, {} saved images, {} volumes and {} MCP endpoints.",
        manifest.version,
        manifest.containers.len(),
        manifest.containers.iter().filter(|c| c.files).count(),
        manifest.images.len(),
        manifest.volumes.iter().filter(|v| v.included).count(),
        manifest.endpoints.len(),
    );
    if !manifest.secrets_left_out.is_empty() {
        let names = manifest
            .containers
            .iter()
            .filter(|c| manifest.secrets_left_out.contains(&c.id))
            .map(|c| format!("\"{}\"", c.name))
            .collect::<Vec<_>>()
            .join(", ");
        text.push_str(&format!(
            " It left out the secret values of {names}: the values of their secret parameters, which a reset then runs their setup without, and the passwords of their own proxies, which their settings ask for again."
        ));
    }
    if !missing.is_empty() {
        text.push_str(&format!(
            " It does not carry the volumes {}, and this installation lacks them; the containers that mount them wait until they are here.",
            missing.join(", ")
        ));
    }
    text
}

#[cfg(test)]
#[path = "restore_tests.rs"]
mod tests;
