//! A container's life in Docker: what the installation allows it, its creation from a request,
//! provisioning its networks and its container, a retry that repairs it, a reset to its source
//! image and its deletion. Each checks what it finds against the records before it changes it,
//! and one that starts the desktop ends once the desktop is ready (`desktop_ready`).
use super::{create_args::TUN_DEVICE, isolation::Reach, private_subnet};
use crate::{App, model::*, volumes::RemovedVolumes};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

impl App {
    /// Settings the installation has switched off cannot be chosen for a desktop. A desktop that
    /// has one keeps it in its settings, also when it is created again; the manager only refuses
    /// to carry what it carries itself, the audio channels and gamepad input, while it is off.
    pub(crate) fn installation_allows(&self, spec: &ContainerSpec) -> Result<()> {
        if spec.internet && !self.config.internet {
            bail!(
                "Internet access is disabled for this installation (INTERNET_ENABLED=false); use the container without internet"
            );
        }
        if spec.sudo && !self.config.sudo {
            bail!(
                "sudo is disabled for this installation (SUDO_ENABLED=false); use the container without sudo"
            );
        }
        if spec.gamepad && !self.config.gamepad {
            bail!("The virtual gamepad is disabled for this installation (GAMEPAD_ENABLED=false)");
        }
        if spec.vpn && !self.config.vpn {
            bail!("VPN access is disabled for this installation (VPN_ENABLED=false)");
        }
        if let Some(refusal) = crate::permissions::refusal(&self.config, spec) {
            bail!(refusal);
        }
        if spec.proxy == Some(crate::outbound::ProxyChoice::Installation)
            && self.config.proxy.is_none()
        {
            bail!(
                "This installation has no proxy (PROXY_URL); give the container a proxy of its own, or none"
            );
        }
        if spec.audio && !self.config.audio {
            bail!("Audio is disabled for this installation");
        }
        if spec.microphone && !self.config.microphone {
            bail!("Microphone is disabled for this installation");
        }
        if spec.gpu
            && let Some(refusal) = crate::gpu::refusal(&self.config)
        {
            bail!(refusal);
        }
        Ok(())
    }
    /// A Docker error from creating or starting a desktop, with the setting behind the device it
    /// names.
    pub(crate) fn device_error(&self, env: &Container, error: anyhow::Error) -> anyhow::Error {
        if env.spec.vpn && format!("{error:#}").contains(TUN_DEVICE) {
            return error.context(
                "VPN access needs /dev/net/tun on the Docker host, which this host does not have; use the container without vpn",
            );
        }
        let error = crate::permissions::explain(&env.spec, error);
        if env.spec.gpu {
            crate::gpu::explain(error, &crate::gpu::routes(&self.config).used)
        } else {
            error
        }
    }
    pub async fn create_container(
        &self,
        id: &str,
        spec: ContainerSpec,
        parameters: std::collections::BTreeMap<String, String>,
        job_id: &str,
    ) -> Result<Value> {
        spec.validate()?;
        self.installation_allows(&spec)?;
        let manager = self
            .config
            .manager
            .as_ref()
            .context("Set MANAGER_CONTAINER; the service must run in Docker")?;
        let manager_info = self.inspect("container", manager).await?;
        if manager_info["Config"]["Labels"]["thoric.manager"] != "true" {
            bail!("MANAGER_CONTAINER does not point to the manager");
        }
        // Cheap checks come before a catalog image download that can take minutes. The volumes
        // are checked again under their locks before the record is saved.
        crate::volumes::refuse_manager_volumes(&manager_info, &spec.mounts)?;
        self.chosen_volumes(&spec.mounts, &[]).await?;
        self.ensure_catalog_image(&spec.image, job_id).await?;
        let image = self
            .inspect("image", &spec.image)
            .await
            .context("Image not available. Build or pull it first")?;
        if image["Config"]["Labels"]["thoric.desktop"] != "true" {
            bail!("Image must be derived from a Thoric desktop image");
        }
        crate::catalog::check_level(&image["Config"]["Labels"])?;
        let image_id = image["Id"].as_str().context("Missing image ID")?;
        let compose = self.pin_compose(id, &spec.image, image_id, &parameters)?;
        let compose_mounts = compose
            .as_ref()
            .map(|(_, pinned)| pinned.mounts())
            .unwrap_or_default();
        crate::compose::check_mounts(&spec.mounts, &compose_mounts)?;
        // An image's compose names its default volume; it cannot hand a desktop the manager's
        // database either.
        crate::volumes::refuse_manager_volumes(&manager_info, &compose_mounts)?;
        let own_volumes = compose
            .as_ref()
            .map(|(_, pinned)| pinned.new_volumes(id))
            .unwrap_or_default();
        let chosen = compose_mounts
            .iter()
            .filter(|mount| !own_volumes.iter().any(|(_, own)| *own == mount.volume))
            .collect::<Vec<_>>();
        let all_mounts = spec
            .mounts
            .iter()
            .chain(&compose_mounts)
            .cloned()
            .collect::<Vec<_>>();
        // The volumes' locks are held from this check until the record is saved, as a delete holds
        // them from its look at the records until the removal: the delete either sees this record
        // and keeps the volume, or removed it first, and then this check stops the creation.
        let volumes = self
            .lock_volumes(all_mounts.iter().map(|mount| mount.volume.as_str()))
            .await;
        self.chosen_volumes(&spec.mounts, &chosen).await?;
        image_volumes_mounted(&image, &all_mounts)?;
        let container = format!("thoric-{id}");
        let mut spec = spec;
        // A container from a snapshot gets an identity of its own, not the source container's.
        crate::identity::assign(&mut spec, &container);
        self.group_names_free(id, &spec)?;
        let env = Container {
            id: id.into(),
            container,
            private_network: format!("thoric-{id}-private"),
            egress_network: format!("thoric-{id}-egress"),
            source_image_id: image_id.into(),
            spec,
            created_at: now(),
            last_error: None,
            desktop: crate::catalog::from_image(&image),
            working_image: None,
            setup: compose.as_ref().map(|(image, pinned)| pinned.setup(image)),
        };
        // The container's own volumes are made before its record, so no record names one that
        // was never made; a missing one later was removed, and the rule for missing volumes holds.
        // The other way round too: a volume made here goes again when the record cannot be
        // saved, so no volume names an owner that never existed.
        let mut created = Vec::new();
        let recorded = async {
            for (parameter, volume) in &own_volumes {
                if self.container_volume(id, parameter, volume).await? {
                    created.push(volume.to_string());
                    self.append_log(
                        job_id,
                        &format!("Created the volume {volume} for {parameter}."),
                    )?;
                }
            }
            // Save intent first: failed setup and interrupted jobs remain discoverable and removable.
            if let Some((_, pinned)) = &compose {
                self.save_pinned(id, pinned)?;
            }
            self.db.put(CONTAINERS, id, &env)
        }
        .await;
        if let Err(error) = recorded {
            for volume in &created {
                if self.docker(&["volume", "rm", volume]).await.is_ok() {
                    let _ = self.append_log(job_id, &format!("Removed the volume {volume} again."));
                }
            }
            return Err(error);
        }
        drop(volumes);
        let result = self.provision(&env, manager).await;
        if let Err(ref e) = result {
            let mut failed = env.clone();
            failed.last_error = Some(format!("{e:#}"));
            self.db.put(CONTAINERS, id, &failed)?;
            // Keep intent and any partial resources for explicit retry/removal; never touch unrelated objects.
        }
        result?;
        self.provisioned_ready(&env, job_id).await?;
        self.run_setup(id, job_id).await?;
        Ok(json!({"container_id":id}))
    }
    pub async fn retry_container(&self, id: &str, job_id: &str) -> Result<Value> {
        let mut env = self.container(id).map_err(|e| anyhow::anyhow!(e.1))?;
        let manager = self
            .config
            .manager
            .as_deref()
            .context("MANAGER_CONTAINER is required")?;
        let info = self.inspect("container", manager).await?;
        if info["Config"]["Labels"]["thoric.manager"] != "true" {
            bail!("Manager label does not match");
        }
        let existing = self.optional_inspect("container", &env.container).await?;
        if existing.is_none() && env.working_image.is_none() {
            // A settings change set the container aside before it recorded the commit, so that
            // container may hold the only copy of the files; never replace it with an empty one.
            let previous = format!("{}-previous", env.container);
            if self
                .optional_inspect("container", &previous)
                .await?
                .is_some()
            {
                bail!("{previous} {}", crate::reconfigure::PREVIOUS_LEFT);
            }
            // A container recreated from the source image lacks what the compose setup changed.
            crate::compose::setup_again(&mut env);
            // One restored from another installation's backup may name that installation's image
            // ID; the catalog image it names is downloaded instead. A desktop with the manager's
            // own volume is refused below, before anything is downloaded, and an image that is
            // still missing is reported there and kept on the record.
            if crate::volumes::refuse_manager_volumes(&info, &env.mounts()).is_ok()
                && let Err(error) = self.source_image(&mut env, job_id).await
            {
                let _ = self.append_log(job_id, &format!("{error:#}"));
            }
        }
        crate::identity::adopt(&mut env, existing.as_ref());
        self.db.put(CONTAINERS, id, &env)?;
        let result = self.provision(&env, manager).await;
        env.last_error = result.as_ref().err().map(|e| format!("{e:#}"));
        self.db.put(CONTAINERS, id, &env)?;
        result?;
        // The container stands, so one a settings change set aside has finished its part. With
        // the commit on record, the files are in the container created from it.
        if env.working_image.is_some() {
            self.remove_previous(&env, job_id).await?;
        }
        // Also a desktop that kept restarting before.
        self.provisioned_ready(&env, job_id).await?;
        // Also a setup that failed or was interrupted earlier.
        self.run_setup(id, job_id).await?;
        Ok(json!({"container_id":id}))
    }
    pub(crate) async fn provision(&self, env: &Container, manager: &str) -> Result<()> {
        self.provision_and(env, manager, true).await
    }
    /// Wait for the desktop of a container this job has provisioned. The job shares the container
    /// while it waits, beside input, files and scripts, unless a compose setup follows, which
    /// runs with the container alone. A desktop that is not ready fails the job, and the job's
    /// error says why; the container keeps it only when its compose setup was due, since that did
    /// not run and a retry runs it. Any other such error would outlive a desktop that comes up
    /// later.
    pub(crate) async fn provisioned_ready(&self, env: &Container, job_id: &str) -> Result<()> {
        if !crate::compose::setup_due(env) {
            crate::jobs::share_from_here();
            return self.desktop_ready(env, job_id).await.map(drop);
        }
        let ready = self
            .desktop_ready(env, job_id)
            .await
            .context("The compose setup did not run");
        if let Err(error) = &ready {
            let mut failed: Container = self.db.get(CONTAINERS, &env.id)?;
            failed.last_error = Some(format!("{error:#}"));
            self.db.put(CONTAINERS, &env.id, &failed)?;
        }
        ready.map(drop)
    }
    /// Create a container its record describes, with its networks, and leave it stopped, as a
    /// restore does.
    pub(crate) async fn create_stopped(&self, env: &Container, manager: &str) -> Result<()> {
        self.provision_and(env, manager, false).await
    }
    async fn provision_and(&self, env: &Container, manager: &str, start: bool) -> Result<()> {
        // Validate all existing resources before creating or starting anything.
        env.spec.validate()?;
        // A GPU desktop without a GPU exits at its renderer check and restarts forever.
        if env.spec.gpu
            && let Some(refusal) = crate::gpu::refusal(&self.config)
        {
            bail!(refusal);
        }
        let manager_info = self.inspect("container", manager).await?;
        let mounts = env.mounts();
        crate::volumes::refuse_manager_volumes(&manager_info, &mounts)?;
        // These were chosen when the container was created, so a volume VOLUME_FILTER no longer
        // lists stays mounted. Also a volume created for this container: one that is gone may
        // have held its work.
        for mount in &mounts {
            self.inspect("volume", &mount.volume)
                .await
                .with_context(|| {
                    format!(
                        "Configured volume {} is missing; setup stopped without creating an empty replacement",
                        mount.volume
                    )
                })?;
        }
        let existing = self.optional_inspect("container", &env.container).await?;
        if existing.is_some() {
            let owned = self.owned(env).await?;
            if owned["Image"] != env.container_image() {
                bail!("Container image does not match");
            }
        } else {
            let image = self.inspect("image", env.container_image()).await.context(
                "Container image is missing; setup stopped without changing the saved image ID",
            )?;
            // A record restored from a backup names its image by ID, which can be any image here.
            crate::catalog::check_desktop(&image["Config"]["Labels"])?;
        }
        for (name, internal) in [(&env.private_network, true), (&env.egress_network, false)] {
            if let Some(info) = self.optional_inspect("network", name).await? {
                Self::check_network(env, &info, internal)?;
            }
        }
        let private = self
            .private_network(env, &manager_info, existing.as_ref())
            .await?;
        self.ensure_network(env, &env.egress_network, Reach::Internet)
            .await?;
        let manager_id = manager_info["Id"].as_str().context("Missing manager ID")?;
        // The network's own list, since a private network made again has the manager on it.
        if private["Containers"].get(manager_id).is_none() {
            self.docker(&[
                "network",
                "connect",
                private["Id"].as_str().context("Missing network ID")?,
                manager_id,
            ])
            .await?;
        }
        if let Some(info) = existing {
            let container_id = info["Id"].as_str().context("Missing container ID")?;
            if info["NetworkSettings"]["Networks"]
                .get(&env.private_network)
                .is_none()
            {
                self.connect_network(
                    env,
                    private["Id"].as_str().context("Missing network ID")?,
                    container_id,
                )
                .await?;
            }
            let mut saved = env.clone();
            self.set_internet(&mut saved, env.spec.internet).await?;
            self.join_groups(env, &info).await?;
            if start && info["State"]["Running"] != true {
                // Also a container an earlier attempt created without its machine ID.
                self.write_machine_id(env).await?;
                self.hand_over_mount_parents(env).await?;
                self.docker(&["start", container_id])
                    .await
                    .map_err(|error| self.device_error(env, error))?;
            }
            return Ok(());
        }
        let args = self.container_args(env, private_subnet(&private));
        self.docker_args(&args)
            .await
            .map_err(|error| self.device_error(env, error))?;
        self.write_machine_id(env).await?;
        self.hand_over_mount_parents(env).await?;
        if env.spec.internet {
            self.connect_network(env, &env.egress_network, &env.container)
                .await?;
        }
        if !env.spec.groups.is_empty() {
            let info = self.inspect("container", &env.container).await?;
            self.join_groups(env, &info).await?;
        }
        if start {
            self.docker(&["start", &env.container])
                .await
                .map_err(|error| self.device_error(env, error))?;
        }
        Ok(())
    }
    /// Delete a container and its networks. Named volumes stay, except with `volumes` those
    /// created for it that no other container mounts.
    pub(crate) async fn remove_container(
        &self,
        env: &Container,
        job_id: &str,
        volumes: bool,
    ) -> Result<RemovedVolumes> {
        self.append_log(
            job_id,
            &format!("Checking ownership before deleting \"{}\".", env.spec.name),
        )?;
        let container = match self.optional_inspect("container", &env.container).await? {
            Some(_) => Some(self.owned(env).await?),
            None => None,
        };
        let container_id = container.as_ref().and_then(|v| v["Id"].as_str());
        let manager_info = if let Some(name) = &self.config.manager {
            let m = self.inspect("container", name).await?;
            if m["Config"]["Labels"]["thoric.manager"] != "true" {
                bail!("Manager ownership does not match");
            }
            Some(m)
        } else {
            None
        };
        let manager = match &manager_info {
            Some(m) => Some(m["Id"].as_str().context("Missing manager ID")?.to_owned()),
            None => None,
        };
        // Listed before anything is deleted; whether another container mounts one is read just
        // before the removal.
        let mut owned = Vec::new();
        if volumes {
            owned = self.owned_volumes(&env.id).await?;
            // A label alone never hands over the manager's own volumes.
            owned.retain(|name| {
                !manager_info
                    .as_ref()
                    .is_some_and(|m| crate::volumes::manager_volume(m, name))
            });
        }
        let mut networks = Vec::new();
        // Preflight ownership and every endpoint BEFORE deleting the container's writable layer.
        for (name, internal) in [(&env.private_network, true), (&env.egress_network, false)] {
            if let Some(info) = self.optional_inspect("network", name).await? {
                Self::check_network(env, &info, internal)?;
                if let Some(members) = info["Containers"].as_object() {
                    for id in members.keys() {
                        if Some(id.as_str()) != container_id && Some(id) != manager.as_ref() {
                            bail!("Network has another container; deletion stopped ({name})");
                        }
                    }
                }
                networks.push(info);
            }
        }
        if let Some(id) = container_id {
            self.docker(&["rm", "--force", id]).await?;
            self.append_log(job_id, "Removed the container and its writable layer.")?;
        } else {
            self.append_log(job_id, "Desktop container was already absent.")?;
        }
        self.remove_previous(env, job_id).await?;
        for info in networks {
            let id = info["Id"].as_str().context("Missing network ID")?;
            let role = if info["Internal"] == true {
                "private"
            } else {
                "internet"
            };
            if let Some(manager) = &manager
                && info["Containers"].get(manager).is_some()
            {
                self.docker(&["network", "disconnect", "--force", id, manager])
                    .await?;
                self.append_log(
                    job_id,
                    &format!("Disconnected the manager from the {role} network."),
                )?;
            }
            self.docker(&["network", "rm", id]).await?;
            self.append_log(job_id, &format!("Removed the container's {role} network."))?;
        }
        // A group whose last member this was loses its network; the others keep theirs.
        if let Err(error) = self.leave_groups(env).await {
            self.append_log(job_id, &format!("Keeping a group's network: {error:#}"))?;
        }
        // The records are read under the volumes' locks, which a creation holds from its check of
        // the volumes until its record is saved: a container saved meanwhile is seen and keeps
        // the volume, and one still waiting for the lock finds the volume gone and stops.
        let mut removed = RemovedVolumes::default();
        let held = self.lock_volumes(owned.iter().map(String::as_str)).await;
        // Every other record, since one that does not read may mount the same volume: then all
        // of them stay, and the rest of the deletion goes on.
        let others = if owned.is_empty() {
            Vec::new()
        } else {
            match self.db.list_complete::<Container>(CONTAINERS) {
                Ok(others) => others,
                Err(error) => {
                    self.append_log(
                        job_id,
                        &format!("Keeping the volumes created for it: {error:#}"),
                    )?;
                    removed.kept.append(&mut owned);
                    Vec::new()
                }
            }
        };
        let others = others
            .into_iter()
            .filter(|other| other.id != env.id)
            .collect::<Vec<_>>();
        let (doomed, shared) = crate::volumes::removable(owned, &others);
        for (name, other) in shared {
            self.append_log(
                job_id,
                &format!(
                    "Keeping the volume {name}: \"{}\" mounts it too.",
                    other.spec.name
                ),
            )?;
            removed.kept.push(name);
        }
        // The container is gone, so Docker refuses only a volume something else still uses.
        for name in doomed {
            match self.docker(&["volume", "rm", &name]).await {
                Ok(_) => {
                    self.append_log(job_id, &format!("Deleted the volume {name}."))?;
                    removed.deleted.push(name);
                }
                Err(error) => {
                    self.append_log(job_id, &format!("Keeping the volume {name}: {error:#}"))?;
                    removed.kept.push(name);
                }
            }
        }
        drop(held);
        if let Some(image) = &env.working_image {
            self.release_working_image(image, job_id).await?;
        }
        self.remove_container_records(&env.id)?;
        self.forget_pinned(&env.id)?;
        self.forget_proxy_password(&env.id)?;
        self.db.remove(CONTAINERS, &env.id)?;
        crate::desktop_mcp::forget_container(self, &env.id)?;
        self.human.end_container(&env.id, &self.events);
        self.append_log(
            job_id,
            &if removed.deleted.is_empty() {
                format!(
                    "Deleted \"{}\". Named volumes and saved images were preserved.",
                    env.spec.name
                )
            } else {
                format!(
                    "Deleted \"{}\" and the volumes created for it. Other named volumes and saved images were preserved.",
                    env.spec.name
                )
            },
        )?;
        Ok(removed)
    }
    /// Remove records that describe processes inside a container that no longer exists.
    pub fn remove_container_records(&self, container_id: &str) -> Result<()> {
        for application in self.db.list::<Application>("application")? {
            if application.container_id == container_id {
                self.db.remove("application", &application.id)?;
            }
        }
        for recording in self.db.list::<Recording>("recording")? {
            if recording.container_id == container_id {
                self.db.remove("recording", &recording.id)?;
            }
        }
        for terminal in self.db.list::<Terminal>("terminal")? {
            if terminal.container_id == container_id {
                self.db.remove("terminal", &terminal.id)?;
            }
        }
        Ok(())
    }
    /// Discard the container's writable layer and recreate it from the pinned source image.
    pub async fn reset_container(&self, id: &str, job_id: &str) -> Result<Value> {
        let mut env = self.container(id).map_err(|e| anyhow::anyhow!(e.1))?;
        let manager = self
            .config
            .manager
            .as_deref()
            .context("MANAGER_CONTAINER is required")?;
        let info = self.inspect("container", manager).await?;
        if info["Config"]["Labels"]["thoric.manager"] != "true" {
            bail!("Manager label does not match");
        }
        crate::volumes::refuse_manager_volumes(&info, &env.mounts())?;
        // Check the image before touching the container, so a missing image leaves the desktop intact.
        self.source_image(&mut env, job_id)
            .await
            .context("Source image is missing; the container was left unchanged")?;
        let existing = match self.optional_inspect("container", &env.container).await? {
            Some(_) => Some(self.owned(&env).await?),
            None => None,
        };
        // The reset keeps the desktop's identity, also one it had before identities were kept.
        crate::identity::adopt(&mut env, existing.as_ref());
        if let Some(owned) = existing {
            self.docker(&[
                "rm",
                "--force",
                owned["Id"].as_str().context("Missing container ID")?,
            ])
            .await?;
            self.append_log(
                job_id,
                "Removed the container and its writable layer. Named volumes were preserved.",
            )?;
        } else {
            self.append_log(job_id, "Desktop container was already absent.")?;
        }
        // A reset starts over from the source image, so files carried across settings changes go too.
        self.remove_previous(&env, job_id).await?;
        if let Some(image) = env.working_image.take() {
            self.release_working_image(&image, job_id).await?;
        }
        self.remove_container_records(id)?;
        crate::compose::setup_again(&mut env);
        let result = self.provision(&env, manager).await;
        env.last_error = result.as_ref().err().map(|e| format!("{e:#}"));
        self.db.put(CONTAINERS, id, &env)?;
        result?;
        self.append_log(
            job_id,
            "Recreated the container from its saved source image.",
        )?;
        self.provisioned_ready(&env, job_id).await?;
        self.run_setup(id, job_id).await?;
        Ok(json!({"container_id": id, "volumes_preserved": true, "images_preserved": true}))
    }
}

/// An image `VOLUME` path needs an explicit named volume, or snapshots would silently exclude it.
pub(crate) fn image_volumes_mounted(image: &Value, mounts: &[Mount]) -> Result<()> {
    if let Some(volumes) = image["Config"]["Volumes"].as_object() {
        for target in volumes.keys() {
            if !mounts.iter().any(|m| m.target == *target) {
                bail!(
                    "Choose a named volume for the image's {target} VOLUME path; otherwise snapshots would exclude its data"
                );
            }
        }
    }
    Ok(())
}
#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
