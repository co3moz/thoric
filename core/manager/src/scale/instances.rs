//! A set's life: made with its first instance or from a container that exists, its instances
//! created and deleted on demand, and deleted with them.
//!
//! An instance is created by the creation's own path (`App::create_from`) with the set's
//! template, the compose the set pinned and the volumes [`super::volumes::plan`] chose, so it
//! starts as the first container did and runs the compose setup itself. Its number is taken under
//! the set's lock before the job starts and given back when the creation ends without a record.
use super::{
    Instance, NOTE_LIMIT, SETS, ScaleSet, Scaling, TEMPLATES, Template, lock,
    volumes::{self, Override},
};
use crate::{
    App,
    docker::{ComposeChoice, Creation},
    error::{Error, Result, missing},
    jobs::launch_named,
    model::{CONTAINERS, Container, ContainerSpec},
    outbound::PasswordChange,
};
use anyhow::{Context, anyhow};
use axum::{Json, http::StatusCode};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

/// The job kind of an instance's creation.
pub(crate) const SCALE_UP: &str = "scale-up";
/// The job kind of an instance's deletion through its set.
pub(crate) const SCALE_DOWN: &str = "scale-down";
/// The job kind of a set's deletion with its instances.
pub(crate) const SET_DELETE: &str = "scale-set-delete";

/// The set called `name`, or 404.
pub(crate) fn load(app: &App, name: &str) -> Result<ScaleSet> {
    app.db.get(SETS, name).map_err(|_| missing())
}

/// The container that is instance `number` of `set`, which an agent names by its number.
pub(crate) fn instance(app: &App, set: &str, number: u64) -> Result<Container> {
    let record = load(app, set)?;
    let not_one = || {
        Error(
            StatusCode::BAD_REQUEST,
            format!(
                "The scale set {set} has no instance {number}; list_instances shows its instances"
            ),
        )
    };
    let id = u32::try_from(number)
        .ok()
        .and_then(|number| record.instances.get(&number))
        .ok_or_else(not_one)?;
    app.container(&id.container_id).map_err(|_| {
        Error(
            StatusCode::CONFLICT,
            format!("Instance {number} of {set} is still being created; wait for its job"),
        )
    })
}

/// Make a scalable container: the set `name` of `spec` with `parameters` for its compose, saved
/// before its first instance is created. Returns the set's MCP endpoint, if one was made.
pub(crate) async fn make_set(
    app: &Arc<App>,
    name: &str,
    spec: &ContainerSpec,
    parameters: BTreeMap<String, String>,
    proxy_password: Option<String>,
    scaling: &Scaling,
    endpoint: Option<crate::desktop_mcp::Profile>,
) -> Result<Option<String>> {
    // A compose the image's record names is checked now, so a policy of a volume it does not
    // mount is refused before anything is made.
    let records: Vec<crate::model::ImageRecord> = app.db.list("image")?;
    let compose = crate::compose::compose_image(&records, &spec.image, &spec.image)
        .and_then(|record| record.compose.clone());
    let mut set = super::new_set(name, spec, scaling);
    let mut known = volumes::slots(&set, None);
    if let Some(compose) = &compose {
        let values = compose.resolve(&parameters, "x")?;
        let pinned = crate::compose::Pinned {
            compose: compose.clone(),
            values,
            readonly: Default::default(),
        };
        known = volumes::slots(&set, Some(&pinned));
    }
    volumes::check_targets(&known, &scaling.volumes)?;
    let template = Template {
        parameters,
        pinned: None,
        proxy_password,
    };
    save_new(app, &mut set, &template, endpoint).await
}

/// Make the container `id` scalable: the set `name` of its settings and the compose it was
/// created with, with the container as instance 1.
pub(crate) async fn make_set_of(
    app: &Arc<App>,
    id: &str,
    name: &str,
    scaling: &Scaling,
    endpoint: Option<crate::desktop_mcp::Profile>,
) -> Result<(ScaleSet, Option<String>)> {
    let container = app.container(id)?;
    let sets = super::all(app)?;
    if let Some((set, _)) = super::instance_of(&sets, id) {
        return Err(Error(
            StatusCode::CONFLICT,
            format!(
                "{} is an instance of the scale set {} already",
                container.spec.name, set.name
            ),
        ));
    }
    let pinned = match &container.setup {
        Some(setup) => Some((
            setup.image.clone(),
            app.db
                .get::<crate::compose::Pinned>(crate::compose::PINNED, id)
                .context("The compose this container was created with is missing")?,
        )),
        None => None,
    };
    let mut set = super::new_set(name, &container.spec, scaling);
    volumes::check_targets(
        &volumes::slots(&set, pinned.as_ref().map(|(_, pinned)| pinned)),
        &scaling.volumes,
    )?;
    set.instances.insert(
        1,
        Instance {
            container_id: id.into(),
            note: String::new(),
        },
    );
    let template = Template {
        parameters: pinned
            .as_ref()
            .map(|(_, pinned)| pinned.values.clone())
            .unwrap_or_default(),
        pinned,
        proxy_password: app.proxy_password(id),
    };
    let endpoint = save_new(app, &mut set, &template, endpoint).await?;
    // Made scalable counts as used, so a set that keeps no instance does not lose its first for
    // the time the container went unused before it was one.
    app.lifecycle.touch(id);
    // The container may run, with connections to a server instance 1 now runs for the set.
    crate::desktop_mcp::servers_changed(app, &BTreeMap::new(), &set);
    Ok((set, endpoint))
}

/// Save a new set with its template and, when a tool profile is named for it, its MCP endpoint,
/// refusing a name another set has.
async fn save_new(
    app: &Arc<App>,
    set: &mut ScaleSet,
    template: &Template,
    endpoint: Option<crate::desktop_mcp::Profile>,
) -> Result<Option<String>> {
    let lock = lock(app, &set.name);
    let _held = lock.lock().await;
    if app.db.contains(SETS, &set.name)? {
        return Err(Error(
            StatusCode::CONFLICT,
            format!("A scale set named {} exists", set.name),
        ));
    }
    app.db.put(TEMPLATES, &set.name.clone(), template)?;
    app.db.put(SETS, &set.name.clone(), &*set)?;
    if let Some(profile) = endpoint {
        // The set is made either way; an endpoint can still be made for it later.
        match crate::desktop_mcp::set_endpoint(app, &set.name, profile).await {
            Ok(name) => {
                set.endpoint = Some(name);
                app.db.put(SETS, &set.name.clone(), &*set)?;
            }
            Err(error) => tracing::warn!(
                "Could not create an MCP endpoint for the scale set {}: {error:#}",
                set.name
            ),
        }
    }
    Ok(set.endpoint.clone())
}

/// Start creating an instance of `set`, the lowest number it has free: answers its number,
/// container and job at once.
pub(crate) async fn grow(
    app: Arc<App>,
    set: &str,
    asked: BTreeMap<String, Override>,
    note: String,
) -> Result<Value> {
    let note = note.trim().to_owned();
    if note.chars().count() > NOTE_LIMIT || note.chars().any(char::is_control) {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            format!("An instance's note is one line of at most {NOTE_LIMIT} characters"),
        ));
    }
    let record = load(&app, set)?;
    let template: Template = app.db.get(TEMPLATES, set).unwrap_or_default();
    volumes::check_targets(
        &volumes::slots(&record, template.pinned.as_ref().map(|(_, pinned)| pinned)),
        &asked,
    )?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    let number = {
        let lock = lock(&app, set);
        let _held = lock.lock().await;
        let mut record = load(&app, set)?;
        if record.deleting {
            return Err(Error(
                StatusCode::CONFLICT,
                format!("The scale set {set} is being deleted with its instances"),
            ));
        }
        let Some(number) = record.free_number() else {
            return Err(Error(
                StatusCode::CONFLICT,
                format!(
                    "The scale set {set} has its most instances, {}; delete one before creating another",
                    record.max
                ),
            ));
        };
        record.instances.insert(
            number,
            Instance {
                container_id: id.clone(),
                note,
            },
        );
        app.db.put(SETS, set, &record)?;
        number
    };
    let name = record.instance_name(number);
    let (owner, container) = (set.to_owned(), id.clone());
    let started = launch_named(
        app.clone(),
        SCALE_UP,
        format!("container:{id}"),
        Some(name.clone()),
        move |app, job_id| async move {
            let created = create_instance(&app, &owner, number, &container, &asked, &job_id).await;
            instance_finished(&app, &owner, &container, created, &job_id).await
        },
    );
    match started {
        Ok(Json(mut answer)) => {
            answer["set"] = json!(set);
            answer["instance"] = json!(number);
            answer["container_id"] = json!(id);
            answer["name"] = json!(name);
            Ok(answer)
        }
        Err(error) => {
            super::forget_container(&app, &id).await?;
            Err(error)
        }
    }
}

/// The work of an instance's creation job.
async fn create_instance(
    app: &Arc<App>,
    set: &str,
    number: u32,
    id: &str,
    asked: &BTreeMap<String, Override>,
    job_id: &str,
) -> anyhow::Result<Value> {
    let record = load(app, set).map_err(|_| anyhow!("The scale set {set} was deleted"))?;
    let pinned = pinned_compose(app, set, id).await?;
    let mut spec = record.template.clone();
    spec.name = record.instance_name(number);
    // A volume the template shares that the first instance made of its own and took with it
    // when it was deleted becomes the instance's own, instead of a missing one.
    let compose = pinned.as_ref().map(|(_, pinned)| pinned);
    let first = volumes::plan(&record, compose, number, id, asked, &[]);
    let mut gone = Vec::new();
    for slot in volumes::slots(&record, compose) {
        if first.shared.contains(&slot.target)
            && slot.volume.starts_with("thoric-")
            && app
                .optional_inspect("volume", &slot.volume)
                .await?
                .is_none()
        {
            app.append_log(
                job_id,
                &format!(
                    "The volume {} at {} is gone, so this instance gets one of its own.",
                    slot.volume, slot.target
                ),
            )?;
            gone.push(slot.volume);
        }
    }
    let planned = volumes::plan(&record, compose, number, id, asked, &gone);
    spec.mounts = planned.mounts;
    let others: Vec<Container> = app.db.list(CONTAINERS)?;
    let dropped = crate::duplicate::free_aliases(id, &mut spec, &others);
    if !dropped.is_empty() {
        app.append_log(
            job_id,
            &format!(
                "Left out the aliases another member of its groups answers to: {}.",
                dropped.join(", ")
            ),
        )?;
    }
    let template: Template = app.db.get(TEMPLATES, set).unwrap_or_default();
    if let Some(password) = &template.proxy_password {
        app.save_proxy_password(
            id,
            spec.proxy.as_ref(),
            &PasswordChange::Set(password.clone()),
        )?;
    }
    for (made_for, volume) in &planned.own {
        app.append_log(
            job_id,
            &format!("Instance {number} gets the volume {volume} of its own for {made_for}."),
        )?;
    }
    for target in &planned.read_only {
        app.append_log(
            job_id,
            &format!(
                "Instance {number} mounts the volume at {target} read-only; instance 1 writes there."
            ),
        )?;
    }
    let compose = match (pinned, planned.values) {
        (Some((image, mut pinned)), Some(values)) => {
            pinned.values = values;
            pinned.readonly = planned.readonly;
            ComposeChoice::Pinned(image, pinned)
        }
        _ => ComposeChoice::Image(template.parameters),
    };
    let mut result = app
        .create_from(
            id,
            spec,
            Creation {
                compose,
                own: planned.own,
            },
            job_id,
        )
        .await?;
    result["set"] = json!(set);
    result["instance"] = json!(number);
    Ok(result)
}

/// The compose the set's instances are created with: the one an earlier instance pinned, or the
/// one of the image's record, pinned now with the set's values for the container `id` and kept
/// for the instances after it. `None` for an image without one, or one not downloaded yet, whose
/// creation then finds the compose itself.
async fn pinned_compose(
    app: &App,
    set: &str,
    id: &str,
) -> anyhow::Result<Option<(String, crate::compose::Pinned)>> {
    let template: Template = app.db.get(TEMPLATES, set).unwrap_or_default();
    if template.pinned.is_some() {
        return Ok(template.pinned);
    }
    let record = load(app, set).map_err(|error| anyhow!(error.1))?;
    let Some(image) = app
        .optional_inspect("image", &record.template.image)
        .await?
    else {
        return Ok(None);
    };
    let image_id = image["Id"].as_str().context("Missing image ID")?;
    let lock = lock(app, set);
    let _held = lock.lock().await;
    let mut template: Template = app.db.get(TEMPLATES, set).unwrap_or_default();
    if template.pinned.is_none() {
        template.pinned =
            app.pin_compose(id, &record.template.image, image_id, &template.parameters)?;
        if template.pinned.is_some() {
            app.db.put(TEMPLATES, set, &template)?;
        }
    }
    Ok(template.pinned)
}

/// What an instance's creation job ends with: the instance's number given back when no record
/// was saved, and the instances the set keeps created once one stands.
async fn instance_finished(
    app: &Arc<App>,
    set: &str,
    id: &str,
    created: anyhow::Result<Value>,
    job_id: &str,
) -> anyhow::Result<Value> {
    if app.container(id).is_err() {
        app.forget_proxy_password(id)?;
        super::forget_container(app, id).await?;
        return created;
    }
    crate::desktop_mcp::announce_set(app, set);
    let result = created?;
    // The instances a set keeps come one after another, each once the one before stands, so
    // they share the volumes the first one made.
    if let Ok(record) = load(app, set)
        && (record.instances.len() as u32) < record.min
    {
        match next(app.clone(), set.to_owned()).await {
            Ok(next) => app.append_log(
                job_id,
                &format!(
                    "The set keeps {} instances, so instance {} is being created.",
                    record.min, next["instance"]
                ),
            )?,
            Err(error) => app.append_log(
                job_id,
                &format!(
                    "Could not create the next instance the set keeps: {}",
                    error.1
                ),
            )?,
        }
    }
    Ok(result)
}

/// The next instance of `set` a creation job starts once its own stands. Boxed as a future that
/// is `Send`, since that job is itself started by [`grow`].
fn next(
    app: Arc<App>,
    set: String,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>> {
    Box::pin(async move { grow(app, &set, BTreeMap::new(), String::new()).await })
}

/// Why deleting instance `number` of `set` is refused, if it is: the set keeps its minimum, an
/// agent leaves the instances it keeps alone, and a set being deleted deletes them itself.
pub(crate) fn refusal(set: &ScaleSet, number: u32, agent: bool) -> Option<String> {
    if set.deleting {
        return Some(format!(
            "The scale set {} is being deleted with its instances",
            set.name
        ));
    }
    if !set.instances.contains_key(&number) {
        return Some(format!(
            "The scale set {} has no instance {number}",
            set.name
        ));
    }
    if set.instances.len() as u32 <= set.min {
        return Some(format!(
            "The scale set {} keeps at least {} instances; lower its minimum first",
            set.name, set.min
        ));
    }
    if agent && set.kept().contains(&number) {
        return Some(format!(
            "Instance {number} is one of the {} instances the scale set {} keeps, which only the dashboard deletes",
            set.min, set.name
        ));
    }
    None
}

/// Start deleting instance `number` of `set` with the volumes made for it; the volumes it shares
/// stay. `agent` says an agent asks, which leaves the instances the set keeps alone.
pub(crate) fn shrink(app: Arc<App>, set: &str, number: u32, agent: bool) -> Result<Value> {
    let record = load(&app, set)?;
    if let Some(refusal) = refusal(&record, number, agent) {
        return Err(Error(StatusCode::CONFLICT, refusal));
    }
    let id = record.instances[&number].container_id.clone();
    let container = app.container(&id).map_err(|_| {
        Error(
            StatusCode::CONFLICT,
            format!("Instance {number} of {set} is still being created; wait for its job"),
        )
    })?;
    let (owner, name) = (set.to_owned(), container.spec.name.clone());
    let Json(mut answer) = crate::jobs::launch(
        app,
        SCALE_DOWN,
        format!("container:{id}"),
        move |app, job_id| async move {
            // Checked again under the set's lock, where the instance leaves the record: two
            // deletions never read the same count.
            let instance = take_out(&app, &owner, number, agent).await?;
            let env = app.container(&id).map_err(|error| anyhow!(error.1))?;
            match app.remove_container(&env, &job_id, true).await {
                Ok(removed) => {
                    let mut result = crate::api::removal(&id, &name, true, removed);
                    result["set"] = json!(owner);
                    result["instance"] = json!(number);
                    Ok(result)
                }
                Err(error) => {
                    put_back(&app, &owner, number, instance).await?;
                    Err(error)
                }
            }
        },
    )?;
    answer["set"] = json!(set);
    answer["instance"] = json!(number);
    Ok(answer)
}

/// Take instance `number` out of the record of `set`, under the set's lock, once nothing refuses
/// its deletion: from then on the set no longer counts it, so a second deletion that reads the
/// count meanwhile sees it gone and the set never goes below its minimum. Answers the instance,
/// to put back should the deletion fail.
pub(crate) async fn take_out(
    app: &App,
    set: &str,
    number: u32,
    agent: bool,
) -> anyhow::Result<Instance> {
    let lock = lock(app, set);
    let _held = lock.lock().await;
    let mut record = load(app, set).map_err(|error| anyhow!(error.1))?;
    if let Some(refusal) = refusal(&record, number, agent) {
        anyhow::bail!(refusal);
    }
    let instance = record
        .instances
        .remove(&number)
        .context("refusal checks the number")?;
    app.db.put(SETS, set, &record)?;
    crate::desktop_mcp::announce_set(app, set);
    Ok(instance)
}

/// Put an instance whose deletion failed back into its set, which counts it again.
pub(crate) async fn put_back(
    app: &App,
    set: &str,
    number: u32,
    instance: Instance,
) -> anyhow::Result<()> {
    let lock = lock(app, set);
    let _held = lock.lock().await;
    let Ok(mut record) = load(app, set) else {
        return Ok(());
    };
    record.instances.entry(number).or_insert(instance);
    app.db.put(SETS, set, &record)?;
    crate::desktop_mcp::announce_set(app, set);
    Ok(())
}

/// Start deleting `set`: every instance with the volumes made for it, the set's MCP endpoint and
/// its template. The volumes the instances share stay. The record is marked and stays until the
/// last instance is gone, so the name stays taken, nothing creates or deletes an instance
/// meanwhile, and a deletion that fails halfway leaves a set that is deleted again.
pub(crate) async fn delete_set(app: Arc<App>, set: &str) -> Result<Value> {
    let members = {
        let lock = lock(&app, set);
        let _held = lock.lock().await;
        let mut record = load(&app, set)?;
        if record.deleting
            && app.db.active_jobs()?.iter().any(|job| {
                job.kind == SET_DELETE && job.resource.as_deref() == Some(&resource(set))
            })
        {
            return Err(Error(
                StatusCode::CONFLICT,
                format!("The scale set {set} is being deleted already"),
            ));
        }
        record.deleting = true;
        app.db.put(SETS, set, &record)?;
        record.instances
    };
    crate::desktop_mcp::announce_set(&app, set);
    let owner = set.to_owned();
    let started = crate::jobs::launch(
        app.clone(),
        SET_DELETE,
        resource(set),
        move |app, job_id| async move {
            let mut deleted = Vec::new();
            for (number, instance) in &members {
                let lock = app.resource_lock(&format!("container:{}", instance.container_id));
                let _held = lock.lock_owned().await;
                let Ok(env) = app.container(&instance.container_id) else {
                    app.append_log(
                        &job_id,
                        &format!(
                            "Instance {number} has no record; one still being created stays as a container of its own."
                        ),
                    )?;
                    continue;
                };
                app.append_log(
                    &job_id,
                    &format!("Deleting instance {number}, \"{}\".", env.spec.name),
                )?;
                app.remove_container(&env, &job_id, true).await?;
                deleted.push(*number);
            }
            {
                let lock = lock(&app, &owner);
                let _held = lock.lock().await;
                app.db.remove(SETS, &owner)?;
                app.db.remove(TEMPLATES, &owner)?;
            }
            crate::desktop_mcp::forget_set(&app, &owner)?;
            app.append_log(
                &job_id,
                &format!("Deleted the scale set {owner}. The volumes its instances shared stay."),
            )?;
            Ok(json!({"deleted": owner, "instances": deleted}))
        },
    );
    match started {
        Ok(Json(answer)) => Ok(answer),
        Err(error) => {
            // Nothing was deleted: the set comes back as it was.
            let lock = lock(&app, set);
            let _held = lock.lock().await;
            if let Ok(mut record) = load(&app, set) {
                record.deleting = false;
                app.db.put(SETS, set, &record)?;
            }
            Err(error)
        }
    }
}

/// The resource a set's deletion job holds.
fn resource(set: &str) -> String {
    format!("scale-set-delete:{set}")
}
