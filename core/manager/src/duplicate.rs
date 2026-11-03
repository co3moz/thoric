//! A container duplicated under a new name: a new container with the original's files and
//! settings, made in one job from what a snapshot, a creation and a volume copy already do.
//!
//! The original's files are committed as a snapshot, a saved image like any other on the Images
//! page, and the duplicate starts from it, so a reset returns the duplicate to the moment it was
//! made; the image stays while the duplicate's record starts from it (`image_removal`). Its
//! settings are the original's whole `spec` but the name and the machine identity, which a new
//! container gets of its own (`identity`), and what the installation has switched off since,
//! which is left out as a restore leaves it out. Its compose is pinned with the original's
//! values, secret ones too, and the password of its own proxy is kept for it, both copied record
//! to record and never through a route or a log. The setup's work is in the files, so a setup
//! that completed does not run again; one that did not runs when the duplicate starts.
//!
//! The volumes created for the original (`volumes::owned`) get copies created for the duplicate,
//! filled through the volume copy's pipe; every other named volume stays a mount both share. In
//! a group, the aliases another member is reached by are left out. MCP endpoints and their tokens
//! are not copied; the duplicate gets an endpoint of its own as a new container does. The desktop
//! MCP servers registered in the desktop user's folder are files there, and come along.
//!
//! The original is only read. Docker pauses it while it commits its files, so the job holds it
//! alone for the commit, as a snapshot does, and then shares it, as a script does, while the
//! volumes are copied. The name is reserved from the request until the job ends, so two
//! duplicates cannot take the same one. Until the new container stands in Docker, a failure takes
//! back what the job made: the container with its networks and records, the volumes it created
//! and the snapshot, unless a container was made from that snapshot meanwhile. A desktop that is
//! then not ready, or a setup that fails, leaves the duplicate as a failed creation leaves a
//! container, for a retry.
use crate::{
    App,
    compose::{PINNED, Pinned, Setup},
    error::{Error, Result},
    jobs::launch_named,
    model::{CONTAINERS, Container, ContainerSpec, ImageRecord, Mount, now},
    outbound::{CREDENTIALS, Credentials, ProxyChoice},
    stages::DUPLICATE_STAGES,
};
use anyhow::{Context, anyhow, bail};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

/// A duplicate's name and what is done with it once it is made.
#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Duplicate {
    /// The new container's name, which no other container has.
    pub name: String,
    /// Start the new container and wait for its desktop; otherwise it is left stopped.
    #[serde(default = "yes")]
    pub start: bool,
    /// Give the new container an MCP endpoint of its own, as a creation does. The original's
    /// endpoints are not copied.
    #[serde(default = "yes")]
    pub mcp_endpoint: bool,
}
fn yes() -> bool {
    true
}

/// Make a new container from a container's files and settings, under a new name. Returns a job.
pub async fn duplicate(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(request): Json<Duplicate>,
) -> Result<Json<Value>> {
    let original = app.container(&id)?;
    copied_spec(&original.spec, &request.name).validate()?;
    // Held by the job until it ends, so another duplicate cannot take the name meanwhile.
    let reserved = app
        .resource_lock(&format!("container-name:{}", name_key(&request.name)))
        .try_lock_owned()
        .map_err(|_| {
            Error(
                StatusCode::CONFLICT,
                format!(
                    "Another duplicate is making a container called \"{}\"; choose another name",
                    request.name
                ),
            )
        })?;
    name_free(&request.name, &app.db.list::<Container>(CONTAINERS)?)?;
    let new_id = uuid::Uuid::new_v4().simple().to_string();
    launch_named(
        app,
        "duplicate",
        format!("container:{new_id}"),
        Some(request.name.clone()),
        move |app, job_id| async move {
            let _reserved = reserved;
            let created = app
                .duplicate_container(&id, &new_id, &request, &job_id)
                .await;
            crate::api::creation_finished(&app, &new_id, created, request.mcp_endpoint, &job_id)
                .await
        },
    )
}

/// What a name is reserved and compared by: two names that differ only in case or in the spaces
/// around them are one name.
fn name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Refuse a name another container has.
pub(crate) fn name_free(name: &str, containers: &[Container]) -> Result<()> {
    if let Some(other) = containers
        .iter()
        .find(|other| name_key(&other.spec.name) == name_key(name))
    {
        return Err(Error(
            StatusCode::CONFLICT,
            format!(
                "A container called \"{}\" exists already; choose another name",
                other.spec.name
            ),
        ));
    }
    Ok(())
}

/// The original's settings for a duplicate called `name`: all of them but its name and its
/// machine identity, which the duplicate is given anew.
pub(crate) fn copied_spec(original: &ContainerSpec, name: &str) -> ContainerSpec {
    ContainerSpec {
        name: name.into(),
        hostname: None,
        mac_address: None,
        machine_id: None,
        ..original.clone()
    }
}

/// A volume created for the original, and the one created for the duplicate in its place.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Moved {
    pub from: String,
    pub to: String,
}

/// The name of the duplicate's copy of `volume`, a volume created for `original`: its name after
/// the duplicate's ID instead, `thoric-<container>-<parameter>` as a creation names one.
pub(crate) fn moved_name(volume: &str, original: &str, duplicate: &str) -> String {
    let rest = volume
        .strip_prefix(&format!("thoric-{original}-"))
        .unwrap_or(volume);
    let mut name = format!("thoric-{duplicate}-{rest}");
    // Volume names are ASCII, so any length is a character boundary.
    name.truncate(128);
    name
}

/// The volumes of `mounts` that were created for `original`, each once, with the names of their
/// copies.
pub(crate) fn moves(
    owned: &[String],
    mounts: &[Mount],
    original: &str,
    duplicate: &str,
) -> Vec<Moved> {
    let mut moves: Vec<Moved> = Vec::new();
    for mount in mounts {
        if owned.contains(&mount.volume) && !moves.iter().any(|m| m.from == mount.volume) {
            moves.push(Moved {
                from: mount.volume.clone(),
                to: moved_name(&mount.volume, original, duplicate),
            });
        }
    }
    moves
}

/// `volume`, or its copy when it is one of `moves`.
fn moved<'a>(moves: &'a [Moved], volume: &'a str) -> &'a str {
    moves
        .iter()
        .find(|moved| moved.from == volume)
        .map_or(volume, |moved| moved.to.as_str())
}

fn move_mounts(mounts: &mut [Mount], moves: &[Moved]) {
    for mount in mounts {
        mount.volume = moved(moves, &mount.volume).to_owned();
    }
}

/// Compose values with the volumes they name moved: a volume parameter's value is its volume.
fn move_values(values: &mut BTreeMap<String, String>, moves: &[Moved]) {
    for value in values.values_mut() {
        *value = moved(moves, value).to_owned();
    }
}

/// Leave out of `spec` the aliases another member of one of its groups is reached by, the
/// original's among them, and return them. Without groups an alias reaches nothing, and all stay.
pub(crate) fn free_aliases(
    id: &str,
    spec: &mut ContainerSpec,
    others: &[Container],
) -> Vec<String> {
    let taken = others
        .iter()
        .filter(|other| other.id != id && spec.groups.iter().any(|g| other.spec.groups.contains(g)))
        .flat_map(|other| crate::groups::names(&other.spec))
        .collect::<std::collections::HashSet<_>>();
    let (kept, dropped) = std::mem::take(&mut spec.aliases)
        .into_iter()
        .partition(|alias| !taken.contains(alias));
    spec.aliases = kept;
    dropped
}

/// The duplicate's setup: the original's, after the compose of the snapshot `image`, with its
/// volumes moved. One that completed stays completed, since its work is in the files. Any other
/// runs again from its first step when the duplicate starts, and waits for a retry when it is
/// left stopped.
pub(crate) fn copied_setup(original: &Setup, image: &str, moves: &[Moved], start: bool) -> Setup {
    let mut setup = original.clone();
    setup.image = image.into();
    move_mounts(&mut setup.mounts, moves);
    move_values(&mut setup.parameters, moves);
    if setup.status != "completed" {
        setup.status = if start { "pending" } else { "failed" }.into();
        setup.finished_at = None;
        for step in &mut setup.steps {
            step.status = "pending".into();
        }
    }
    setup
}

/// Why a duplicate left stopped has not run the setup its original did not finish.
const SETUP_WAITS: &str = "The compose setup had not completed in the container this one duplicates; retry to start this one and run it";

/// The duplicate's record: `id` with `spec` and the original's setup, created from the snapshot
/// `image`.
pub(crate) fn duplicate_record(
    original: &Container,
    id: &str,
    mut spec: ContainerSpec,
    image: &ImageRecord,
    moves: &[Moved],
    start: bool,
) -> Container {
    spec.image = image.reference.clone();
    move_mounts(&mut spec.mounts, moves);
    let container = format!("thoric-{id}");
    crate::identity::assign(&mut spec, &container);
    let setup = original
        .setup
        .as_ref()
        .map(|setup| copied_setup(setup, &image.id, moves, start));
    let last_error = setup
        .as_ref()
        .is_some_and(|setup| setup.status == "failed")
        .then(|| SETUP_WAITS.to_owned());
    Container {
        id: id.into(),
        container,
        private_network: format!("thoric-{id}-private"),
        egress_network: format!("thoric-{id}-egress"),
        source_image_id: image.image_id.clone(),
        created_at: now(),
        spec,
        last_error,
        desktop: image.desktop.clone(),
        working_image: None,
        setup,
    }
}

/// What a duplicate has made so far, which a failure takes back.
#[derive(Default)]
struct Made {
    image: Option<ImageRecord>,
    volumes: Vec<String>,
    record: Option<Container>,
}

/// What the original gives the duplicate beyond its record, read before its files are copied.
struct Plan {
    spec: ContainerSpec,
    moves: Vec<Moved>,
    pinned: Option<Pinned>,
    credentials: Option<Credentials>,
    /// The other volumes, which both mount.
    shared: Vec<String>,
    /// The aliases left out in a group, and the settings this installation switched off.
    aliases_left_out: Vec<String>,
    switched_off: Vec<&'static str>,
}

impl App {
    async fn duplicate_container(
        &self,
        original_id: &str,
        id: &str,
        request: &Duplicate,
        job_id: &str,
    ) -> anyhow::Result<Value> {
        self.plan_stages(job_id, &DUPLICATE_STAGES)?;
        self.begin_stage(job_id, "check")?;
        // Alone, as a snapshot holds it: Docker pauses the desktop while it commits, and work that
        // only uses it, such as a script or input, would fail on a paused one. Shared afterwards,
        // while a reset, a settings change or a delete still waits until its volumes are copied.
        let alone = self
            .resource_lock(&format!("container:{original_id}"))
            .lock_owned()
            .await;
        let original = self.container(original_id).map_err(|e| anyhow!(e.1))?;
        self.append_log(
            job_id,
            &format!(
                "Duplicating \"{}\" as \"{}\".",
                original.spec.name, request.name
            ),
        )?;
        let manager = self
            .config
            .manager
            .clone()
            .context("Set MANAGER_CONTAINER; the service must run in Docker")?;
        let manager_info = self.inspect("container", &manager).await?;
        if manager_info["Config"]["Labels"]["thoric.manager"] != "true" {
            bail!("MANAGER_CONTAINER does not point to the manager");
        }
        let docker = self.owned(&original).await.context(
            "The container's Docker container could not be read; it may be missing, which a retry or a reset repairs before a duplicate",
        )?;
        let plan = self
            .plan(&original, id, request, (&manager_info, &docker), job_id)
            .await?;
        let mut made = Made::default();
        let outcome = async {
            let image = self
                .save_snapshot(
                    job_id,
                    &original,
                    &docker,
                    request.name.clone(),
                    format!(
                        "The files of \"{}\" when \"{}\" was duplicated from them. Resetting \"{}\" returns it to them.",
                        original.spec.name, request.name, request.name
                    ),
                )
                .await?;
            made.image = Some(image.clone());
            let reading = alone.downgrade();
            self.copy_owned(job_id, id, &plan.moves, &mut made).await?;
            drop(reading);
            self.begin_stage(job_id, "create")?;
            let env = duplicate_record(
                &original,
                id,
                plan.spec.clone(),
                &image,
                &plan.moves,
                request.start,
            );
            self.save_duplicate(&env, &plan, &image, &manager_info)
                .await?;
            made.record = Some(env.clone());
            if request.start {
                self.provision(&env, &manager).await?;
            } else {
                self.create_stopped(&env, &manager).await?;
            }
            anyhow::Ok((env, image))
        }
        .await;
        let (env, image) = match outcome {
            Ok(standing) => standing,
            Err(error) => {
                self.take_back(job_id, &made).await;
                return Err(error);
            }
        };
        if request.start {
            self.begin_stage(job_id, "start")?;
            self.provisioned_ready(&env, job_id).await?;
            self.run_setup(id, job_id).await?;
        } else {
            self.append_log(job_id, "Created the container; it is stopped.")?;
        }
        Ok(json!({
            "container_id": id,
            "name": env.spec.name,
            "duplicated_from": original_id,
            "image_id": image.id,
            "volumes_copied": plan.moves,
            "volumes_shared": plan.shared,
            "aliases_left_out": plan.aliases_left_out,
            "switched_off": plan.switched_off,
            "started": request.start,
        }))
    }

    /// Read what the duplicate gets and check it before anything is made: its settings, the
    /// volumes to copy and to share, its compose values and its proxy's password. `docker` is
    /// `docker inspect` of the original's container.
    async fn plan(
        &self,
        original: &Container,
        id: &str,
        request: &Duplicate,
        (manager, docker): (&Value, &Value),
        job_id: &str,
    ) -> anyhow::Result<Plan> {
        let mut spec = copied_spec(&original.spec, &request.name);
        let off = crate::backup::switch_off_disabled(&self.config, &mut spec);
        if !off.is_empty() {
            self.append_log(
                job_id,
                &format!(
                    "Switched off {}, which this installation disables.",
                    off.join(", ")
                ),
            )?;
        }
        spec.validate()?;
        self.installation_allows(&spec)?;
        let mounts = original.mounts();
        crate::volumes::refuse_manager_volumes(manager, &mounts)?;
        let owned = self.owned_volumes(&original.id).await?;
        let moves = moves(&owned, &mounts, &original.id, id);
        let mut shared = Vec::new();
        for mount in &mounts {
            if !moves.iter().any(|m| m.from == mount.volume) && !shared.contains(&mount.volume) {
                shared.push(mount.volume.clone());
            }
        }
        for moved in &moves {
            self.append_log(
                job_id,
                &format!(
                    "The volume {} was created for the container, so it gets a copy of its own, {}.",
                    moved.from, moved.to
                ),
            )?;
        }
        if !shared.is_empty() {
            self.append_log(
                job_id,
                &format!(
                    "Both containers will mount the volumes {}.",
                    shared.join(", ")
                ),
            )?;
        }
        // What a creation checks before it downloads anything is checked before the commit, which
        // can take minutes: the volumes a request may mount here, and the desktop's level.
        let shared_of = |list: &[Mount]| {
            list.iter()
                .filter(|mount| shared.contains(&mount.volume))
                .cloned()
                .collect::<Vec<_>>()
        };
        let compose = shared_of(original.compose_mounts());
        self.chosen_volumes(
            &shared_of(&original.spec.mounts),
            &compose.iter().collect::<Vec<_>>(),
        )
        .await?;
        for moved in &moves {
            self.open_volume(&moved.from)
                .await
                .map_err(|error| anyhow!(error.1))?;
        }
        crate::catalog::check_desktop(&docker["Config"]["Labels"])?;
        let others = self.db.list::<Container>(CONTAINERS)?;
        let dropped = free_aliases(id, &mut spec, &others);
        if !dropped.is_empty() {
            self.append_log(
                job_id,
                &format!(
                    "Left out the aliases {}, which another member of its groups is reached by.",
                    dropped.join(", ")
                ),
            )?;
        }
        let mut named = spec.clone();
        crate::identity::assign(&mut named, &format!("thoric-{id}"));
        crate::groups::names_free(id, &named, &others)?;
        // Read now, while the original cannot be deleted; a record that is there but does not read
        // stops the duplicate rather than leave it without its values.
        let pinned = match self.db.contains(PINNED, &original.id)? {
            true => {
                let mut pinned: Pinned = self.db.get(PINNED, &original.id)?;
                move_values(&mut pinned.values, &moves);
                Some(pinned)
            }
            false => None,
        };
        let credentials = match &spec.proxy {
            Some(ProxyChoice::Custom { password: true, .. })
                if self.db.contains(CREDENTIALS, &original.id)? =>
            {
                Some(self.db.get(CREDENTIALS, &original.id)?)
            }
            _ => None,
        };
        Ok(Plan {
            spec,
            moves,
            pinned,
            credentials,
            shared,
            aliases_left_out: dropped,
            switched_off: off,
        })
    }

    /// Create the duplicate's own volumes and copy the original's into them, in the transfer
    /// slots archives and volume copies share.
    async fn copy_owned(
        &self,
        job_id: &str,
        id: &str,
        moves: &[Moved],
        made: &mut Made,
    ) -> anyhow::Result<()> {
        if moves.is_empty() {
            return Ok(());
        }
        self.begin_stage(job_id, "volumes")?;
        let _slot = self.archives.clone().acquire_owned().await?;
        for Moved { from, to } in moves {
            let _held = self.lock_volumes([to.as_str()]).await;
            let source = self.open_volume(from).await.map_err(|e| anyhow!(e.1))?;
            let info = self.inspect("volume", from).await?;
            let parameter = info["Labels"][crate::volumes::PARAMETER]
                .as_str()
                .unwrap_or_default();
            let target = self.volume_named(to).await.map_err(|e| anyhow!(e.1))?;
            if !self.container_volume(id, parameter, to).await? {
                bail!("The volume {to} exists already");
            }
            made.volumes.push(to.clone());
            self.append_log(
                job_id,
                &format!("Created the volume {to}. Copying the files of {from}."),
            )?;
            let copied = self.pipe(job_id, &source, &target).await?;
            self.append_log(job_id, &format!("Copied {copied} bytes of files."))?;
        }
        Ok(())
    }

    /// Save the duplicate's record with its compose values and its proxy's password, all together,
    /// under the locks of the volumes it mounts, as a creation saves its record.
    async fn save_duplicate(
        &self,
        env: &Container,
        plan: &Plan,
        image: &ImageRecord,
        manager: &Value,
    ) -> anyhow::Result<()> {
        let mounts = env.mounts();
        let _volumes = self
            .lock_volumes(mounts.iter().map(|mount| mount.volume.as_str()))
            .await;
        let shared = |mount: &&Mount| plan.shared.contains(&mount.volume);
        let compose = env
            .compose_mounts()
            .iter()
            .filter(shared)
            .collect::<Vec<_>>();
        let own = env
            .spec
            .mounts
            .iter()
            .filter(shared)
            .cloned()
            .collect::<Vec<_>>();
        self.chosen_volumes(&own, &compose).await?;
        crate::volumes::refuse_manager_volumes(manager, &mounts)?;
        let info = self.inspect("image", &image.image_id).await?;
        crate::catalog::check_desktop(&info["Config"]["Labels"])?;
        crate::docker::image_volumes_mounted(&info, &mounts)?;
        self.group_names_free(&env.id, &env.spec)?;
        // Again, since a creation or a rename may have taken the name while the files were copied.
        name_free(&env.spec.name, &self.db.list::<Container>(CONTAINERS)?)
            .map_err(|error| anyhow!(error.1))?;
        let mut rows: Vec<(&str, &str, Value)> = Vec::new();
        if let Some(pinned) = &plan.pinned {
            rows.push((PINNED, &env.id, serde_json::to_value(pinned)?));
        }
        if let Some(credentials) = &plan.credentials {
            rows.push((CREDENTIALS, &env.id, serde_json::to_value(credentials)?));
        }
        rows.push((CONTAINERS, &env.id, serde_json::to_value(env)?));
        self.db.insert_all(&rows)
    }

    /// Take back what a failed duplicate made: its container, networks and records, then the
    /// volumes it created and the snapshot. What cannot go is named in the log and kept.
    async fn take_back(&self, job_id: &str, made: &Made) {
        let _ = self.append_log(job_id, "Taking back what the duplicate made.");
        if let Some(env) = &made.record
            && let Err(error) = self.remove_container(env, job_id, false).await
        {
            // The record still names the volumes and the image, which stay with it, and says why.
            let reason = format!("The duplicate failed and could not be taken back: {error:#}");
            let _ = self.append_log(job_id, &reason);
            if let Ok(mut failed) = self.db.get::<Container>(CONTAINERS, &env.id) {
                failed.last_error = Some(reason);
                let _ = self.db.put(CONTAINERS, &env.id, &failed);
            }
            return;
        }
        for volume in made.volumes.iter().rev() {
            if self.docker(&["volume", "rm", volume]).await.is_ok() {
                let _ = self.append_log(job_id, &format!("Removed the volume {volume} again."));
            }
        }
        let Some(image) = &made.image else {
            // A commit whose record was not saved left its tag, which only this job names.
            let tag = format!("thoric/snapshot:{job_id}");
            if self.docker(&["image", "rm", &tag]).await.is_ok() {
                let _ = self.append_log(job_id, "Removed the snapshot again.");
            }
            return;
        };
        // The Images page lists the snapshot from its record on, so a container may have been
        // made from it meanwhile, whose reset needs it, as `image_removal` holds.
        match self.db.list_complete::<Container>(CONTAINERS) {
            Ok(containers) => {
                if let Some(other) = containers.iter().find(|other| {
                    other.source_image_id == image.image_id || other.spec.image == image.reference
                }) {
                    let _ = self.append_log(
                        job_id,
                        &format!(
                            "Kept the snapshot, which \"{}\" was created from.",
                            other.spec.name
                        ),
                    );
                    return;
                }
            }
            Err(error) => {
                let _ = self.append_log(job_id, &format!("Kept the snapshot: {error:#}"));
                return;
            }
        }
        if let Err(error) = self.db.remove("image", &image.id) {
            let _ = self.append_log(job_id, &format!("Kept the snapshot: {error:#}"));
            return;
        }
        match self.docker(&["image", "rm", &image.reference]).await {
            Ok(_) => {
                let _ = self.append_log(job_id, "Removed the snapshot again.");
            }
            Err(error) => {
                let _ = self.append_log(
                    job_id,
                    &format!("Kept the image {}: {error:#}", image.reference),
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "duplicate_tests.rs"]
mod tests;
