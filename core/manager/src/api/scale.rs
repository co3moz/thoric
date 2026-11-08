//! A scale set's routes: the list, making an existing container scalable, a set's limits and
//! volume policies, its deletion, and its instances' creation and deletion. A container is made
//! scalable at its creation through `POST /api/containers` with `scaling`.
use super::default_true;
use crate::{
    App,
    error::{Error, Result},
    model::{CONTAINERS, Container},
    scale::{
        self, SETS, ScaleSet, Scaling, ServerPolicy, TEMPLATES, Template, VolumePolicy,
        instances::{self, load},
        volumes::{self, Override},
    },
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

/// `GET /api/scale-sets`: every set with its instances.
pub async fn sets(State(app): State<Arc<App>>) -> Result<Json<Value>> {
    let containers: Vec<Container> = app.db.list(CONTAINERS)?;
    let items = scale::all(&app)?
        .iter()
        .map(|set| described(&app, set, &containers))
        .collect::<Vec<_>>();
    Ok(Json(json!({"items": items})))
}

/// `GET /api/scale-sets/{name}`.
pub async fn set(State(app): State<Arc<App>>, Path(name): Path<String>) -> Result<Json<Value>> {
    let set = load(&app, &name)?;
    let containers: Vec<Container> = app.db.list(CONTAINERS)?;
    Ok(Json(described(&app, &set, &containers)))
}

/// A set as the routes answer it, with its volumes: where each is mounted, the template's volume
/// there and what an instance gets for it.
fn described(app: &App, set: &ScaleSet, containers: &[Container]) -> Value {
    let template: Template = app.db.get(TEMPLATES, &set.name).unwrap_or_default();
    let pinned = template.pinned.as_ref().map(|(_, pinned)| pinned);
    let places = pinned
        .map(|pinned| pinned.setup("").places)
        .unwrap_or_default();
    let mut answer = scale::summary(set, containers);
    answer["slots"] = json!(
        volumes::slots(set, pinned)
            .iter()
            .map(|slot| json!({
                "target": slot.target,
                "volume": slot.volume,
                "parameter": slot.key,
                "readonly": slot.readonly,
                "policy": set.policy(&slot.target),
                "label": places.iter().find(|place| place.path == slot.target).map(|place| &place.label),
            }))
            .collect::<Vec<_>>()
    );
    answer
}

/// How an existing container is made scalable.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MakeScalable {
    /// The container that becomes instance 1, keeping its name.
    pub container_id: String,
    #[serde(default)]
    pub scaling: Scaling,
    /// Give the set an MCP endpoint, `/mcp/<set>`, whose tools take `instance`.
    #[serde(default = "default_true")]
    pub mcp_endpoint: bool,
}

/// `POST /api/scale-sets`: make an existing container scalable, as instance 1 of a new set whose
/// template is its settings and the compose values it was created with.
pub async fn make_scalable(
    State(app): State<Arc<App>>,
    Json(body): Json<MakeScalable>,
) -> Result<Json<Value>> {
    body.scaling.validate()?;
    let container = app.container(&body.container_id)?;
    let name = body
        .scaling
        .set
        .clone()
        .unwrap_or_else(|| scale::name_of(&container.spec.name));
    scale::set_name(&name)?;
    let (set, _) = instances::make_set_of(
        &app,
        &body.container_id,
        &name,
        &body.scaling,
        body.mcp_endpoint,
    )
    .await?;
    let containers: Vec<Container> = app.db.list(CONTAINERS)?;
    Ok(Json(described(&app, &set, &containers)))
}

/// A change of a set's limits, volume policies and server policies; what it leaves out stays.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateSet {
    #[serde(default)]
    pub min: Option<u32>,
    #[serde(default)]
    pub max: Option<u32>,
    #[serde(default)]
    pub idle_delete_minutes: Option<u32>,
    /// What each volume becomes in the instances created from now on, by its folder.
    #[serde(default)]
    pub volumes: Option<BTreeMap<String, VolumePolicy>>,
    /// Where each desktop MCP server runs, by its name, at once: `each` in every instance, or
    /// `first` in instance 1 alone for all of them. It replaces the set's whole map.
    #[serde(default)]
    pub servers: Option<BTreeMap<String, ServerPolicy>>,
}

/// `PUT /api/scale-sets/{name}`. The template stays as it is; a change applies to the instances
/// created from now on and to which ones the set keeps, and a change of where a desktop server
/// runs to every instance at once.
pub async fn update_set(
    State(app): State<Arc<App>>,
    Path(name): Path<String>,
    Json(body): Json<UpdateSet>,
) -> Result<Json<Value>> {
    let lock = scale::lock(&app, &name);
    let _held = lock.lock().await;
    let mut set = load(&app, &name)?;
    if set.deleting {
        return Err(Error(
            StatusCode::CONFLICT,
            format!("The scale set {name} is being deleted with its instances"),
        ));
    }
    let min = body.min.unwrap_or(set.min);
    let max = body.max.unwrap_or(set.max);
    let idle = body.idle_delete_minutes.unwrap_or(set.idle_delete_minutes);
    scale::limits(min, max, idle)?;
    if let Some(highest) = set.instances.keys().next_back()
        && *highest > max
    {
        return Err(Error(
            StatusCode::CONFLICT,
            format!("Instance {highest} exists; delete the instances above {max} first"),
        ));
    }
    if let Some(policies) = &body.volumes {
        let template: Template = app.db.get(TEMPLATES, &name).unwrap_or_default();
        volumes::check_targets(
            &volumes::slots(&set, template.pinned.as_ref().map(|(_, pinned)| pinned)),
            policies,
        )?;
        set.volumes = policies
            .iter()
            .filter(|(_, policy)| **policy != VolumePolicy::Shared)
            .map(|(target, policy)| (target.clone(), *policy))
            .collect();
    }
    let before = set.servers.clone();
    if let Some(servers) = &body.servers {
        scale::check_servers(servers)?;
        set.servers = scale::server_policies(servers);
    }
    (set.min, set.max, set.idle_delete_minutes) = (min, max, idle);
    app.db.put(SETS, &name, &set)?;
    crate::desktop_mcp::servers_changed(&app, &before, &set);
    let containers: Vec<Container> = app.db.list(CONTAINERS)?;
    Ok(Json(described(&app, &set, &containers)))
}

/// `DELETE /api/scale-sets/{name}`: delete the set with its instances and the volumes made for
/// them, in a job.
pub async fn delete_set(
    State(app): State<Arc<App>>,
    Path(name): Path<String>,
) -> Result<Json<Value>> {
    Ok(Json(instances::delete_set(app, &name).await?))
}

/// A new instance: what it gets for the template's volumes, over the set's policy, and what it is
/// for.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewInstance {
    /// By the folder a volume of the template is mounted at: `shared` for the template's volume,
    /// `new` for an empty one of the instance's own.
    #[serde(default)]
    pub volumes: BTreeMap<String, Override>,
    /// What the instance is for, such as the agent that works in it; one line of at most 200
    /// characters.
    #[serde(default)]
    pub note: String,
}

/// `POST /api/scale-sets/{name}/instances`: create an instance, the lowest number the set has
/// free, in a job.
pub async fn create_instance(
    State(app): State<Arc<App>>,
    Path(name): Path<String>,
    Json(body): Json<NewInstance>,
) -> Result<Json<Value>> {
    Ok(Json(
        instances::grow(app, &name, body.volumes, body.note).await?,
    ))
}

/// `DELETE /api/scale-sets/{name}/instances/{number}`: delete an instance with the volumes made
/// for it, in a job. The set keeps its minimum.
pub async fn delete_instance(
    State(app): State<Arc<App>>,
    Path((name, number)): Path<(String, u32)>,
) -> Result<Json<Value>> {
    Ok(Json(instances::shrink(app, &name, number, false)?))
}

/// What `list_instances` answers: the set, the volumes its instances mount and whether each
/// shares it, and each instance's state.
pub(crate) async fn list_instances(app: Arc<App>, name: &str) -> Result<Value> {
    let set = load(&app, name)?;
    let containers: Vec<Container> = app.db.list(CONTAINERS)?;
    let Json(listed) = super::containers(State(app.clone())).await?;
    let mut answer = described(&app, &set, &containers);
    let members = set
        .instances
        .values()
        .filter_map(|instance| containers.iter().find(|c| c.id == instance.container_id))
        .collect::<Vec<_>>();
    let mut items = Vec::new();
    for (number, instance) in &set.instances {
        let record = members
            .iter()
            .find(|container| container.id == instance.container_id);
        let entry = listed.as_array().and_then(|items| {
            items
                .iter()
                .find(|item| item["id"] == instance.container_id)
        });
        let mounts = record
            .map(|record| {
                record
                    .mounts()
                    .iter()
                    .map(|mount| {
                        let shared = members.iter().any(|other| {
                            other.id != record.id
                                && other.mounts().iter().any(|m| m.volume == mount.volume)
                        });
                        json!({
                            "target": mount.target,
                            "volume": mount.volume,
                            "readonly": mount.readonly,
                            "sharing": if shared { "shared" } else { "own" },
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        items.push(json!({
            "instance": number,
            "name": record.map(|record| &record.spec.name),
            "container_id": instance.container_id,
            "note": instance.note,
            "creating": record.is_none(),
            "kept": set.kept().contains(number),
            "state": entry.map(|entry| &entry["state"]),
            "health": entry.map(|entry| &entry["health"]),
            "in_use": entry.map(|entry| &entry["in_use"]),
            "mounts": mounts,
        }));
    }
    answer["instances"] = json!(items);
    answer["free"] = json!(set.max as usize - set.instances.len().min(set.max as usize));
    Ok(answer)
}
