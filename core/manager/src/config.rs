//! The installation's settings: environment variables the manager reads once when it starts, with
//! `.env.example` as their catalog. The settings of [`EDITABLE`] can also be changed from the
//! dashboard's Settings page, which in the WSL package writes them into `.env` and recreates the
//! manager; see [`changes`].
use crate::model::LABEL;
use anyhow::{Context, Result, bail};
use argon2::{Argon2, PasswordHasher};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap, env, path::PathBuf};

pub(crate) mod changes;

pub struct Config {
    pub listen: String,
    pub data: PathBuf,
    pub web: PathBuf,
    pub docker_bin: String,
    /// The Engine API socket for pulls with byte progress; without one, pulls use the CLI.
    pub docker_socket: Option<PathBuf>,
    pub manager: Option<String>,
    pub base_image: String,
    pub desktop_context: PathBuf,
    pub auth: bool,
    pub username: String,
    pub password_hash: Option<String>,
    pub secure_cookie: bool,
    pub audio: bool,
    pub microphone: bool,
    /// Whether desktops may have internet access, `sudo` and a virtual gamepad. Like `audio`, a
    /// switched-off feature cannot be chosen for a desktop; see [`crate::App::installation_allows`].
    pub internet: bool,
    pub sudo: bool,
    pub gamepad: bool,
    /// Whether desktops may get `NET_ADMIN` and `/dev/net/tun`, which a VPN client needs; see
    /// [`crate::App::installation_allows`].
    pub vpn: bool,
    /// The features of a new container as the installation sets them; see [`Config::feature_defaults`].
    pub defaults: Features,
    pub wsl_gpu: bool,
    pub cuda: bool,
    /// Whether desktops may get the render devices under `/dev/dri` where the host has some.
    pub dri_gpu: bool,
    pub adapter: String,
    /// The GPU devices the Docker host showed when the manager started. Not a setting: the GPU
    /// settings allow the routes, and these decide which of them desktops get; see [`crate::gpu`].
    pub gpu_host: crate::gpu::Host,
    /// The logging driver Docker gives a container that names none, as `docker info` said when
    /// the manager started, or `None` when it did not answer. Not a setting: desktops get log
    /// options of their own only under json-file; see [`crate::desktop_limits`].
    pub log_driver: Option<String>,
    /// The architecture of the images the Docker host runs, such as `amd64` or `arm64`, as
    /// `docker info` said when the manager started. Not a setting: a download measures the image
    /// of this architecture; see [`crate::pulls::image_architecture`].
    pub architecture: String,
    /// Whether desktops may create user namespaces, which is how Chromium-based browsers and
    /// Electron applications sandbox their processes. See [`Config::seccomp_profile`].
    pub user_namespaces: bool,
    /// How many processes and threads a desktop may have, `0` for no limit; see
    /// [`crate::desktop_limits`].
    pub pids_limit: u64,
    /// The names `ALLOWED_HOSTS` adds to the hosts the manager answers.
    pub allowed_hosts: Vec<String>,
    /// The name of the optional HTTPS service, which the manager answers as well. Compose passes
    /// it on its own, so `ALLOWED_HOSTS` stays what `.env` says.
    pub desktop_host: Option<String>,
    pub upload_limit: u64,
    /// The largest unpacked image archive an import accepts, in bytes.
    pub archive_limit: u64,
    /// Whether the root `/mcp` answers: the administrator's agent channel to every container, the
    /// one not made on the dashboard. The MCP endpoints at `/mcp/<name>` answer either way.
    pub root_mcp_enabled: bool,
    /// The administrator's bearer token, which `/mcp` requires and which opens every MCP endpoint
    /// too, or `None` when `MCP_AUTH_MODE=none` turns the check off for both. Empty when no token
    /// is set, which opens nothing.
    pub mcp_token: Option<String>,
    /// The bearer token `/metrics` asks for, or `None` while `METRICS_TOKEN` is empty, which keeps
    /// the route off; see [`crate::metrics`].
    pub metrics_token: Option<String>,
    /// The template list the templates page reads in the browser, or `None` to turn it off.
    pub templates_url: Option<String>,
    /// The label a volume carries for a request to list and choose it, `key` or `key=value`, or
    /// `None` for every volume. See [`volume_filter`].
    pub volume_filter: Option<String>,
    /// The proxy a container chooses with `{"mode": "installation"}`, from `PROXY_URL`, its
    /// credentials and `PROXY_BYPASS`; see [`crate::outbound`].
    pub proxy: Option<crate::outbound::InstallationProxy>,
    /// Whether the manager looks for newer releases, which the dashboard then offers.
    pub update_check: bool,
    /// Where the WSL package takes update requests from the dashboard; see [`crate::releases`].
    pub update_dir: PathBuf,
    /// What startup has to say about the settings it was given, such as a renamed variable that
    /// is still set; logged once the manager starts.
    pub notices: Vec<String>,
    /// The environment the manager read its settings from, without `ADMIN_PASSWORD`, of which it
    /// keeps only the hash. A change from the dashboard is checked against it, and the editable
    /// settings' text in it is what the Settings page starts from.
    pub environment: BTreeMap<String, String>,
}
/// The features of a new container: what the create form starts with, and what a create request gets
/// when it leaves one out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Features {
    pub gpu: bool,
    pub internet: bool,
    pub sudo: bool,
    pub audio: bool,
    pub microphone: bool,
    pub gamepad: bool,
    pub vpn: bool,
    /// Minutes a new container's desktop may go unused before it is stopped, `0` for never; see
    /// [`crate::lifecycle`].
    pub idle_stop_minutes: u32,
}
/// A variable a release renamed. An installation keeps its `.env` across upgrades, so the old name
/// keeps its meaning while the new one is not set, and startup says to rename it.
pub(crate) struct Renamed {
    pub old: &'static str,
    pub new: &'static str,
    /// The release that renamed it.
    pub release: &'static str,
}
pub(crate) const RENAMED: &[Renamed] = &[Renamed {
    old: "MCP_ENABLED",
    new: "ROOT_MCP_ENABLED",
    release: "0.6.3",
}];
/// The settings the dashboard's Settings page may change, through the WSL package. The rest, such
/// as the sign-in, the root MCP and its token, and what only Compose reads, change only in `.env`,
/// with `thoric settings` in the package. `DASHBOARD_SETTINGS` in `packaging/wsl/thoric`,
/// the list the package accepts, is the same; the tests hold the two together.
pub(crate) const EDITABLE: &[&str] = &[
    "SECURE_COOKIE",
    "ALLOWED_HOSTS",
    "DESKTOP_IMAGE",
    "DESKTOP_USER_NAMESPACES",
    "DESKTOP_PIDS_LIMIT",
    "WSL_GPU",
    "NVIDIA_COMPUTE",
    "GPU_ADAPTER",
    "AUDIO_ENABLED",
    "MICROPHONE_ENABLED",
    "INTERNET_ENABLED",
    "SUDO_ENABLED",
    "GAMEPAD_ENABLED",
    "VPN_ENABLED",
    "GPU_DEFAULT",
    "INTERNET_DEFAULT",
    "SUDO_DEFAULT",
    "AUDIO_DEFAULT",
    "MICROPHONE_DEFAULT",
    "GAMEPAD_DEFAULT",
    "VPN_DEFAULT",
    "IDLE_STOP_DEFAULT",
    "UPLOAD_MAX_MB",
    "IMAGE_ARCHIVE_MAX_MB",
    "TEMPLATES_URL",
    "VOLUME_FILTER",
    "PROXY_URL",
    "PROXY_USERNAME",
    "PROXY_BYPASS",
    "UPDATE_CHECK",
];
/// The template list on the project's GitHub wiki.
pub(crate) const TEMPLATES_URL: &str =
    "https://raw.githubusercontent.com/wiki/co3moz/thoric/templates/index.json";
/// What [`Config::parse`] reads the environment for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reading {
    /// The manager starting.
    Startup,
    /// A change from the dashboard, read against the environment the manager started with. Only
    /// the editable settings change, so the password, which the manager keeps only as a hash, is
    /// not read again.
    Change,
}
fn flag_value(key: &str, value: Option<&str>, default: bool) -> Result<bool> {
    match value {
        Some("true" | "1") => Ok(true),
        Some("false" | "0") => Ok(false),
        None => Ok(default),
        _ => bail!("{key} must be true or false"),
    }
}
/// A number of MiB within `range`.
fn mib_value(
    key: &str,
    value: Option<&str>,
    default: u64,
    range: std::ops::RangeInclusive<u64>,
) -> Result<u64> {
    let refused = || {
        format!(
            "{key} must be a whole number of MiB between {} and {}",
            range.start(),
            range.end()
        )
    };
    let mib = match value {
        None => default,
        Some(text) => text.parse::<u64>().with_context(refused)?,
    };
    if !range.contains(&mib) {
        bail!(refused());
    }
    Ok(mib)
}
/// The value of a renamed setting under either of its names. Empty counts as not set, since
/// Compose passes the old name empty once `.env` no longer has it.
pub(crate) fn renamed_value(
    get: &dyn Fn(&str) -> Option<String>,
    renamed: &Renamed,
    notices: &mut Vec<String>,
) -> Result<Option<String>> {
    let set = |key: &str| get(key).filter(|value| !value.is_empty());
    let Renamed { old, new, release } = renamed;
    match (set(new), set(old)) {
        (new_value, None) => Ok(new_value),
        (None, Some(old_value)) => {
            notices.push(format!(
                "{old} was renamed {new} in Thoric {release}. Its value still applies; rename it in .env."
            ));
            Ok(Some(old_value))
        }
        (Some(new_value), Some(old_value)) if new_value == old_value => {
            notices.push(format!(
                "{old} was renamed {new} in Thoric {release}, and {new} applies. Remove {old} from .env."
            ));
            Ok(Some(new_value))
        }
        (Some(new_value), Some(old_value)) => bail!(
            "{old}={old_value} and {new}={new_value} disagree. {old} was renamed {new} in Thoric {release}; remove {old} from .env"
        ),
    }
}
/// Whether `value` can stand as `KEY=value` in `.env` and come back unchanged: one line of
/// printable ASCII, without the quotes, `$`, `#`, backslashes and backquotes that Compose's
/// quoting and interpolation would read otherwise, and without the spaces at either end that it
/// trims. `thoric apply-settings` checks a value from the dashboard the same way.
pub(crate) fn env_value(key: &str, value: &str) -> Result<()> {
    if value.len() > 2048
        || value.trim() != value
        || !value
            .bytes()
            .all(|b| (b' '..=b'~').contains(&b) && !b"\"'`$#\\".contains(&b))
    {
        bail!(
            "{key} must be one line of at most 2048 printable ASCII characters, without quotes, $, #, backslashes, backquotes or spaces at either end"
        );
    }
    Ok(())
}
/// The names of a comma-separated list, as Host headers carry them.
fn host_names(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}
/// What is wrong with a settings change from the dashboard: each setting's own problem, found
/// with the others as they are, and one the changes have only together.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Problems {
    pub fields: BTreeMap<String, String>,
    pub together: Option<String>,
}
impl Problems {
    pub(crate) fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.together.is_none()
    }
}
impl std::fmt::Display for Problems {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let messages: Vec<&str> = self
            .fields
            .values()
            .chain(&self.together)
            .map(String::as_str)
            .collect();
        f.write_str(&messages.join("; "))
    }
}
impl Config {
    pub fn load() -> Result<Self> {
        Self::parse(&|key| env::var(key).ok(), Reading::Startup)
    }

    /// The configuration the manager would start with once `changes` are in its environment,
    /// read by the same code as at startup: a change it would refuse then is refused before
    /// anything restarts.
    pub(crate) fn with_changes(&self, changes: &BTreeMap<String, String>) -> Result<Self> {
        let get = |key: &str| changes.get(key).or(self.environment.get(key)).cloned();
        Self::parse(&get, Reading::Change)
    }

    /// Check a settings change from the dashboard: only [`EDITABLE`] settings, each a value
    /// `.env` keeps as it is, which the manager accepts alone and together with the others.
    pub(crate) fn check_changes(&self, changes: &BTreeMap<String, String>) -> Problems {
        let mut problems = Problems::default();
        for (key, value) in changes {
            let problem = if !EDITABLE.contains(&key.as_str()) {
                Some(format!(
                    "{key} changes only in .env, with thoric settings in the WSL package"
                ))
            } else if let Err(error) = env_value(key, value) {
                Some(format!("{error:#}"))
            } else {
                let alone = BTreeMap::from([(key.clone(), value.clone())]);
                self.with_changes(&alone)
                    .err()
                    .map(|error| format!("{error:#}"))
            };
            if let Some(problem) = problem {
                problems.fields.insert(key.clone(), problem);
            }
        }
        if problems.is_empty() {
            problems.together = self
                .with_changes(changes)
                .err()
                .map(|error| format!("{error:#}"));
        }
        problems
    }

    fn parse(get: &dyn Fn(&str) -> Option<String>, reading: Reading) -> Result<Self> {
        let environment = RefCell::new(BTreeMap::new());
        // Every value read is kept, so a change can be read against the same environment.
        let var = |key: &str| {
            let value = get(key);
            if let Some(value) = &value
                && key != "ADMIN_PASSWORD"
            {
                environment
                    .borrow_mut()
                    .insert(key.to_owned(), value.clone());
            }
            value
        };
        let flag = |key: &str, default: bool| flag_value(key, var(key).as_deref(), default);
        let mode = var("AUTH_MODE").unwrap_or_else(|| "password".into());
        if mode != "password" && mode != "none" {
            bail!("AUTH_MODE must be password or none");
        }
        let auth = mode == "password";
        let password_hash = if auth && reading == Reading::Startup {
            let password = var("ADMIN_PASSWORD").unwrap_or_default();
            if password.chars().count() < 8 || password.len() > 4096 {
                bail!(
                    "ADMIN_PASSWORD must have at least 8 characters and at most 4096 bytes (or use AUTH_MODE=none)"
                );
            }
            // A random salt of 16 bytes, from the system's generator.
            Some(
                Argon2::default()
                    .hash_password(password.as_bytes())
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?
                    .to_string(),
            )
        } else {
            None
        };
        let audio = flag("AUDIO_ENABLED", true)?;
        let mut notices = Vec::new();
        let root_mcp = renamed_value(&var, &RENAMED[0], &mut notices)?;
        let root_mcp_enabled = flag_value("ROOT_MCP_ENABLED", root_mcp.as_deref(), false)?;
        let mcp_token = mcp_token(
            root_mcp_enabled,
            var("MCP_AUTH_MODE").as_deref(),
            var("MCP_TOKEN").unwrap_or_default(),
        )?;
        let upload_mb = mib_value(
            "UPLOAD_MAX_MB",
            var("UPLOAD_MAX_MB").as_deref(),
            1024,
            1..=65536,
        )?;
        let archive_mb = mib_value(
            "IMAGE_ARCHIVE_MAX_MB",
            var("IMAGE_ARCHIVE_MAX_MB").as_deref(),
            65536,
            1..=1_048_576,
        )?;
        let proxy = crate::outbound::installation(&var, &mut notices)?;
        let mut cfg = Self {
            listen: var("LISTEN_ADDR").unwrap_or_else(|| "0.0.0.0:8080".into()),
            data: var("DATA_DIR").unwrap_or_else(|| "data".into()).into(),
            web: var("WEB_DIR")
                .unwrap_or_else(|| "core/web/dist".into())
                .into(),
            docker_bin: var("DOCKER_BIN").unwrap_or_else(|| "docker".into()),
            docker_socket: docker_socket(var("DOCKER_HOST").as_deref()),
            manager: var("MANAGER_CONTAINER").filter(|s| !s.is_empty()),
            base_image: var("DESKTOP_IMAGE")
                .unwrap_or_else(|| crate::catalog::DEFAULT_IMAGE.into()),
            desktop_context: var("DESKTOP_CONTEXT")
                .unwrap_or_else(|| "images/desktop".into())
                .into(),
            username: var("ADMIN_USERNAME").unwrap_or_else(|| "admin".into()),
            auth,
            password_hash,
            secure_cookie: flag("SECURE_COOKIE", false)?,
            audio,
            microphone: audio && flag("MICROPHONE_ENABLED", true)?,
            internet: flag("INTERNET_ENABLED", true)?,
            sudo: flag("SUDO_ENABLED", true)?,
            gamepad: flag("GAMEPAD_ENABLED", true)?,
            vpn: flag("VPN_ENABLED", true)?,
            // As the create form chose before these settings: GPU access where the installation
            // has it, internet and sudo, and neither sound, a gamepad nor a VPN.
            defaults: Features {
                gpu: flag("GPU_DEFAULT", true)?,
                internet: flag("INTERNET_DEFAULT", true)?,
                sudo: flag("SUDO_DEFAULT", true)?,
                audio: flag("AUDIO_DEFAULT", false)?,
                microphone: flag("MICROPHONE_DEFAULT", false)?,
                gamepad: flag("GAMEPAD_DEFAULT", false)?,
                vpn: flag("VPN_DEFAULT", false)?,
                idle_stop_minutes: crate::lifecycle::idle_stop_default(
                    var("IDLE_STOP_DEFAULT").as_deref(),
                )?,
            },
            wsl_gpu: flag("WSL_GPU", true)?,
            cuda: flag("NVIDIA_COMPUTE", true)?,
            dri_gpu: flag("DRI_GPU", true)?,
            adapter: var("GPU_ADAPTER").unwrap_or_default(),
            gpu_host: crate::gpu::Host::default(),
            log_driver: None,
            architecture: crate::pulls::image_architecture(None),
            user_namespaces: flag("DESKTOP_USER_NAMESPACES", true)?,
            pids_limit: crate::desktop_limits::pids_limit(var("DESKTOP_PIDS_LIMIT").as_deref())?,
            allowed_hosts: host_names(var("ALLOWED_HOSTS").as_deref()),
            desktop_host: host_names(var("DESKTOP_HOST").as_deref())
                .into_iter()
                .next(),
            upload_limit: upload_mb * 1024 * 1024,
            archive_limit: archive_mb * 1024 * 1024,
            root_mcp_enabled,
            mcp_token,
            metrics_token: metrics_token(var("METRICS_TOKEN").as_deref())?,
            templates_url: templates_url(var("TEMPLATES_URL").as_deref())?,
            volume_filter: volume_filter(var("VOLUME_FILTER").as_deref())?,
            proxy,
            update_check: flag("UPDATE_CHECK", true)?,
            update_dir: crate::releases::UPDATE_DIR.into(),
            notices,
            environment: BTreeMap::new(),
        };
        super::model::image_ref(&cfg.base_image).context("DESKTOP_IMAGE")?;
        if let Some(profile) = cfg.seccomp_profile()
            && !profile.is_file()
        {
            bail!(
                "{} is missing; restore it or set DESKTOP_USER_NAMESPACES=false to give desktops Docker's default seccomp profile",
                profile.display()
            );
        }
        if cfg.auth && (cfg.username.trim().is_empty() || cfg.username.len() > 256) {
            bail!("ADMIN_USERNAME must not be empty and must be at most 256 bytes");
        }
        cfg.environment = environment.into_inner();
        Ok(cfg)
    }

    /// The seccomp profile desktops are created with, or `None` for Docker's default.
    ///
    /// Docker's default profile refuses `clone` and `unshare` with namespace flags to a container
    /// without `CAP_SYS_ADMIN`. Chrome and Electron applications then cannot start their sandbox
    /// and abort unless they get `--no-sandbox`, which the desktop's menu entries and default web
    /// browser never pass. `images/desktop/seccomp.json` is Docker's default with one rule added
    /// that allows both calls; everything that needs `CAP_SYS_ADMIN`, such as `mount` and
    /// `setns`, stays refused. The manager image bundles it with the desktop build context.
    pub(crate) fn seccomp_profile(&self) -> Option<PathBuf> {
        self.user_namespaces
            .then(|| self.desktop_context.join("seccomp.json"))
    }

    /// The settings of `.env.example` that Compose passes to the manager, in the order the settings
    /// page lists them, each with the value the manager runs with rather than the text it was
    /// given: an empty `TEMPLATES_URL` shows the wiki's list. A secret says only whether it is
    /// set; its value never leaves the manager. An [`EDITABLE`] setting also has the text its
    /// environment gave, empty when none did, which the Settings page edits.
    pub(crate) fn shown(&self) -> Vec<Value> {
        let value = |name: &str, value: Value| {
            let editable = EDITABLE.contains(&name);
            let mut entry = json!({"name": name, "value": value, "editable": editable});
            if editable {
                entry["text"] = json!(self.environment.get(name).cloned().unwrap_or_default());
            }
            entry
        };
        let secret = |name: &str, set: bool| json!({"name": name, "secret": true, "set": set});
        let mib = |bytes: u64| json!(bytes / (1024 * 1024));
        let on = |enabled: bool, mode: &'static str| if enabled { mode } else { "none" };
        vec![
            value("AUTH_MODE", json!(on(self.auth, "password"))),
            value("ADMIN_USERNAME", json!(self.username)),
            secret("ADMIN_PASSWORD", self.password_hash.is_some()),
            value("SECURE_COOKIE", json!(self.secure_cookie)),
            value("ALLOWED_HOSTS", json!(self.allowed_hosts.join(","))),
            value("DESKTOP_HOST", json!(self.desktop_host)),
            value("MANAGER_CONTAINER", json!(self.manager)),
            value("ROOT_MCP_ENABLED", json!(self.root_mcp_enabled)),
            value(
                "MCP_AUTH_MODE",
                json!(on(self.mcp_token.is_some(), "token")),
            ),
            secret(
                "MCP_TOKEN",
                self.mcp_token
                    .as_ref()
                    .is_some_and(|token| !token.is_empty()),
            ),
            secret("METRICS_TOKEN", self.metrics_token.is_some()),
            value("DESKTOP_IMAGE", json!(self.base_image)),
            value("DESKTOP_USER_NAMESPACES", json!(self.user_namespaces)),
            value("DESKTOP_PIDS_LIMIT", json!(self.pids_limit)),
            value("WSL_GPU", json!(self.wsl_gpu)),
            value("NVIDIA_COMPUTE", json!(self.cuda)),
            value("DRI_GPU", json!(self.dri_gpu)),
            value("GPU_ADAPTER", json!(self.adapter)),
            value("AUDIO_ENABLED", json!(self.audio)),
            value("MICROPHONE_ENABLED", json!(self.microphone)),
            value("INTERNET_ENABLED", json!(self.internet)),
            value("SUDO_ENABLED", json!(self.sudo)),
            value("GAMEPAD_ENABLED", json!(self.gamepad)),
            value("VPN_ENABLED", json!(self.vpn)),
            value("GPU_DEFAULT", json!(self.defaults.gpu)),
            value("INTERNET_DEFAULT", json!(self.defaults.internet)),
            value("SUDO_DEFAULT", json!(self.defaults.sudo)),
            value("AUDIO_DEFAULT", json!(self.defaults.audio)),
            value("MICROPHONE_DEFAULT", json!(self.defaults.microphone)),
            value("GAMEPAD_DEFAULT", json!(self.defaults.gamepad)),
            value("VPN_DEFAULT", json!(self.defaults.vpn)),
            value("IDLE_STOP_DEFAULT", json!(self.defaults.idle_stop_minutes)),
            value("UPLOAD_MAX_MB", mib(self.upload_limit)),
            value("IMAGE_ARCHIVE_MAX_MB", mib(self.archive_limit)),
            value(
                "TEMPLATES_URL",
                json!(self.templates_url.as_deref().unwrap_or("none")),
            ),
            value(
                "VOLUME_FILTER",
                json!(self.volume_filter.as_deref().unwrap_or("none")),
            ),
            value(
                "PROXY_URL",
                json!(self.proxy.as_ref().map(|proxy| proxy.url.as_str())),
            ),
            value(
                "PROXY_USERNAME",
                json!(
                    self.proxy
                        .as_ref()
                        .and_then(|proxy| proxy.username.as_deref())
                ),
            ),
            secret(
                "PROXY_PASSWORD",
                self.proxy
                    .as_ref()
                    .is_some_and(|proxy| proxy.password.is_some()),
            ),
            value(
                "PROXY_BYPASS",
                json!(self.proxy.as_ref().map(|proxy| proxy.bypass.join(","))),
            ),
            value("UPDATE_CHECK", json!(self.update_check)),
        ]
    }

    /// Whether desktops can have GPU access: a route the settings allow that the host has.
    pub(crate) fn gpu_enabled(&self) -> bool {
        !crate::gpu::routes(self).used.is_empty()
    }

    /// The features a new container starts with: the installation's defaults, but never one it
    /// has switched off. The microphone travels over the audio connection, so it is on only
    /// with audio; [`crate::api::create`] decides that for a request that chooses audio itself.
    pub(crate) fn feature_defaults(&self) -> Features {
        let audio = self.defaults.audio && self.audio;
        Features {
            gpu: self.defaults.gpu && self.gpu_enabled(),
            internet: self.defaults.internet && self.internet,
            sudo: self.defaults.sudo && self.sudo,
            audio,
            microphone: self.defaults.microphone && self.microphone && audio,
            gamepad: self.defaults.gamepad && self.gamepad,
            vpn: self.defaults.vpn && self.vpn,
            idle_stop_minutes: self.defaults.idle_stop_minutes,
        }
    }

    /// Whether the manager answers requests for this host name.
    pub(crate) fn host_allowed(&self, name: &str) -> bool {
        self.allowed_hosts.iter().any(|host| host == name)
            || self.desktop_host.as_deref() == Some(name)
    }

    /// The scheme and host the templates page reads from, which the interface's content security
    /// policy has to allow.
    pub(crate) fn templates_origin(&self) -> Option<&str> {
        let url = self.templates_url.as_deref()?;
        let host = url.find("://")? + 3;
        Some(&url[..url[host..].find('/').map_or(url.len(), |end| host + end)])
    }
}

/// `METRICS_TOKEN`: empty keeps `/metrics` off, and anything else is the token it asks for, held
/// to what an `Authorization` header carries as it is.
pub(crate) fn metrics_token(value: Option<&str>) -> Result<Option<String>> {
    let token = value.unwrap_or_default();
    if token.is_empty() {
        return Ok(None);
    }
    if token.len() > 256 || !token.bytes().all(|b| b.is_ascii_graphic()) {
        bail!("METRICS_TOKEN must be empty or 1 to 256 visible ASCII characters without spaces");
    }
    Ok(Some(token.into()))
}

/// `TEMPLATES_URL`: empty for the project wiki's list, `none` for no templates page, or the URL
/// of another list. The browser reads it, and the content security policy names its host, so it
/// is held to characters neither can misread.
pub(crate) fn templates_url(value: Option<&str>) -> Result<Option<String>> {
    let value = value.unwrap_or_default().trim();
    if value.is_empty() {
        return Ok(Some(TEMPLATES_URL.into()));
    }
    if value == "none" {
        return Ok(None);
    }
    let host = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .map(|rest| rest.split('/').next().unwrap_or_default());
    if value.len() > 2048
        || host.is_none_or(|host| {
            host.is_empty()
                || !host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
        })
        || !value
            .bytes()
            .all(|b| b.is_ascii_graphic() && !b"\"'`;,<>\\{}|^".contains(&b))
    {
        bail!("TEMPLATES_URL must be an http or https URL, empty for the project wiki, or none");
    }
    Ok(Some(value.into()))
}

/// `VOLUME_FILTER`: the label a volume needs to be listed and chosen, as `key` or `key=value`.
/// Empty keeps the label the manager puts on every volume it creates, so volumes made outside
/// Thoric, the manager's own data volume among them, are out of reach; `none` lists every
/// volume. The manager gives the label to each volume it creates, so the label has to be one
/// `docker volume create --label` and Docker's `label` filter read the same way.
pub(crate) fn volume_filter(value: Option<&str>) -> Result<Option<String>> {
    let value = value.unwrap_or_default().trim();
    if value.is_empty() {
        return Ok(Some(format!("{LABEL}=true")));
    }
    if value == "none" {
        return Ok(None);
    }
    let (key, label) = value.split_once('=').unwrap_or((value, ""));
    if value.len() > 256
        || key.is_empty()
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
        || !label.bytes().all(|b| b.is_ascii_graphic() && b != b',')
    {
        bail!("VOLUME_FILTER must be a label as key or key=value, empty for {LABEL}=true, or none");
    }
    // The ownership label means the same on every volume the manager creates.
    if key == LABEL && !matches!(value.split_once('='), None | Some((_, "true"))) {
        bail!("VOLUME_FILTER can ask for {LABEL} only with the value true");
    }
    Ok(Some(value.into()))
}

/// The socket must reach the daemon the CLI talks to, so it follows the CLI's own `DOCKER_HOST`.
pub(crate) fn docker_socket(host: Option<&str>) -> Option<PathBuf> {
    match host.filter(|host| !host.is_empty()) {
        None => Some("/var/run/docker.sock".into()),
        Some(host) => host.strip_prefix("unix://").map(PathBuf::from),
    }
}

/// An unset mode keeps the token requirement, so only an explicit `none` opens `/mcp` and the MCP
/// endpoints without one. The token itself is required only while the root `/mcp` is on; without
/// it, an endpoint opens with its own token.
pub(crate) fn mcp_token(
    enabled: bool,
    mode: Option<&str>,
    token: String,
) -> Result<Option<String>> {
    match mode.unwrap_or("token") {
        "token" => {
            validate_mcp_token(enabled, &token)?;
            Ok(Some(token))
        }
        "none" => Ok(None),
        _ => bail!("MCP_AUTH_MODE must be token or none"),
    }
}

pub(crate) fn validate_mcp_token(enabled: bool, token: &str) -> Result<()> {
    if enabled
        && (!(1..=256).contains(&token.len()) || !token.bytes().all(|b| b.is_ascii_graphic()))
    {
        bail!(
            "MCP_TOKEN must contain 1 to 256 visible ASCII characters without spaces when ROOT_MCP_ENABLED=true (or use MCP_AUTH_MODE=none)"
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
