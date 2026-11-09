//! MCP endpoints: which containers an endpoint covers, with which scopes and desktop servers, and
//! the token that opens it.
use super::{
    profiles::Profile,
    registry::{server_name, slug},
    scopes::{Scope, catalog, describe},
};
use crate::{
    App,
    error::{Error, Result, missing},
    model::{Container, now},
};
use anyhow::bail;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use blake2::{Blake2s256, Digest};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use serde_json::{Value, json};
use std::sync::Arc;
use subtle::ConstantTimeEq;

/// The database kind of endpoints, by name.
pub(crate) const ENDPOINTS: &str = "mcp-endpoint";
const GRANT_LIMIT: usize = 16;
/// A grant leaves out at most this many tools by name.
const EXCLUSION_LIMIT: usize = 1024;

/// Which desktop servers of a container an endpoint offers.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Servers {
    /// Every one, also those declared later.
    All,
    /// These, by name.
    Named(Vec<String>),
}
impl Servers {
    pub fn includes(&self, name: &str) -> bool {
        match self {
            Servers::All => true,
            Servers::Named(names) => names.iter().any(|n| n == name),
        }
    }
}
impl Serialize for Servers {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Servers::All => serializer.serialize_str("all"),
            Servers::Named(names) => names.serialize(serializer),
        }
    }
}
impl<'de> Deserialize<'de> for Servers {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::String(all) if all == "all" => Ok(Servers::All),
            names @ Value::Array(_) => serde_json::from_value(names)
                .map(Servers::Named)
                .map_err(D::Error::custom),
            _ => Err(D::Error::custom(
                "servers is \"all\" or a list of server names",
            )),
        }
    }
}
/// The two forms [`Servers`]'s serde reads and writes, for the HTTP API's document.
impl schemars::JsonSchema for Servers {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Servers".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "`all` for every desktop MCP server of the container, or the names of some.",
            "oneOf": [{"const": "all"}, {"type": "array", "items": {"type": "string"}}],
        })
    }
}

/// One container of an endpoint, or the scale set whose instances it covers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Grant {
    /// Empty for a grant of a scale set.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub container_id: String,
    /// The scale set the grant covers instead of a container: every instance it has, now and
    /// later, which the tools take by `instance`. Such a grant is its endpoint's only one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// The container's name on the endpoint, which its tools take when it covers several.
    pub alias: String,
    pub scopes: Vec<Scope>,
    /// Which of the scopes' tools the grant offers. A record from before profiles has none and
    /// reads as `full`, every tool of its scopes, which is also not written.
    #[serde(default, skip_serializing_if = "Profile::is_full")]
    pub profile: Profile,
    pub servers: Servers,
    /// Tools left out although their scope or server is granted, by the name the endpoint gives
    /// them for this container alone: a container tool's own name, such as
    /// `application_diagnostics`, or `SERVER__TOOL` for a desktop server's. Empty means
    /// everything the scopes and servers give, also tools a server adds later.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded_tools: Vec<String>,
}
impl Grant {
    /// Whether the grant covers the container `id`: it names it, or the set it is an instance of.
    pub fn covers(&self, sets: &[crate::scale::ScaleSet], id: &str) -> bool {
        match &self.set {
            Some(set) => sets
                .iter()
                .any(|known| known.name == *set && known.number_of(id).is_some()),
            None => self.container_id == id,
        }
    }
    /// Whether the grant leaves out the tool of this name.
    pub fn excludes(&self, name: &str) -> bool {
        self.excluded_tools.iter().any(|excluded| excluded == name)
    }
    /// Whether the grant offers the container tool `name` of a scope it has: its profile keeps
    /// it and its deny list does not name it.
    pub fn offers(&self, name: &str) -> bool {
        self.profile.includes(name) && !self.excludes(name)
    }
    /// Whether the grant has the `scale` scope, which only a grant of a scale set may have.
    pub fn scales(&self) -> bool {
        self.scopes.contains(&Scope::Scale)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Endpoint {
    /// Its path: `/mcp/<name>`.
    pub name: String,
    /// Created with its container, and deleted with it.
    #[serde(default)]
    pub automatic: bool,
    pub grants: Vec<Grant>,
    /// The hash of its token. Without one only the administrator's MCP token opens it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_created_at: Option<u64>,
    pub created_at: u64,
}
impl Endpoint {
    /// The scale set the endpoint covers, whose tools take `instance`.
    pub fn set(&self) -> Option<(&Grant, &str)> {
        match self.grants.as_slice() {
            [grant] => grant.set.as_deref().map(|set| (grant, set)),
            _ => None,
        }
    }
    /// Whether `token` is this endpoint's, compared in constant time.
    pub fn opens(&self, token: &str) -> bool {
        self.token_hash
            .as_ref()
            .is_some_and(|hash| bool::from(hash.as_bytes().ct_eq(hashed(token).as_bytes())))
    }
}

/// A token's hash. Tokens are random, so a fast hash is enough to keep them out of the database.
fn hashed(token: &str) -> String {
    Blake2s256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
/// A new token: 244 random bits, as a session's.
fn new_token() -> String {
    format!(
        "bsk_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// An endpoint's name: lowercase letters, digits and dashes, at most 48.
pub(crate) fn endpoint_name(name: &str) -> anyhow::Result<()> {
    if !slug(name, 48) {
        bail!(
            "An endpoint name is lowercase letters, digits and dashes, at most 48, not starting with a dash"
        );
    }
    Ok(())
}
/// A name made of a container's name, as `slug` takes it, or `fallback` when nothing is left.
pub(crate) fn slug_of(name: &str, limit: usize, fallback: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut out: String = out.chars().take(limit).collect();
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() { fallback.into() } else { out }
}
/// `base`, or `base-2`, `base-3`... when `taken` says it is in use, within `limit` characters.
fn unique(base: &str, limit: usize, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.into();
    }
    (2..)
        .map(|n| {
            let suffix = format!("-{n}");
            let head: String = base.chars().take(limit - suffix.len()).collect();
            format!("{}{suffix}", head.trim_end_matches('-'))
        })
        .find(|candidate| !taken(candidate))
        .unwrap()
}

/// A grant as a request names it; the alias defaults to the container's name.
#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GrantRequest {
    /// The container, or empty with `set`.
    #[serde(default)]
    pub container_id: String,
    /// A scale set instead of a container: the endpoint's tools take `instance`, and it covers
    /// no other container.
    #[serde(default)]
    pub set: Option<String>,
    #[serde(default)]
    pub alias: Option<String>,
    pub scopes: Vec<Scope>,
    /// Which of the scopes' tools the grant offers: `full`, the default, `headless` or `desktop`.
    #[serde(default)]
    pub profile: Profile,
    pub servers: Servers,
    #[serde(default)]
    pub excluded_tools: Vec<String>,
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateEndpoint {
    pub name: String,
    pub grants: Vec<GrantRequest>,
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateEndpoint {
    pub grants: Vec<GrantRequest>,
}

/// Check grants and give each an alias: one per container, of existing containers.
pub(crate) fn grants(app: &App, requested: Vec<GrantRequest>) -> anyhow::Result<Vec<Grant>> {
    if requested.is_empty() || requested.len() > GRANT_LIMIT {
        bail!("An endpoint covers 1 to {GRANT_LIMIT} containers");
    }
    if requested.iter().any(|request| request.set.is_some()) {
        let [request] = <[GrantRequest; 1]>::try_from(requested)
            .map_err(|_| anyhow::anyhow!("An endpoint of a scale set covers nothing else"))?;
        return Ok(vec![set_grant(app, request)?]);
    }
    let mut out: Vec<Grant> = Vec::new();
    for request in requested {
        if request.scopes.contains(&Scope::Scale) {
            bail!("The scale scope is for an endpoint of a scale set");
        }
        let container = app
            .container(&request.container_id)
            .map_err(|_| anyhow::anyhow!("Container not found: {}", request.container_id))?;
        if out.iter().any(|g| g.container_id == request.container_id) {
            bail!("An endpoint names each container once");
        }
        let alias = match request.alias {
            Some(alias) => {
                if !slug(&alias, 24) {
                    bail!(
                        "A container's alias is lowercase letters, digits and dashes, at most 24: {alias:?}"
                    );
                }
                alias
            }
            None => unique(&slug_of(&container.spec.name, 24, "desktop"), 24, |a| {
                out.iter().any(|g| g.alias == a)
            }),
        };
        if out.iter().any(|g| g.alias == alias) {
            bail!("Each container of an endpoint needs its own alias: {alias}");
        }
        let mut scopes = Vec::new();
        for scope in request.scopes {
            if !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
        servers_named(&request.servers)?;
        let excluded_tools = excluded(&request.excluded_tools, &scopes, false)?;
        out.push(Grant {
            container_id: request.container_id,
            set: None,
            alias,
            scopes,
            profile: request.profile,
            servers: request.servers,
            excluded_tools,
        });
    }
    Ok(out)
}

/// A grant of the scale set a request names, which must exist.
fn set_grant(app: &App, request: GrantRequest) -> anyhow::Result<Grant> {
    let set = request.set.unwrap_or_default();
    if !request.container_id.is_empty() {
        bail!("A grant names a container or a scale set, not both");
    }
    if !app.db.contains(crate::scale::SETS, &set)? {
        bail!("Scale set not found: {set}");
    }
    let mut scopes = Vec::new();
    for scope in request.scopes {
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    servers_named(&request.servers)?;
    let excluded_tools = excluded(&request.excluded_tools, &scopes, true)?;
    Ok(Grant {
        container_id: String::new(),
        alias: slug_of(&set, 24, "set"),
        set: Some(set),
        scopes,
        profile: request.profile,
        servers: request.servers,
        excluded_tools,
    })
}

/// Check the servers a grant names.
fn servers_named(servers: &Servers) -> anyhow::Result<()> {
    if let Servers::Named(names) = servers {
        if names.len() > 64 {
            bail!("An endpoint names at most 64 servers of a container");
        }
        for name in names {
            server_name(name)?;
        }
    }
    Ok(())
}

/// A grant's deny list, checked: names in the characters exposed tool names have, each once. A
/// name without `__` is a container tool's and must belong to the granted scopes, or be `get_job`
/// with a scope that returns jobs, so a typo is refused instead of leaving the tool in. A name
/// with `__` is a desktop server's tool, `SERVER__TOOL`; the server's part must be a server name,
/// and the tool is kept whether or not the server is known now: it may be stopped, or declared
/// later.
fn excluded(names: &[String], scopes: &[Scope], set: bool) -> anyhow::Result<Vec<String>> {
    if names.len() > EXCLUSION_LIMIT {
        bail!("A grant leaves out at most {EXCLUSION_LIMIT} tools");
    }
    let mut out: Vec<String> = Vec::new();
    for name in names {
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            bail!(
                "An excluded tool is named as the endpoint exposes it, in letters, digits, _ and -, at most 64: {name:?}"
            );
        }
        match name.split_once("__") {
            // Server names have no underscore, so the first `__` ends the server's part.
            Some((server, _)) => {
                if server_name(server).is_err() {
                    bail!("{name} does not name a desktop server's tool as SERVER__TOOL");
                }
            }
            None => {
                if !scopes.iter().any(|scope| scope.offers(name))
                    && !(name == "get_job" && scopes.iter().any(|scope| scope.has_jobs()))
                    && !(set && name == super::scopes::LIST_INSTANCES)
                {
                    bail!("{name} is not a tool of the granted scopes, so it cannot be left out");
                }
            }
        }
        if !out.contains(name) {
            out.push(name.clone());
        }
    }
    Ok(out)
}

/// An endpoint as the dashboard and the administrator's tools show it: never its token.
pub(crate) fn summary(app: &App, endpoint: &Endpoint) -> Value {
    json!({
        "name": endpoint.name,
        "path": format!("/mcp/{}", endpoint.name),
        "automatic": endpoint.automatic,
        "has_token": endpoint.token_hash.is_some(),
        "token_created_at": endpoint.token_created_at,
        "created_at": endpoint.created_at,
        "grants": endpoint.grants.iter().map(|grant| json!({
            "container_id": grant.container_id,
            "set": grant.set,
            "container_name": app.container(&grant.container_id).map(|c| c.spec.name).unwrap_or_default(),
            "alias": grant.alias,
            "scopes": grant.scopes,
            "profile": grant.profile,
            "servers": grant.servers,
            "excluded_tools": grant.excluded_tools,
        })).collect::<Vec<_>>(),
        "tool_count": catalog(app, endpoint).len(),
    })
}

fn load(app: &App, name: &str) -> Result<Endpoint> {
    app.db.get(ENDPOINTS, name).map_err(|_| missing())
}

/// Tell the agents of the endpoints covering `container_id` that their tools changed, and the
/// desktops what their servers are now.
pub(crate) fn announce(app: &App, container_id: &str) {
    app.desktop_mcp.report_changes();
    let sets = crate::scale::all(app).unwrap_or_default();
    for endpoint in app.db.list::<Endpoint>(ENDPOINTS).unwrap_or_default() {
        if endpoint
            .grants
            .iter()
            .any(|grant| grant.covers(&sets, container_id))
        {
            let _ = app.desktop_mcp.changes.send(endpoint.name);
        }
    }
}

/// Tell the agents of the endpoints of the scale set `set` that its instances changed, which
/// changes their tools when the first instance brings its desktop servers or the last takes them.
pub(crate) fn announce_set(app: &App, set: &str) {
    app.desktop_mcp.report_changes();
    for endpoint in app.db.list::<Endpoint>(ENDPOINTS).unwrap_or_default() {
        if endpoint.set().is_some_and(|(_, name)| name == set) {
            let _ = app.desktop_mcp.changes.send(endpoint.name);
        }
    }
}

/// End what an endpoint holds open after it changed or went away.
/// Forget what a deleted endpoint reached, so an endpoint made later with its name starts with
/// nothing: the jobs it started, the requests for a person it asked, which are declined, and the
/// terminals it opened, which only the administrator reaches from now on.
fn forget(app: &App, name: &str) -> anyhow::Result<()> {
    app.desktop_mcp.close_service(name);
    app.desktop_mcp.jobs.lock().unwrap().remove(name);
    app.human.end_endpoint(name, &app.events);
    crate::terminal::forget_endpoint(app, name)
}

fn release(app: &App, name: &str) {
    app.desktop_mcp
        .pool
        .retain(|(endpoint, _, _)| endpoint != name);
    let _ = app.desktop_mcp.changes.send(name.to_owned());
    app.desktop_mcp.report_changes();
}

pub async fn endpoints(State(app): State<Arc<App>>) -> Result<Json<Value>> {
    let mut items: Vec<Endpoint> = app.db.list(ENDPOINTS)?;
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(
        json!({"items": items.iter().map(|e| summary(&app, e)).collect::<Vec<_>>()}),
    ))
}

pub async fn create_endpoint(
    State(app): State<Arc<App>>,
    Json(body): Json<CreateEndpoint>,
) -> Result<Json<Value>> {
    endpoint_name(&body.name)?;
    let lock = app.resource_lock("mcp-endpoints");
    let _guard = lock.lock().await;
    // Under the lock, so a container deleted meanwhile cannot be granted.
    let grants = grants(&app, body.grants)?;
    if app.db.contains(ENDPOINTS, &body.name)? {
        return Err(Error(
            StatusCode::CONFLICT,
            format!("An MCP endpoint named {} exists", body.name),
        ));
    }
    let token = new_token();
    let endpoint = Endpoint {
        name: body.name,
        automatic: false,
        grants,
        token_hash: Some(hashed(&token)),
        token_created_at: Some(now()),
        created_at: now(),
    };
    app.db.put(ENDPOINTS, &endpoint.name, &endpoint)?;
    app.desktop_mcp.report_changes();
    Ok(Json(
        json!({"endpoint": summary(&app, &endpoint), "token": token}),
    ))
}

pub async fn update_endpoint(
    State(app): State<Arc<App>>,
    Path(name): Path<String>,
    Json(body): Json<UpdateEndpoint>,
) -> Result<Json<Value>> {
    let lock = app.resource_lock("mcp-endpoints");
    let _guard = lock.lock().await;
    let grants = grants(&app, body.grants)?;
    let mut endpoint = load(&app, &name)?;
    endpoint.grants = grants;
    app.db.put(ENDPOINTS, &name, &endpoint)?;
    release(&app, &name);
    Ok(Json(summary(&app, &endpoint)))
}

pub async fn delete_endpoint(
    State(app): State<Arc<App>>,
    Path(name): Path<String>,
) -> Result<Json<Value>> {
    let lock = app.resource_lock("mcp-endpoints");
    let _guard = lock.lock().await;
    load(&app, &name)?;
    app.db.remove(ENDPOINTS, &name)?;
    release(&app, &name);
    forget(&app, &name)?;
    Ok(Json(json!({"deleted": true, "name": name})))
}

/// Create or replace an endpoint's token; the old one stops working at once.
pub async fn token(State(app): State<Arc<App>>, Path(name): Path<String>) -> Result<Json<Value>> {
    let lock = app.resource_lock("mcp-endpoints");
    let _guard = lock.lock().await;
    let mut endpoint = load(&app, &name)?;
    let token = new_token();
    endpoint.token_hash = Some(hashed(&token));
    endpoint.token_created_at = Some(now());
    app.db.put(ENDPOINTS, &name, &endpoint)?;
    // Sessions opened with the old token end with it.
    app.desktop_mcp.close_service(&name);
    app.desktop_mcp.report_changes();
    Ok(Json(
        json!({"endpoint": summary(&app, &endpoint), "token": token}),
    ))
}

pub async fn revoke_token(
    State(app): State<Arc<App>>,
    Path(name): Path<String>,
) -> Result<Json<Value>> {
    let lock = app.resource_lock("mcp-endpoints");
    let _guard = lock.lock().await;
    let mut endpoint = load(&app, &name)?;
    endpoint.token_hash = None;
    endpoint.token_created_at = None;
    app.db.put(ENDPOINTS, &name, &endpoint)?;
    app.desktop_mcp.close_service(&name);
    app.desktop_mcp.report_changes();
    Ok(Json(summary(&app, &endpoint)))
}

pub async fn endpoint_tools(
    State(app): State<Arc<App>>,
    Path(name): Path<String>,
) -> Result<Json<Value>> {
    let endpoint = load(&app, &name)?;
    let items = catalog(&app, &endpoint)
        .into_iter()
        .map(|entry| {
            let (source, container, server) = match &entry.target {
                super::scopes::Target::Server {
                    container_id,
                    server,
                    ..
                } => ("server", json!(container_id), json!(server)),
                super::scopes::Target::Thoric { containers, .. } => (
                    "thoric",
                    json!(if containers.len() == 1 {
                        Some(&containers[0].1)
                    } else {
                        None
                    }),
                    Value::Null,
                ),
                super::scopes::Target::Job { .. }
                | super::scopes::Target::Instance { .. }
                | super::scopes::Target::Set { .. } => ("thoric", Value::Null, Value::Null),
                super::scopes::Target::InstanceServer { server, .. }
                | super::scopes::Target::HomeServer { server, .. } => {
                    ("server", Value::Null, json!(server))
                }
            };
            json!({
                "name": entry.tool.name,
                "description": entry.tool.description,
                "source": source,
                "container_id": container,
                "server": server,
            })
        })
        .collect::<Vec<_>>();
    Ok(Json(json!({"items": items})))
}

pub async fn scopes_list() -> Json<Value> {
    Json(describe())
}

/// Give a new container an endpoint of its own, named after it, with the default scopes, the
/// tools of `profile` among them, all its desktop servers and no token yet. Returns its name.
pub(crate) async fn automatic_endpoint(
    app: &App,
    container: &Container,
    profile: Profile,
) -> anyhow::Result<String> {
    let lock = app.resource_lock("mcp-endpoints");
    let _guard = lock.lock().await;
    // An endpoint is saved under its name, and one that does not read keeps its name too.
    let taken = app.db.ids(ENDPOINTS)?;
    let name = unique(&slug_of(&container.spec.name, 48, "desktop"), 48, |n| {
        taken.iter().any(|t| t == n)
    });
    let endpoint = Endpoint {
        name: name.clone(),
        automatic: true,
        grants: vec![Grant {
            container_id: container.id.clone(),
            set: None,
            alias: slug_of(&container.spec.name, 24, "desktop"),
            scopes: Scope::DEFAULT.to_vec(),
            profile,
            servers: Servers::All,
            excluded_tools: Vec::new(),
        }],
        token_hash: None,
        token_created_at: None,
        created_at: now(),
    };
    app.db.put(ENDPOINTS, &name, &endpoint)?;
    app.desktop_mcp.report_changes();
    Ok(name)
}

/// Give a scale set an endpoint of its own, named after it, with the default scopes, `scale`,
/// the tools of `profile` among them, all its instances' desktop servers and no token yet.
/// Returns its name.
pub(crate) async fn set_endpoint(app: &App, set: &str, profile: Profile) -> anyhow::Result<String> {
    let lock = app.resource_lock("mcp-endpoints");
    let _guard = lock.lock().await;
    let taken = app.db.ids(ENDPOINTS)?;
    let name = unique(&slug_of(set, 48, "set"), 48, |n| {
        taken.iter().any(|t| t == n)
    });
    let mut scopes = Scope::DEFAULT.to_vec();
    scopes.push(Scope::Scale);
    let endpoint = Endpoint {
        name: name.clone(),
        automatic: true,
        grants: vec![Grant {
            container_id: String::new(),
            set: Some(set.into()),
            alias: slug_of(set, 24, "set"),
            scopes,
            profile,
            servers: Servers::All,
            excluded_tools: Vec::new(),
        }],
        token_hash: None,
        token_created_at: None,
        created_at: now(),
    };
    app.db.put(ENDPOINTS, &name, &endpoint)?;
    app.desktop_mcp.report_changes();
    Ok(name)
}

/// Remove a deleted scale set from every endpoint, and its own endpoint with it.
pub(crate) fn forget_set(app: &App, set: &str) -> anyhow::Result<()> {
    for mut endpoint in app.db.list::<Endpoint>(ENDPOINTS)? {
        if !endpoint.set().is_some_and(|(_, name)| name == set) {
            continue;
        }
        endpoint.grants.clear();
        if endpoint.automatic {
            app.db.remove(ENDPOINTS, &endpoint.name)?;
            forget(app, &endpoint.name)?;
        } else {
            // An endpoint left without its set keeps its name and token for new grants.
            app.db.put(ENDPOINTS, &endpoint.name, &endpoint)?;
        }
        release(app, &endpoint.name);
    }
    Ok(())
}

/// Remove a deleted container from every endpoint, and its own endpoint with it.
pub(crate) fn forget_container(app: &App, container_id: &str) -> anyhow::Result<()> {
    for mut endpoint in app.db.list::<Endpoint>(ENDPOINTS)? {
        if !endpoint
            .grants
            .iter()
            .any(|grant| grant.container_id == container_id)
        {
            continue;
        }
        endpoint
            .grants
            .retain(|grant| grant.container_id != container_id);
        if endpoint.automatic && endpoint.grants.is_empty() {
            app.db.remove(ENDPOINTS, &endpoint.name)?;
            forget(app, &endpoint.name)?;
        } else {
            // An endpoint left without containers keeps its name and token for new grants.
            app.db.put(ENDPOINTS, &endpoint.name, &endpoint)?;
        }
        release(app, &endpoint.name);
    }
    Ok(())
}
