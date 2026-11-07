//! Image composes: parameters asked for when a container is created from a saved image, and
//! setup scripts that receive them as environment variables once the new desktop is ready.
//!
//! A compose is one or more steps, each a script with the parameters it asks for, such as one
//! step for every template an image was built from. A step's script receives only its own
//! parameters, so two steps may ask for the same name: in a compose of several steps a value
//! goes by `<step>.<NAME>`, and a request may still name it alone when only one step asks for
//! it. A compose of one step reads, writes and names its values as composes did before steps.
//!
//! A compose belongs to an image record. A new container pins a copy of it together with the
//! values it was given, so editing the image later changes neither what the container ran nor
//! what a reset runs again. The values live in a record of their own that no route returns:
//! the container shows the values of ordinary parameters and only the names of secret ones,
//! and the setup job's log masks every secret value. An image built on a saved image keeps that
//! image's compose before its own (`inherit`).
pub(crate) mod inherit;
mod parameters;

use crate::{
    App,
    error::{Error, Result},
    model::*,
    script_progress::{ScriptPart, ScriptProgress},
};
use anyhow::{Context, anyhow, bail};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
pub(crate) use parameters::Parameter;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::Duration,
};

const RUNNER: &str = include_str!("../helpers/setup_runner.py");
/// Gives the desktop user the folders of the volume parameters; runs as root.
const PATHS: &str = include_str!("../helpers/setup_paths.py");
/// The database kind of each container's pinned compose and parameter values.
pub(crate) const PINNED: &str = "environment-setup";
const PARAMETER_LIMIT: usize = 32;
const STEP_LIMIT: usize = 8;
/// What the scripts of a compose take together.
const SCRIPT_LIMIT: usize = 1_000_000;
/// The longest `timeout_seconds` a step may ask for: two hours, for setups that download large
/// files such as language models or editors.
pub(crate) const TIMEOUT_LIMIT: u64 = 7200;
/// What a step's `docker exec` gets beyond the script's own timeout, for the runner to start and
/// to stop the script and report it when the timeout is reached.
const RUNNER_GRACE: Duration = Duration::from_secs(120);

/// The steps a new container's setup runs, in order. One step that names nothing is written
/// as composes were before steps: its fields at the top. Several are written as `steps`.
#[derive(Clone, Debug, PartialEq)]
pub struct Compose {
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Names the step in a compose of several, such as the template it came from: lowercase
    /// letters, digits and dashes. Its values go by `<id>.<NAME>`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// What the forms call the step; its id when empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Asked for when a container is created; each one is an environment variable of the script.
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    /// A bash script, run with fail-on-error and stdin closed.
    pub script: String,
    /// Run the script as root instead of the desktop user.
    #[serde(default)]
    pub root: bool,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}
fn default_timeout() -> u64 {
    600
}

/// A compose of several steps as it is written.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Steps<S> {
    steps: S,
}

impl Serialize for Compose {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        match self.steps.as_slice() {
            [step] if step.id.is_empty() && step.name.is_empty() => step.serialize(serializer),
            steps => Steps { steps }.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Compose {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        use serde::de::Error as _;
        let value = Value::deserialize(deserializer)?;
        let steps = if value.get("steps").is_some() {
            Steps::<Vec<Step>>::deserialize(value)
                .map_err(D::Error::custom)?
                .steps
        } else {
            vec![Step::deserialize(value).map_err(D::Error::custom)?]
        };
        Ok(Self { steps })
    }
}

/// The two forms [`Compose`]'s serde reads and writes, for the HTTP API's document.
impl schemars::JsonSchema for Compose {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Compose".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let step = generator.subschema_for::<Step>();
        schemars::json_schema!({
            "oneOf": [
                step.clone(),
                {
                    "type": "object",
                    "properties": {"steps": {"type": "array", "items": step}},
                    "required": ["steps"],
                    "additionalProperties": false,
                },
            ],
        })
    }
}

impl Step {
    /// What messages and forms call the step.
    pub fn title(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.script.trim().is_empty() || self.script.contains('\0') {
            bail!(
                "A compose needs a setup script of 1 to {SCRIPT_LIMIT} bytes without NUL characters"
            );
        }
        if self.name.chars().count() > 100 || self.name.contains(['\n', '\r']) {
            bail!("A step's name is one line of at most 100 characters");
        }
        if !(1..=TIMEOUT_LIMIT).contains(&self.timeout_seconds) {
            bail!("timeout_seconds must be between 1 and {TIMEOUT_LIMIT}");
        }
        if self.parameters.len() > PARAMETER_LIMIT {
            bail!("A compose has at most {PARAMETER_LIMIT} parameters");
        }
        let mut names = HashSet::new();
        for parameter in &self.parameters {
            parameter.validate()?;
            if !names.insert(parameter.name.as_str()) {
                bail!("Parameter names must be unique: {}", parameter.name);
            }
            if let Some(variable) = parameter.volume_variable()
                && self.parameters.iter().any(|p| p.name == variable)
            {
                bail!(
                    "{variable} carries the volume of {}, so no parameter can have that name",
                    parameter.name
                );
            }
        }
        Ok(())
    }
}

impl Compose {
    /// A compose of the one step `step`.
    #[cfg(test)]
    pub fn single(step: Step) -> Self {
        Self { steps: vec![step] }
    }

    /// Whether values go by `<step>.<NAME>`, as they do with several steps.
    fn scoped(&self) -> bool {
        self.steps.len() > 1
    }

    /// The step a scoped value or volume name carries.
    fn scope<'a>(&self, step: &'a Step) -> Option<&'a str> {
        self.scoped().then_some(step.id.as_str())
    }

    /// The name the value of `parameter` of `step` goes by.
    fn key(&self, step: &Step, parameter: &Parameter) -> String {
        match self.scope(step) {
            Some(scope) => format!("{scope}.{}", parameter.name),
            None => parameter.name.clone(),
        }
    }

    /// Every parameter with its step and the name its value goes by, in order.
    fn entries(&self) -> impl Iterator<Item = (&Step, &Parameter, String)> {
        self.steps.iter().flat_map(move |step| {
            step.parameters
                .iter()
                .map(move |parameter| (step, parameter, self.key(step, parameter)))
        })
    }

    /// An error of `step`, which names it when there are several.
    fn within(&self, step: &Step, error: anyhow::Error) -> anyhow::Error {
        if self.scoped() {
            anyhow!("{}: {error}", step.title())
        } else {
            error
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.steps.is_empty() || self.steps.len() > STEP_LIMIT {
            bail!("A compose has 1 to {STEP_LIMIT} steps");
        }
        if self
            .steps
            .iter()
            .map(|step| step.script.len())
            .sum::<usize>()
            > SCRIPT_LIMIT
        {
            bail!(
                "A compose needs a setup script of 1 to {SCRIPT_LIMIT} bytes without NUL characters"
            );
        }
        let mut ids = HashSet::new();
        let mut paths = HashSet::new();
        for step in &self.steps {
            if (self.scoped() || !step.id.is_empty()) && template_id(&step.id).is_err() {
                bail!(
                    "Each step of a compose of several needs an id of lowercase letters, digits and dashes, at most 64 and not starting with a dash: {:?}",
                    step.id
                );
            }
            if !ids.insert(step.id.as_str()) {
                bail!("Step ids must be unique: {}", step.id);
            }
            step.validate().map_err(|error| self.within(step, error))?;
            for parameter in step
                .parameters
                .iter()
                .filter(|p| p.volume_variable().is_some())
            {
                if !paths.insert(parameter.path.as_str()) {
                    bail!(
                        "Volume parameters need paths of their own: {} is used twice",
                        parameter.path
                    );
                }
                // Docker names a volume in at most 128 characters; container IDs take 32.
                let own = parameter.new_volume_for("", self.scope(step));
                if own.is_some_and(|name| name.len() + 32 > 128) {
                    bail!(
                        "{} would name the volumes it creates with more than 128 characters; shorten the step id {} or the parameter name",
                        self.key(step, parameter),
                        step.id
                    );
                }
            }
        }
        Ok(())
    }

    /// Whether a value given as `name` may go to a secret parameter: by the name the value goes
    /// by, or by the parameter's own name, which a step of several also takes alone.
    pub fn takes_secret(&self, name: &str) -> bool {
        self.entries()
            .any(|(_, parameter, key)| parameter.secret && (key == name || parameter.name == name))
    }

    /// The volumes a container with these values mounts: each volume parameter's volume at its
    /// path, for those given one.
    pub fn mounts(&self, values: &BTreeMap<String, String>) -> Vec<Mount> {
        self.entries()
            .filter_map(|(_, parameter, key)| parameter.mount(values.get(&key)?))
            .collect()
    }

    /// Every parameter's value for the new container `container`, by the name it goes by: the
    /// one given, otherwise the default, and for a volume parameter left empty that creates a
    /// volume, the one it gets.
    pub fn resolve(
        &self,
        given: &BTreeMap<String, String>,
        container: &str,
    ) -> anyhow::Result<BTreeMap<String, String>> {
        let given = self.named(given)?;
        let mut values = BTreeMap::new();
        for (step, parameter, key) in self.entries() {
            let value = parameter
                .resolve(given.get(&key).copied(), container, self.scope(step))
                .map_err(|error| self.within(step, error))?;
            values.insert(key, value);
        }
        Ok(values)
    }

    /// The values a request gives, by the names they go by. A parameter of a compose of several
    /// steps may also be named alone, when no other step asks for that name.
    fn named<'a>(
        &self,
        given: &'a BTreeMap<String, String>,
    ) -> anyhow::Result<BTreeMap<String, &'a String>> {
        let entries = self.entries().collect::<Vec<_>>();
        let mut named = BTreeMap::new();
        for (name, value) in given {
            let key = if entries.iter().any(|(_, _, key)| key == name) {
                name.clone()
            } else {
                let keys = entries
                    .iter()
                    .filter(|(_, parameter, _)| &parameter.name == name)
                    .map(|(_, _, key)| key.as_str())
                    .collect::<Vec<_>>();
                match keys.as_slice() {
                    [key] => key.to_string(),
                    [] => bail!("The image's compose has no parameter {name}"),
                    keys => bail!(
                        "Several steps of the image's compose ask for {name}; name it as {}",
                        keys.join(" or ")
                    ),
                }
            };
            if named.insert(key.clone(), value).is_some() {
                bail!("{key} was given twice");
            }
        }
        Ok(named)
    }
}

/// Check a compose's volumes against the container's own volume settings: a path the compose
/// mounts a volume at gets no second one, and a volume it mounts is mounted nowhere else, so
/// one folder of the desktop does not show up under two paths.
pub(crate) fn check_mounts(own: &[Mount], compose: &[Mount]) -> anyhow::Result<()> {
    for (index, mount) in compose.iter().enumerate() {
        if own.iter().any(|other| other.target == mount.target) {
            bail!(
                "The image's compose mounts the volume {} at {}; remove the volume setting with that path",
                mount.volume,
                mount.target
            );
        }
        if let Some(other) = own
            .iter()
            .chain(&compose[..index])
            .find(|other| other.volume == mount.volume)
        {
            bail!(
                "The volume {} would be mounted at both {} and {}; choose another volume for one of them",
                mount.volume,
                other.target,
                mount.target
            );
        }
    }
    Ok(())
}

/// Where a container's compose setup stands, as its record shows it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Setup {
    /// `pending`, `running`, `completed` or `failed`.
    pub status: String,
    /// The image record whose compose the container was created with.
    pub image: String,
    /// The values of the parameters that are not secret, by the names they go by.
    pub parameters: BTreeMap<String, String>,
    /// The names of the secret parameters.
    pub secrets: Vec<String>,
    #[serde(default)]
    pub finished_at: Option<u64>,
    /// The volumes the compose's volume parameters were given, mounted beside the container's
    /// own every time it is created.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<Mount>,
    /// The folders its volume and folder parameters name. Files can be uploaded into them, and
    /// the upload dialog offers them by the parameter's label.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub places: Vec<Place>,
    /// The steps of a compose of several, in order, and where each stands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<StepStatus>,
}

/// A folder of the desktop that a compose names, such as the project of an editor it installs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Place {
    /// The parameter's label, or its name without one, after its step's name when there are
    /// several steps.
    pub label: String,
    pub path: String,
}

/// One step of a compose of several, as the container record shows it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepStatus {
    pub id: String,
    pub name: String,
    /// `pending`, `running`, `completed` or `failed`.
    pub status: String,
}

/// The compose a container was created with and every value it was given, secret ones too.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Pinned {
    /// Saved as `template` before templates were called composes.
    #[serde(alias = "template")]
    pub compose: Compose,
    pub values: BTreeMap<String, String>,
}
impl Pinned {
    /// The values of the parameters that are not secret, by the names they go by.
    pub fn public_values(&self) -> BTreeMap<String, String> {
        let entries = self.compose.entries().collect::<Vec<_>>();
        self.values
            .iter()
            .filter(|(name, _)| {
                !entries
                    .iter()
                    .any(|(_, parameter, key)| key == *name && parameter.secret)
            })
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    }
    /// What the container record shows of it.
    pub fn setup(&self, image: &str) -> Setup {
        let compose = &self.compose;
        let entries = compose.entries().collect::<Vec<_>>();
        let secret = |name: &String| {
            entries
                .iter()
                .any(|(_, parameter, key)| key == name && parameter.secret)
        };
        Setup {
            status: "pending".into(),
            image: image.into(),
            parameters: self.public_values(),
            secrets: self
                .values
                .keys()
                .filter(|name| secret(name))
                .cloned()
                .collect(),
            finished_at: None,
            mounts: self.mounts(),
            places: entries
                .iter()
                .filter_map(|(step, parameter, key)| {
                    let mut place = parameter.place(self.values.get(key)?)?;
                    if compose.scoped() {
                        place.label = format!("{} · {}", step.title(), place.label);
                    }
                    Some(place)
                })
                .collect(),
            steps: if compose.scoped() {
                compose
                    .steps
                    .iter()
                    .map(|step| StepStatus {
                        id: step.id.clone(),
                        name: step.title().into(),
                        status: "pending".into(),
                    })
                    .collect()
            } else {
                Vec::new()
            },
        }
    }
    pub fn mounts(&self) -> Vec<Mount> {
        self.compose.mounts(&self.values)
    }
    /// The volume parameters, by the name their value goes by, with the folder each mounts at.
    pub fn volume_slots(&self) -> Vec<(String, String)> {
        self.compose
            .entries()
            .filter(|(_, parameter, _)| parameter.volume_variable().is_some())
            .map(|(_, parameter, key)| (key, parameter.path.clone()))
            .collect()
    }
    /// The volumes the container `container` gets of its own, as `(parameter, volume)`: those
    /// of the parameters that create one and were left empty, so their value names it.
    pub fn new_volumes(&self, container: &str) -> Vec<(String, &str)> {
        self.compose
            .entries()
            .filter_map(|(step, parameter, key)| {
                let value = self.values.get(&key)?;
                let own = parameter.new_volume_for(container, self.compose.scope(step));
                (own.as_ref() == Some(value)).then_some((key, value.as_str()))
            })
            .collect()
    }
    /// The paths of the volume parameters, which the desktop user is given before the first
    /// script runs, whether a volume was mounted there or not.
    pub fn paths(&self) -> Vec<&str> {
        self.compose
            .entries()
            .filter(|(_, parameter, _)| parameter.volume_variable().is_some())
            .map(|(_, parameter, _)| parameter.path.as_str())
            .collect()
    }
    /// The variables of step `index`'s script: its own parameters' values, a volume
    /// parameter's as its path and volume.
    pub fn environment(&self, index: usize) -> BTreeMap<String, String> {
        let step = &self.compose.steps[index];
        step.parameters
            .iter()
            .filter_map(|p| Some(p.environment(self.values.get(&self.compose.key(step, p))?)))
            .flatten()
            .collect()
    }
    /// The setup runner's request for step `index` on stdin, which keeps values out of command
    /// lines. The secrets of the other steps are masked too, in case this script prints
    /// something an earlier one saved. Bash reads the line trace before the script.
    pub fn request(&self, index: usize) -> Value {
        let step = &self.compose.steps[index];
        let secrets = step
            .parameters
            .iter()
            .filter(|p| p.secret)
            .map(|p| &p.name)
            .collect::<Vec<_>>();
        let mask = self
            .compose
            .steps
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .flat_map(|(_, other)| other.parameters.iter().map(move |p| (other, p)))
            .filter(|(_, p)| p.secret)
            .filter_map(|(other, p)| self.values.get(&self.compose.key(other, p)))
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        json!({
            "script": step.script.replace("\r\n", "\n"),
            "parameters": self.environment(index),
            "secrets": secrets,
            "mask": mask,
            "timeout_seconds": step.timeout_seconds,
            "trace": crate::script_progress::TRACE,
        })
    }
}

/// The image record whose compose applies to a new container from `requested`, a reference
/// or image ID that Docker resolved to `image_id`. Records of one image share the newest compose.
pub(crate) fn compose_image<'a>(
    records: &'a [ImageRecord],
    requested: &str,
    image_id: &str,
) -> Option<&'a ImageRecord> {
    records.iter().find(|record| {
        record.compose.is_some()
            && (record.image_id == image_id
                || record.image_id == requested
                || record.reference == requested)
    })
}

impl App {
    /// The compose and values the new container `container` from this image runs, or `None`
    /// without one.
    pub(crate) fn pin_compose(
        &self,
        container: &str,
        requested: &str,
        image_id: &str,
        given: &BTreeMap<String, String>,
    ) -> anyhow::Result<Option<(String, Pinned)>> {
        let records: Vec<ImageRecord> = self.db.list("image")?;
        let Some(record) = compose_image(&records, requested, image_id) else {
            if let Some(name) = given.keys().next() {
                bail!("The image has no compose, so it takes no parameter {name}");
            }
            return Ok(None);
        };
        let compose = record.compose.clone().context("Missing compose")?;
        let values = compose.resolve(given, container)?;
        Ok(Some((record.id.clone(), Pinned { compose, values })))
    }

    /// The compose a container was created with, which a snapshot of it keeps.
    pub(crate) fn pinned_compose(&self, container_id: &str) -> Option<Compose> {
        self.db
            .get::<Pinned>(PINNED, container_id)
            .ok()
            .map(|pinned| pinned.compose)
    }

    /// Every value a container's compose was given, secret ones too, by the names they go by.
    /// Empty without a compose.
    pub(crate) fn compose_values(&self, container_id: &str) -> BTreeMap<String, String> {
        self.db
            .get::<Pinned>(PINNED, container_id)
            .map(|pinned| pinned.values)
            .unwrap_or_default()
    }

    /// The values of a container's compose that are not secret, by the names they go by.
    pub(crate) fn public_compose_values(&self, container_id: &str) -> BTreeMap<String, String> {
        self.db
            .get::<Pinned>(PINNED, container_id)
            .map(|pinned| pinned.public_values())
            .unwrap_or_default()
    }

    pub(crate) fn save_pinned(&self, container_id: &str, pinned: &Pinned) -> anyhow::Result<()> {
        self.db.put(PINNED, container_id, pinned)
    }

    pub(crate) fn forget_pinned(&self, container_id: &str) -> anyhow::Result<()> {
        self.db.remove(PINNED, container_id)
    }

    /// Run the compose setup of a container whose setup has not completed. A failure is
    /// recorded on the container, which keeps running so the outcome can be inspected.
    pub(crate) async fn run_setup(&self, id: &str, job_id: &str) -> anyhow::Result<()> {
        let mut env: Container = self.db.get(CONTAINERS, id)?;
        if !setup_due(&env) {
            return Ok(());
        }
        let pinned: Pinned = self
            .db
            .get(PINNED, id)
            .context("The compose values of this container are missing; create it again")?;
        set_status(&mut env, "running", None);
        env.last_error = None;
        self.db.put(CONTAINERS, id, &env)?;
        let outcome = self.execute_setup(&env, &pinned, job_id).await;
        let mut env: Container = self.db.get(CONTAINERS, id)?;
        set_status(
            &mut env,
            if outcome.is_ok() {
                "completed"
            } else {
                "failed"
            },
            Some(now()),
        );
        env.last_error = outcome.as_ref().err().map(|e| format!("{e:#}"));
        self.db.put(CONTAINERS, id, &env)?;
        outcome
    }

    async fn execute_setup(
        &self,
        env: &Container,
        pinned: &Pinned,
        job_id: &str,
    ) -> anyhow::Result<()> {
        let container = self
            .desktop_ready(env, job_id)
            .await
            .context("The compose setup did not run")?;
        let paths = pinned.paths();
        if !paths.is_empty() {
            self.append_log(
                job_id,
                &format!("Preparing {} for the desktop user.", paths.join(", ")),
            )?;
            let mut args: Vec<String> = ["exec", "--user", "0:0", &container, "python3", "-c"]
                .map(str::to_owned)
                .to_vec();
            args.push(PATHS.into());
            args.extend(paths.iter().map(|path| path.to_string()));
            self.docker_logged(job_id, &args)
                .await
                .context("The folders of the compose's volume parameters could not be prepared")?;
        }
        let count = pinned.compose.steps.len();
        let mut progress = ScriptProgress::setup(
            pinned
                .compose
                .steps
                .iter()
                .map(|step| ScriptPart {
                    id: step.id.clone(),
                    name: step.title().into(),
                    script: step.script.replace("\r\n", "\n"),
                })
                .collect(),
        );
        for (index, step) in pinned.compose.steps.iter().enumerate() {
            let user = if step.root { "root" } else { "1001:1001" };
            let names = pinned.environment(index).into_keys().collect::<Vec<_>>();
            let which = if count > 1 {
                format!("step {} of {count}, {}, ", index + 1, step.title())
            } else {
                String::new()
            };
            self.append_log(
                job_id,
                &format!(
                    "Running the compose setup {which}as {user} (timeout: {} seconds). Parameters: {}.",
                    step.timeout_seconds,
                    if names.is_empty() {
                        "none".into()
                    } else {
                        names.join(", ")
                    }
                ),
            )?;
            self.set_step(&env.id, index, "running")?;
            progress.start_part(index, now());
            self.save_script(job_id, &mut progress)?;
            let mut args = crate::exec::desktop_exec(user, step.root);
            args.extend([
                container.clone(),
                "python3".into(),
                "-c".into(),
                RUNNER.into(),
            ]);
            let input = serde_json::to_vec(&pinned.request(index))?;
            // The runner stops the script at its timeout; Docker's own limit only catches a
            // runner that does not, so it stays at least the hour every logged command gets.
            let limit = (Duration::from_secs(step.timeout_seconds) + RUNNER_GRACE)
                .max(crate::docker::LOGGED_LIMIT);
            let outcome = self
                .docker_setup_logged(job_id, &args, input, limit, &mut progress)
                .await;
            if outcome.is_ok() {
                progress.finish_part(index, now());
                self.save_script(job_id, &mut progress)?;
            }
            self.set_step(
                &env.id,
                index,
                if outcome.is_ok() {
                    "completed"
                } else {
                    "failed"
                },
            )?;
            outcome.with_context(|| {
                if count > 1 {
                    format!("The compose step {} failed", step.title())
                } else {
                    "The compose setup failed".into()
                }
            })?;
        }
        progress.end();
        self.save_script(job_id, &mut progress)?;
        self.append_log(job_id, "Compose setup completed.")?;
        Ok(())
    }

    /// Record where step `index` of a container's compose stands, for a compose of several.
    fn set_step(&self, id: &str, index: usize, status: &str) -> anyhow::Result<()> {
        let mut env: Container = self.db.get(CONTAINERS, id)?;
        if let Some(step) = env
            .setup
            .as_mut()
            .and_then(|setup| setup.steps.get_mut(index))
        {
            step.status = status.into();
            self.db.put(CONTAINERS, id, &env)?;
        }
        Ok(())
    }
}

/// Whether a container's compose setup has yet to run or to finish, as after a retry or a reset.
pub(crate) fn setup_due(env: &Container) -> bool {
    env.setup
        .as_ref()
        .is_some_and(|setup| setup.status != "completed")
}

/// Record where the setup stands. A setup that starts or waits to start again runs every
/// step again, and one that failed leaves the step it stopped in failed.
fn set_status(env: &mut Container, status: &str, finished_at: Option<u64>) {
    if let Some(setup) = env.setup.as_mut() {
        setup.status = status.into();
        setup.finished_at = finished_at;
        for step in &mut setup.steps {
            match status {
                "pending" | "running" => step.status = "pending".into(),
                "failed" if step.status == "running" => step.status = "failed".into(),
                _ => {}
            }
        }
    }
}

/// A new container made from the source image lacks what the setup changed, so it runs again.
pub(crate) fn setup_again(env: &mut Container) {
    set_status(env, "pending", None);
}

/// No job survives a restart, so a setup still waiting or running then will not finish on its
/// own; retry runs it again.
pub(crate) fn mark_interrupted(db: &crate::db::Db) -> anyhow::Result<()> {
    for mut env in db.list::<Container>(CONTAINERS)? {
        if env
            .setup
            .as_ref()
            .is_some_and(|setup| setup.status == "pending" || setup.status == "running")
        {
            set_status(&mut env, "failed", Some(now()));
            // An error that stopped the setup earlier explains more than the restart does.
            env.last_error.get_or_insert_with(|| {
                "The manager restarted before the compose setup finished; retry to run it again"
                    .into()
            });
            db.put(CONTAINERS, &env.id, &env)?;
        }
    }
    Ok(())
}

pub async fn get(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<Json<Compose>> {
    let record: ImageRecord = app.db.get("image", &id)?;
    record
        .compose
        .map(Json)
        .ok_or_else(|| Error(StatusCode::NOT_FOUND, "This image has no compose".into()))
}

pub async fn set(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(compose): Json<Compose>,
) -> Result<Json<Value>> {
    compose.validate()?;
    replace(&app, &id, Some(compose)).await
}

pub async fn remove(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<Json<Value>> {
    replace(&app, &id, None).await
}

async fn replace(app: &App, id: &str, compose: Option<Compose>) -> Result<Json<Value>> {
    let _guard = app.hold(&format!("image:{id}")).await?;
    let mut record: ImageRecord = app.db.get("image", id)?;
    record.compose = compose;
    app.db.put("image", id, &record)?;
    Ok(Json(crate::api::image_summary(&record)))
}

#[cfg(test)]
#[path = "compose_tests.rs"]
mod tests;
