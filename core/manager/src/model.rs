use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub const LABEL: &str = "thoric.managed";
/// The database kind of container records. They keep the name they were stored under before
/// environments were called containers, so an existing installation reads them unchanged.
pub const CONTAINERS: &str = "environment";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Mount {
    pub volume: String,
    pub target: String,
    #[serde(default)]
    pub readonly: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ContainerSpec {
    pub name: String,
    pub image: String,
    #[serde(default = "resolution")]
    pub resolution: String,
    #[serde(default)]
    pub gpu: bool,
    #[serde(default = "yes")]
    pub internet: bool,
    #[serde(default)]
    pub audio: bool,
    #[serde(default)]
    pub microphone: bool,
    #[serde(default = "yes")]
    pub sudo: bool,
    /// Attach /dev/uinput so a virtual controller can be created inside the desktop.
    #[serde(default)]
    pub gamepad: bool,
    /// Give the desktop `NET_ADMIN` and `/dev/net/tun`, so a VPN client inside it can open a
    /// tunnel in its own network namespace.
    #[serde(default)]
    pub vpn: bool,
    /// Give the desktop `SYS_PTRACE`, so a debugger run as root attaches to its running programs;
    /// see [`crate::permissions`].
    #[serde(default)]
    pub debugger: bool,
    /// Let the desktop's programs schedule in real time, raise their priority and lock memory.
    #[serde(default)]
    pub realtime: bool,
    /// Give the desktop `PERFMON`, so `perf` reads the performance counters of its programs.
    #[serde(default)]
    pub profiler: bool,
    /// Attach the host's `/dev/kvm`, for virtual machines and emulators inside the desktop.
    #[serde(default)]
    pub kvm: bool,
    /// The proxy the desktop's programs go through, `None` for none; see [`crate::outbound`].
    #[serde(default)]
    pub proxy: Option<crate::outbound::ProxyChoice>,
    /// The machine identity every Docker container created for this one gets (see `identity`).
    /// A new container is given what its request leaves out; records saved before identities
    /// were kept have none until another Docker container is created for them.
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default, deserialize_with = "crate::identity::mac_input")]
    pub mac_address: Option<String>,
    #[serde(default, deserialize_with = "crate::identity::machine_id_input")]
    pub machine_id: Option<String>,
    /// Ports inside the desktop that the manager proxies for holders of the MCP token. The
    /// manager reaches them on the private network, so a change needs no new container.
    #[serde(default)]
    pub ports: Vec<u16>,
    /// The groups the container belongs to: containers that share one reach each other on the
    /// group's network (see `groups`). A change applies in place.
    #[serde(default)]
    pub groups: Vec<String>,
    /// More names the members of its groups reach the container by, beside its hostname and name.
    #[serde(default)]
    pub aliases: Vec<String>,
    pub cpus: Option<f64>,
    pub memory_mb: Option<u64>,
    #[serde(default = "shm")]
    pub shm_mb: u64,
    #[serde(default)]
    pub mounts: Vec<Mount>,
    /// When the manager stops or deletes the container by itself (see `lifecycle`). It is the
    /// manager's alone, so a change applies in place; records saved before it existed never do.
    #[serde(default)]
    #[schemars(with = "crate::lifecycle::LifecycleRequest")]
    pub lifecycle: crate::lifecycle::Lifecycle,
}
fn resolution() -> String {
    "1920x1080".into()
}
fn yes() -> bool {
    true
}
/// Leaves a setting that is on by default out of what is written.
pub(crate) fn is_true(value: &bool) -> bool {
    *value
}
fn shm() -> u64 {
    512
}

impl ContainerSpec {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() || self.name.chars().count() > 100 {
            bail!("Container name must be 1 to 100 characters");
        }
        image_ref(&self.image)?;
        let (w, h) = self
            .resolution
            .split_once('x')
            .ok_or_else(|| anyhow::anyhow!("Resolution must use WIDTHxHEIGHT"))?;
        let (w, h) = (w.parse::<u32>()?, h.parse::<u32>()?);
        if !(320..=7680).contains(&w) || !(240..=4320).contains(&h) {
            bail!("Resolution must be between 320x240 and 7680x4320");
        }
        if let Some(c) = self.cpus
            && (!c.is_finite() || c <= 0.0)
        {
            bail!("CPU limit must be positive");
        }
        if let Some(m) = self.memory_mb
            && (!(256..=u64::MAX / 1_048_576).contains(&m))
        {
            bail!("Memory must be at least 256 MB");
        }
        if self.shm_mb < 64 || self.shm_mb > u64::MAX / 1_048_576 {
            bail!("Shared memory must be at least 64 MB");
        }
        if self.microphone && !self.audio {
            bail!("Enable audio to use the microphone");
        }
        if let Some(proxy) = &self.proxy {
            proxy.validate()?;
        }
        if let Some(name) = &self.hostname {
            crate::identity::hostname(name)?;
        }
        if let Some(mac) = &self.mac_address {
            crate::identity::mac_address(mac)?;
        }
        if let Some(id) = &self.machine_id {
            crate::identity::machine_id(id)?;
        }
        let mut ports = std::collections::HashSet::new();
        if self.ports.len() > 32
            || self
                .ports
                .iter()
                .any(|port| *port == 0 || !ports.insert(port))
        {
            bail!("Proxied ports must be at most 32 different ports between 1 and 65535");
        }
        crate::groups::validate(self)?;
        self.lifecycle.validate()?;
        let mut targets = std::collections::HashSet::new();
        for mount in &self.mounts {
            volume_name(&mount.volume)?;
            mount_target(&mount.target)?;
            if !targets.insert(&mount.target) {
                bail!("Volume targets must be unique");
            }
            if mount.readonly
                && (mount.target == "/home/desktop" || mount.target.starts_with("/home/desktop/."))
            {
                bail!("Desktop home and configuration paths must be writable");
            }
        }
        Ok(())
    }
}
pub fn image_ref(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 255
        || value.starts_with('-')
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._:@-".contains(&b))
    {
        bail!("Invalid Docker image reference");
    }
    Ok(())
}
/// A path a volume can be mounted at in a desktop: absolute and normalized, and none of the
/// system folders or the paths the manager mounts itself.
pub fn mount_target(target: &str) -> Result<()> {
    if !target.starts_with('/')
        || target == "/"
        || target.contains([',', '\n', '\r', '\0'])
        || target[1..]
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
    {
        bail!("Invalid volume target");
    }
    if infrastructure(target) {
        bail!("Volume target conflicts with desktop infrastructure");
    }
    Ok(())
}
/// Whether `path` is one of the desktop's system folders, inside one, or a parent folder that
/// holds more of them, such as `/opt` or `/home`.
fn infrastructure(path: &str) -> bool {
    [
        "/proc",
        "/sys",
        "/dev",
        "/run",
        "/tmp",
        "/usr",
        "/etc",
        "/bin",
        "/sbin",
        "/lib",
        "/lib64",
        "/var/run",
        "/var/lock",
        "/opt/VirtualGL",
    ]
    .iter()
    .any(|p| path == *p || path.starts_with(&format!("{p}/")))
        || ["/", "/opt", "/var", "/home"].contains(&path)
}
/// A folder of the desktop that a compose names: a canonical absolute path outside the system
/// folders, which the setup script receives and the upload dialog offers.
pub fn desktop_folder(path: &str) -> Result<()> {
    crate::apps::directory(path)?;
    if infrastructure(path) {
        bail!("A folder outside the desktop's system folders is needed");
    }
    Ok(())
}
/// The id a template has on the templates page: lowercase letters, digits and dashes.
pub fn template_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || value.starts_with('-')
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        bail!("Invalid template id");
    }
    Ok(())
}
pub fn volume_name(value: &str) -> Result<()> {
    if value.len() < 2
        || value.len() > 128
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        bail!("Invalid volume name");
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Container {
    pub id: String,
    pub container: String,
    pub private_network: String,
    pub egress_network: String,
    pub source_image_id: String,
    pub created_at: u64,
    pub spec: ContainerSpec,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub desktop: Option<crate::catalog::DesktopInfo>,
    /// Set once a settings change recreated the container: the container runs this commit of
    /// its earlier files, while `source_image_id` stays the image the desktop started from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_image: Option<WorkingImage>,
    /// Set when the image had a compose: its setup's progress and the values it was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<crate::compose::Setup>,
}
impl Container {
    /// The image the container is created from.
    pub fn container_image(&self) -> &str {
        self.working_image
            .as_ref()
            .map_or(&self.source_image_id, |image| &image.image_id)
    }
    /// The volumes its compose's volume parameters were given, which its settings neither show
    /// nor change.
    pub fn compose_mounts(&self) -> &[Mount] {
        self.setup.as_ref().map_or(&[], |setup| &setup.mounts)
    }
    /// Every volume the container mounts: those of its settings, then its compose's.
    pub fn mounts(&self) -> Vec<Mount> {
        let mut mounts = self.spec.mounts.clone();
        mounts.extend_from_slice(self.compose_mounts());
        mounts
    }
    /// The folders its compose names, which the upload dialog offers by their labels.
    pub fn places(&self) -> &[crate::compose::Place] {
        self.setup.as_ref().map_or(&[], |setup| &setup.places)
    }
}
/// A commit that carries a container's files over to its replacement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkingImage {
    pub image_id: String,
    pub reference: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageRecord {
    pub id: String,
    pub name: String,
    pub image_id: String,
    pub reference: String,
    pub parent_image_id: Option<String>,
    /// The container a snapshot was taken from. Records saved before environments were called
    /// containers name it `source_environment`.
    #[serde(alias = "source_environment")]
    pub source_container: Option<String>,
    pub kind: String,
    pub created_at: u64,
    pub description: String,
    #[serde(default)]
    pub desktop: Option<crate::catalog::DesktopInfo>,
    /// What a script build ran, so the image can be exported and built again. Snapshots,
    /// imports and builds made before it was recorded have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<BuildConfig>,
    /// What a new container from this image asks for and runs once its desktop is ready. Records
    /// saved before templates were called composes name it `template`.
    #[serde(default, alias = "template", skip_serializing_if = "Option::is_none")]
    pub compose: Option<crate::compose::Compose>,
    /// A small picture lists show beside the name.
    #[serde(
        default,
        deserialize_with = "crate::icons::lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub icon: Option<crate::icons::Icon>,
    /// The templates on the templates page the image was built from, in the order they ran.
    /// Records saved before templates could be merged name one as `template_id`.
    #[serde(
        default,
        alias = "template_id",
        deserialize_with = "one_or_more",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub template_ids: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildConfig {
    /// The base as the request named it: a catalog reference or a local image ID.
    pub base_image: String,
    pub internet: bool,
    /// The script of a build of one part; empty when it has parts.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub script: String,
    /// The scripts of a build merged from several templates, in the order they run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<BuildPart>,
    /// The desktop MCP servers the build declared in the image, in `/etc/thoric/mcp.d`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp: Vec<crate::desktop_mcp::Declaration>,
    /// Whether the image kept the compose of the saved image it was built on, before its own.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub inherit_compose: bool,
}
/// One script of a build merged from several templates, run as root in a layer of its own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BuildPart {
    /// Lowercase letters, digits and dashes, unique in the build, such as the template's id.
    pub id: String,
    pub name: String,
    pub script: String,
}
pub const BUILD_PART_LIMIT: usize = 8;
const BUILD_SCRIPT_LIMIT: usize = 1_000_000;
/// Check what a build runs: one script, or up to eight parts with ids and names of their own.
pub fn build_scripts(script: &str, parts: &[BuildPart]) -> Result<()> {
    match (script.trim().is_empty(), parts.is_empty()) {
        (true, true) => bail!("A build needs a script, or parts with a script each"),
        (false, false) => bail!("A build has a script or parts, not both"),
        _ => {}
    }
    if parts.len() > BUILD_PART_LIMIT {
        bail!("A build has at most {BUILD_PART_LIMIT} parts");
    }
    if script.len() + parts.iter().map(|part| part.script.len()).sum::<usize>() > BUILD_SCRIPT_LIMIT
    {
        bail!("The scripts of a build take at most {BUILD_SCRIPT_LIMIT} bytes together");
    }
    let mut ids = std::collections::HashSet::new();
    for part in parts {
        if template_id(&part.id).is_err() {
            bail!(
                "Each part of a build needs an id of lowercase letters, digits and dashes, at most 64 and not starting with a dash: {:?}",
                part.id
            );
        }
        if !ids.insert(part.id.as_str()) {
            bail!("Part ids must be unique: {}", part.id);
        }
        if part.name.trim().is_empty()
            || part.name.chars().count() > 100
            || part.name.contains(['\n', '\r'])
        {
            bail!("Each part of a build needs a name of one line and at most 100 characters");
        }
        if part.script.trim().is_empty() {
            bail!("The part {} has no script", part.name);
        }
    }
    Ok(())
}
/// A list that records saved before it could hold several wrote as one value, or left out.
pub(crate) fn one_or_more<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMore {
        One(String),
        More(Vec<String>),
    }
    Ok(match Option::<OneOrMore>::deserialize(deserializer)? {
        None => Vec::new(),
        Some(OneOrMore::One(value)) => vec![value],
        Some(OneOrMore::More(values)) => values,
    })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Application {
    pub id: String,
    #[serde(alias = "environment_id")]
    pub container_id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub gpu: bool,
    pub created_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Terminal {
    pub id: String,
    #[serde(alias = "environment_id")]
    pub container_id: String,
    pub root: bool,
    pub cwd: String,
    pub created_at: u64,
    /// The MCP endpoint that opened the terminal, the only one that reaches it besides the
    /// administrator; `None` for one the administrator opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recording {
    pub id: String,
    #[serde(alias = "environment_id")]
    pub container_id: String,
    pub name: String,
    pub fps: u8,
    pub created_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Job {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub resource: Option<String>,
    /// The name a job that makes a container or an image gives it, so a list can show what is on
    /// its way before the record exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub status: String,
    pub created_at: u64,
    pub finished_at: Option<u64>,
    /// The log as the API shows it, from `Db::job`. A saved record keeps only what managers
    /// before the `job_log` table wrote into it, and an empty log otherwise, which those
    /// managers still read; the rest is in that table.
    pub log: String,
    pub result: Option<serde_json::Value>,
    /// Measured progress of the running step, such as an image download.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<serde_json::Value>,
    /// The named stages of a job that reports them, such as a script build, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<crate::stages::Stage>,
    /// What the scripts of a build or a compose setup reported about their steps, kept once the
    /// job ends. Job lists leave it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<crate::script_progress::ScriptReport>,
}
/// How many bytes of a job's log are kept: the last ones.
pub(crate) const LOG_LIMIT: usize = 262_144;
impl Job {
    /// Keep the last [`LOG_LIMIT`] bytes of the log, from the first character that starts in
    /// them.
    pub fn trim_log(&mut self) {
        if self.log.len() > LOG_LIMIT {
            let mut at = self.log.len() - LOG_LIMIT;
            while !self.log.is_char_boundary(at) {
                at += 1;
            }
            self.log.drain(..at);
        }
    }
}
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
