//! What a desktop MCP server is: its declaration, how it is checked, and the record the manager
//! keeps of it with the tools it last read.
use crate::{App, process};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, time::Duration};

/// Reads, watches, writes and removes the declarations inside a desktop, as the desktop user.
pub(crate) const HELPER: &str = include_str!("../../helpers/mcp_registry.py");
/// The database kind of the servers the manager knows, by `<container>:<name>`.
pub(crate) const SERVERS: &str = "desktop-mcp-server";
/// Where an image declares its servers, which a build writes.
pub(crate) const IMAGE_FOLDER: &str = "/etc/thoric/mcp.d";
const ARGUMENT_LIMIT: usize = 100;
const VARIABLE_LIMIT: usize = 50;
const HEADER_LIMIT: usize = 20;
/// The most a server listing may be: both folders' files at the helper's limits, escaped.
const LISTING_LIMIT: usize = 32 << 20;
/// A build declares at most this many servers.
pub(crate) const DECLARATION_LIMIT: usize = 16;
/// The variables the desktop sets for every server, which a declaration cannot change.
const DESKTOP_VARIABLES: [&str; 6] = [
    "DISPLAY",
    "HOME",
    "USER",
    "XDG_RUNTIME_DIR",
    "PULSE_SERVER",
    "PATH",
];
/// Headers the adapter writes itself, or that would change how the request reaches the server.
const RESERVED_HEADERS: [&str; 9] = [
    "accept",
    "connection",
    "content-length",
    "content-type",
    "host",
    "mcp-protocol-version",
    "mcp-session-id",
    "te",
    "transfer-encoding",
];

/// How the manager starts a server: a program on stdio, or a Streamable HTTP server already
/// listening on the desktop's loopback. Values in `args`, `env`, `url` and `headers` may name the
/// container's compose values as `${NAME}`, or `${STEP.NAME}` for one step of several.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerSpec {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

/// A server with its name, as a build, a template or a registration request names it.
#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
pub(crate) struct Declaration {
    pub name: String,
    #[serde(flatten)]
    pub spec: ServerSpec,
}
impl<'de> Deserialize<'de> for Declaration {
    // `deny_unknown_fields` does not work through `flatten`, so the name is taken off first.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut fields = Map::deserialize(deserializer)?;
        let name = match fields.remove("name") {
            Some(Value::String(name)) => name,
            _ => return Err(D::Error::custom("A server needs a name")),
        };
        let spec = ServerSpec::deserialize(Value::Object(fields)).map_err(D::Error::custom)?;
        Ok(Self { name, spec })
    }
}
impl Declaration {
    pub fn validate(&self) -> Result<()> {
        server_name(&self.name)?;
        self.spec.validate()
    }
}

/// Check the servers a build declares: valid, at most [`DECLARATION_LIMIT`], each name once.
pub(crate) fn check_declarations(declarations: &[Declaration]) -> Result<()> {
    if declarations.len() > DECLARATION_LIMIT {
        bail!("A build declares at most {DECLARATION_LIMIT} MCP servers");
    }
    let mut names = std::collections::HashSet::new();
    for declaration in declarations {
        declaration.validate()?;
        if !names.insert(&declaration.name) {
            bail!(
                "A build declares each MCP server once: {}",
                declaration.name
            );
        }
    }
    Ok(())
}

/// Write a build's declarations beside its Dockerfile, and return the steps that put them in the
/// image's folder.
pub(crate) async fn write_declarations(
    dir: &std::path::Path,
    declarations: &[Declaration],
) -> Result<String> {
    let folder = dir.join("mcp");
    tokio::fs::create_dir_all(&folder).await?;
    for declaration in declarations {
        tokio::fs::write(
            folder.join(format!("{}.json", declaration.name)),
            serde_json::to_vec_pretty(&declaration.spec)?,
        )
        .await?;
    }
    Ok(format!(
        "COPY mcp/ {IMAGE_FOLDER}/\nRUN chmod 755 /etc/thoric {IMAGE_FOLDER} && chmod 644 {IMAGE_FOLDER}/*.json\n"
    ))
}

/// Lowercase letters, digits and dashes, at most 32: the file name in the desktop and the prefix
/// of its tools on an endpoint.
pub(crate) fn server_name(name: &str) -> Result<()> {
    if !slug(name, 32) {
        bail!(
            "A server name is lowercase letters, digits and dashes, at most 32, not starting with a dash: {name:?}"
        );
    }
    Ok(())
}
pub(crate) fn slug(value: &str, limit: usize) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn line(value: &str, what: &str, limit: usize) -> Result<()> {
    if value.is_empty() || value.len() > limit || value.chars().any(char::is_control) {
        bail!("{what} must be 1 to {limit} bytes on one line");
    }
    Ok(())
}

impl ServerSpec {
    pub fn transport(&self) -> &'static str {
        if self.url.is_some() { "http" } else { "stdio" }
    }
    pub fn validate(&self) -> Result<()> {
        if self.description.len() > 500 || self.description.contains('\0') {
            bail!("A server description is at most 500 bytes");
        }
        match (&self.command, &self.url) {
            (Some(_), Some(_)) => bail!("A server has a command or a url, not both"),
            (None, None) => bail!("A server needs a command, or the url of an HTTP server"),
            (Some(command), None) => {
                line(command, "The command", 4096)?;
                if !self.headers.is_empty() {
                    bail!("headers are for an HTTP server; a command takes env");
                }
            }
            (None, Some(url)) => {
                loopback_url(url, true)?;
                if !self.args.is_empty() || !self.env.is_empty() || self.cwd.is_some() {
                    bail!("args, env and cwd are for a command; an HTTP server takes headers");
                }
            }
        }
        if self.args.len() > ARGUMENT_LIMIT
            || self
                .args
                .iter()
                .any(|arg| arg.len() > 4096 || arg.contains('\0'))
        {
            bail!(
                "A server takes at most {ARGUMENT_LIMIT} arguments of at most 4096 bytes without NUL"
            );
        }
        if let Some(cwd) = &self.cwd {
            crate::apps::directory(cwd)?;
        }
        if self.env.len() > VARIABLE_LIMIT {
            bail!("A server takes at most {VARIABLE_LIMIT} environment variables");
        }
        for (key, value) in &self.env {
            if key.is_empty()
                || key.len() > 128
                || !key.bytes().enumerate().all(|(i, b)| {
                    b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit())
                })
            {
                bail!("Invalid environment variable name: {key}");
            }
            if DESKTOP_VARIABLES.contains(&key.as_str()) {
                bail!("{key} is set by the desktop and cannot be changed");
            }
            if value.len() > 4096 || value.contains(['\0', '\n', '\r']) {
                bail!("The value of {key} must be at most 4096 bytes on one line");
            }
        }
        if self.headers.len() > HEADER_LIMIT {
            bail!("A server takes at most {HEADER_LIMIT} headers");
        }
        for (name, value) in &self.headers {
            if name.is_empty()
                || name.len() > 64
                || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                bail!("Invalid header name: {name}");
            }
            if RESERVED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                bail!("The {name} header is set by the manager");
            }
            if value.len() > 4096 || value.contains(['\0', '\n', '\r']) {
                bail!("The {name} header must be at most 4096 bytes on one line");
            }
        }
        Ok(())
    }

    /// The declaration with every `${NAME}` replaced by the container's compose value.
    pub fn resolve(&self, values: &BTreeMap<String, String>) -> Result<Self> {
        let mut resolved = self.clone();
        for arg in &mut resolved.args {
            *arg = substitute(arg, values)?;
        }
        for value in resolved.env.values_mut() {
            *value = substitute(value, values)?;
        }
        for value in resolved.headers.values_mut() {
            *value = substitute(value, values)?;
        }
        if let Some(url) = &mut resolved.url {
            *url = substitute(url, values)?;
            loopback_url(url, false)?;
        }
        Ok(resolved)
    }

    /// What the server lists show: the names of its variables and headers, never their values,
    /// and the URL without its query, which may carry a token as a header does.
    pub fn summary(&self) -> Value {
        json!({
            "description": self.description,
            "transport": self.transport(),
            "command": self.command,
            "args": self.args,
            "cwd": self.cwd,
            "env": self.env.keys().collect::<Vec<_>>(),
            "url": self.url.as_deref().map(|url| split_query(url).0),
            "headers": self.headers.keys().collect::<Vec<_>>(),
        })
    }
}

/// A URL as what may be shown and what may not: up to its query or fragment, and the rest,
/// empty when it has none. A server's query is where a token goes when a header cannot.
pub(crate) fn split_query(url: &str) -> (&str, &str) {
    match url.find(['?', '#']) {
        Some(at) => (&url[..at], &url[at..]),
        None => (url, ""),
    }
}

/// Replace `${NAME}` with a compose value. A `$` that does not open one stays as it is.
pub(crate) fn substitute(text: &str, values: &BTreeMap<String, String>) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("${") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 2..];
        let end = after
            .find('}')
            .context("A ${ in a server declaration is not closed")?;
        let name = &after[..end];
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        {
            bail!("${{{name}}} does not name a compose value");
        }
        out.push_str(value(values, name)?);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The compose value `name` names. A declaration written for a compose of one step names its
/// values alone, and keeps them once the image is merged with another or built on: a name
/// without a step stands for the one step's value that has it.
fn value<'a>(values: &'a BTreeMap<String, String>, name: &str) -> Result<&'a str> {
    if let Some(value) = values.get(name) {
        return Ok(value);
    }
    let scoped = values
        .iter()
        .filter(|(key, _)| {
            !name.contains('.') && key.split_once('.').is_some_and(|(_, own)| own == name)
        })
        .collect::<Vec<_>>();
    match scoped.as_slice() {
        [(_, value)] => Ok(value),
        [] => bail!("The container has no compose value {name} for this server"),
        several => bail!(
            "Several steps of the container's compose have a value {name}; name it as {}",
            several
                .iter()
                .map(|(key, _)| key.as_str())
                .collect::<Vec<_>>()
                .join(" or ")
        ),
    }
}

/// An HTTP server's URL: http or https on the desktop's own loopback, with a port. While values
/// are still to be filled in, the port may be one, such as `http://127.0.0.1:${PORT}/mcp`.
pub(crate) fn loopback_url(url: &str, unresolved: bool) -> Result<()> {
    let refuse = || {
        anyhow::anyhow!(
            "An HTTP server's url is http(s)://127.0.0.1, localhost or [::1] with a port and a path, such as http://127.0.0.1:8931/mcp"
        )
    };
    if url.len() > 2048 || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(refuse());
    }
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or_else(refuse)?;
    let (authority, _) = rest.split_once('/').unwrap_or((rest, ""));
    let port = ["127.0.0.1:", "localhost:", "[::1]:"]
        .iter()
        .find_map(|host| authority.strip_prefix(host))
        .ok_or_else(refuse)?;
    let placeholder = unresolved && port.starts_with("${") && port.ends_with('}');
    if !placeholder && port.parse::<u16>().map_or(true, |port| port == 0) {
        return Err(refuse());
    }
    Ok(())
}

/// Where a server's declaration comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Source {
    /// The image's folder: a template or a build declared it.
    Image,
    /// The desktop user's folder: registered while the container runs.
    Container,
}
impl<'de> Deserialize<'de> for Source {
    // A record that says anything else counts as the desktop user's, which gets no secret.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match String::deserialize(deserializer)?.as_str() {
            "image" => Source::Image,
            _ => Source::Container,
        })
    }
}

impl Source {
    /// The compose values a declaration from here may name as `${NAME}`: every one in the image's
    /// folder, which only root changes and the manager reads itself as root, and those that are
    /// not secret in the desktop user's, which that user writes and whose servers run as that
    /// user.
    pub(super) fn values(self, app: &App, container_id: &str) -> BTreeMap<String, String> {
        match self {
            Source::Image => app.compose_values(container_id),
            Source::Container => app.public_compose_values(container_id),
        }
    }
}

/// A server the manager knows, with the tools it last read from it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ServerRecord {
    pub container_id: String,
    pub name: String,
    pub source: Source,
    /// `None` when the declaration could not be read or checked; `error` says why.
    pub spec: Option<ServerSpec>,
    #[serde(default)]
    pub tools: Vec<rmcp::model::Tool>,
    #[serde(default)]
    pub tools_at: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
}
impl ServerRecord {
    pub fn key(container_id: &str, name: &str) -> String {
        format!("{container_id}:{name}")
    }
    /// `ready`, `error` or `pending`, and `stopped` while its container does not run.
    pub fn state(&self, running: bool) -> &'static str {
        if self.error.is_some() {
            "error"
        } else if !running {
            "stopped"
        } else if self.tools_at.is_none() {
            "pending"
        } else {
            "ready"
        }
    }
}

/// One server as the desktop's folders hold it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Found {
    pub name: String,
    pub source: Source,
    pub spec: Result<ServerSpec, String>,
}

/// The servers a helper listed, all from `source`: the folder of the image when the manager read
/// it as root, the desktop user's otherwise, whatever the listing says, since that user can make
/// a helper it runs print anything.
pub(crate) fn found(listing: &Value, source: Source) -> Result<Vec<Found>> {
    let entries = listing["servers"]
        .as_array()
        .context("The desktop's server listing is invalid")?;
    let mut servers: Vec<Found> = Vec::new();
    for entry in entries {
        let name = entry["name"].as_str().unwrap_or_default().to_owned();
        if server_name(&name).is_err() {
            if let Some(error) = entry["error"].as_str() {
                tracing::debug!("Desktop MCP servers: {error}");
            }
            continue;
        }
        let spec = match (&entry["spec"], entry["error"].as_str()) {
            (_, Some(error)) => Err(error.to_owned()),
            (spec, None) => serde_json::from_value::<ServerSpec>(spec.clone())
                .map_err(|e| format!("{name}.json: {e}"))
                .and_then(|spec| {
                    spec.validate()
                        .map(|()| spec)
                        .map_err(|e| format!("{name}.json: {e:#}"))
                }),
        };
        servers.retain(|other| other.name != name);
        servers.push(Found { name, source, spec });
    }
    Ok(servers)
}

/// A desktop's servers: the image's, and the desktop user's, whose declaration of a name replaces
/// the image's.
pub(crate) fn merged(mut image: Vec<Found>, user: Vec<Found>) -> Vec<Found> {
    image.retain(|server| !user.iter().any(|own| own.name == server.name));
    image.extend(user);
    image
}

/// The arguments of a `docker exec` of the helper reading the image's folder as root, which
/// trusts only what root alone can change. Python is isolated from the environment and the
/// working folder, and starts in `/`.
fn image_args(container: &str) -> Vec<String> {
    [
        "exec",
        "--user",
        "0:0",
        "--workdir",
        "/",
        container,
        "python3",
        "-I",
        "-u",
        "-c",
        HELPER,
        "image",
    ]
    .map(str::to_owned)
    .to_vec()
}

/// The arguments of a `docker exec` of the helper, as the desktop user.
fn helper_args(container: &str, interactive: bool, arguments: &[&str]) -> Vec<String> {
    let mut args = vec!["exec".to_owned()];
    if interactive {
        args.push("-i".into());
    }
    args.extend(
        [
            "--user",
            "1001:1001",
            container,
            "python3",
            "-u",
            "-c",
            HELPER,
        ]
        .map(str::to_owned),
    );
    args.extend(arguments.iter().map(|a| (*a).to_owned()));
    args
}
pub(crate) fn watch_args(container: &str) -> Vec<String> {
    helper_args(container, true, &["watch"])
}

impl App {
    /// The servers a running desktop declares now.
    pub(crate) async fn read_servers(&self, container: &str) -> Result<Vec<Found>> {
        let image = self.read_image_servers(container).await?;
        let user = self
            .listing(helper_args(container, false, &["list"]), Source::Container)
            .await?;
        Ok(merged(image, user))
    }
    /// The servers the image of a running desktop declares, read as root.
    pub(crate) async fn read_image_servers(&self, container: &str) -> Result<Vec<Found>> {
        self.listing(image_args(container), Source::Image).await
    }
    async fn listing(&self, args: Vec<String>, source: Source) -> Result<Vec<Found>> {
        // The helper lists up to 64 files of 64 KiB, escaped as JSON, which the 4 MiB of an
        // ordinary Docker call cannot hold.
        let output = process::capture(
            self.command(&args),
            Duration::from_secs(120),
            LISTING_LIMIT,
            process::Overflow::Reject,
        )
        .await?;
        if !output.status.success() {
            bail!("Docker: {}", String::from_utf8_lossy(&output.stderr).trim());
        }
        found(
            &serde_json::from_slice(&output.stdout)
                .context("The desktop's server listing is invalid")?,
            source,
        )
    }
    /// Write the desktop user's declaration of a server, replacing one of the same name.
    pub(crate) async fn write_server(
        &self,
        container: &str,
        declaration: &Declaration,
    ) -> Result<()> {
        let data = serde_json::to_vec_pretty(&declaration.spec)?;
        let output = process::capture_input(
            self.command(&helper_args(
                container,
                true,
                &["write", declaration.name.as_str()],
            )),
            Duration::from_secs(30),
            64 * 1024,
            process::Overflow::Reject,
            Some(data),
        )
        .await?;
        if !output.status.success() {
            bail!(
                "Could not save the server in the desktop: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }
    /// Remove the desktop user's declaration of a server. Returns false when there was none.
    pub(crate) async fn remove_server(&self, container: &str, name: &str) -> Result<bool> {
        let output = process::capture(
            self.command(&helper_args(container, false, &["remove", name])),
            Duration::from_secs(30),
            64 * 1024,
            process::Overflow::Reject,
        )
        .await?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(2) => Ok(false),
            _ => bail!(
                "Could not remove the server from the desktop: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        }
    }
}
