//! An image's routes: the list of catalog and saved images with what lists show of them, a catalog
//! image's download or source build, an import, a script build with its configuration, and a
//! snapshot of a container's files.
use super::default_true;
use crate::{
    App,
    error::{Error, Result, missing},
    jobs::{launch_retryable, retry},
    model::*,
    script_progress::{ScriptPart, ScriptProgress, TRACE, TRACE_NAME},
};
use anyhow::{Context, bail};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

/// Build a catalog image from the bundled Docker context instead of downloading it.
pub async fn build_catalog(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let image = crate::catalog::find(&id).ok_or_else(missing)?;
    let resource = format!("image:{}", image.id);
    let request = retry::Request::SourceBuild {
        image_id: image.id.clone(),
    };
    launch_retryable(
        app,
        "source-build",
        resource,
        None,
        request,
        move |app, job_id| async move {
            app.append_log(
                &job_id,
                &format!(
                    "Building {}. Package installation requires internet access.",
                    image.name
                ),
            )?;
            app.docker_logged(
                &job_id,
                &[
                    "build".into(),
                    "--progress=plain".into(),
                    "--target".into(),
                    image.build_target.clone(),
                    "-t".into(),
                    image.reference.clone(),
                    app.config.desktop_context.to_string_lossy().into_owned(),
                ],
            )
            .await?;
            let info = app.inspect("image", &image.reference).await?;
            Ok(
                json!({"id": image.id, "kind": "catalog", "reference": image.reference, "image_id": info["Id"]}),
            )
        },
    )
}
/// Download a catalog image, or a newer release of it.
pub async fn pull_catalog(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let image = crate::catalog::find(&id).ok_or_else(missing)?;
    let resource = format!("image:{}", image.id);
    let request = retry::Request::ImagePull {
        image_id: image.id.clone(),
    };
    launch_retryable(
        app,
        "image-pull",
        resource,
        None,
        request,
        move |app, job_id| async move { app.pull_catalog_image(&image, &job_id).await },
    )
}
/// Every image: the catalog first, then the saved images, each with its sizes.
pub async fn images(State(app): State<Arc<App>>) -> Result<Json<Value>> {
    let mut list = image_list(&app).await?;
    crate::layers::add_sizes(&app, &mut list).await;
    Ok(Json(json!(list)))
}
/// Every image as lists show it, before its sizes are added.
pub(crate) async fn image_list(app: &Arc<App>) -> Result<Vec<Value>> {
    let records = app.db.list::<ImageRecord>("image")?;
    let listed = app
        .docker(&["image", "ls", "-a", "--no-trunc", "--format", "{{.ID}}"])
        .await;
    let ids: std::collections::HashSet<String> = listed
        .as_ref()
        .map(|bytes| {
            String::from_utf8_lossy(bytes)
                .lines()
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let catalog =
        futures_util::future::join_all(crate::catalog::catalog().into_iter().map(|image| {
            let app = app.clone();
            let docker_listed = listed.is_ok();
            async move {
                let inspected = app.inspect("image", &image.reference).await.ok();
                crate::catalog::listed(&image, inspected.as_ref(), docker_listed)
            }
        }))
        .await;
    let saved = records.iter().map(|r| {
        let available = listed.as_ref().ok().map(|_| ids.contains(&r.image_id));
        let mut value = image_summary(r);
        value["available"] = json!(available);
        value["mcp_servers"] = json!(declared_servers(&records, r));
        if crate::compose::inherit::base_compose_missing(&records, r) {
            value["base_compose_missing"] = json!(true);
        }
        value
    });
    Ok(catalog
        .into_iter()
        .map(|mut value| {
            value["mcp_servers"] = json!([]);
            value
        })
        .chain(saved)
        .collect())
}
/// The desktop MCP servers an image's builds declared: its own build's and those of the images
/// it was built or snapshotted from, whose layers it carries.
pub(crate) fn declared_servers(records: &[ImageRecord], record: &ImageRecord) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut current = Some(record);
    for _ in 0..32 {
        let Some(image) = current else { break };
        for declaration in image.build.iter().flat_map(|build| &build.mcp) {
            if !names.contains(&declaration.name) {
                names.push(declaration.name.clone());
            }
        }
        current = image.parent_image_id.as_ref().and_then(|parent| {
            records
                .iter()
                .find(|other| other.image_id == *parent && other.id != image.id)
        });
    }
    names.sort();
    names
}
/// An image record as lists and job results show it. A build or setup script can be up to a
/// megabyte, so only the configuration and compose routes return them; an icon is named by its
/// version, and only the icon route returns the picture.
pub(crate) fn image_summary(record: &ImageRecord) -> Value {
    let mut value = json!(record);
    for (key, list) in [("build", "parts"), ("compose", "steps")] {
        if let Some(part) = value.get_mut(key).and_then(Value::as_object_mut) {
            part.remove("script");
            // The declarations may carry header values; lists name the servers instead.
            part.remove("mcp");
            for item in part
                .get_mut(list)
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
                .filter_map(Value::as_object_mut)
            {
                item.remove("script");
            }
        }
    }
    if let Some(icon) = &record.icon {
        value["icon"] = json!(icon.version());
    }
    value
}
/// The request that built an image, ready to build it again here or on another installation.
pub async fn image_config(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<Json<BuildImage>> {
    let record: ImageRecord = app.db.get("image", &id)?;
    let build = record.build.ok_or_else(|| {
        Error(
            StatusCode::NOT_FOUND,
            "No build configuration was recorded for this image".into(),
        )
    })?;
    // One template is named as builds named it before templates could be merged.
    let (template_id, template_ids) = match record.template_ids.as_slice() {
        [one] => (Some(one.clone()), Vec::new()),
        _ => (None, record.template_ids),
    };
    Ok(Json(BuildImage {
        name: record.name,
        description: record.description,
        base_image: build.base_image,
        internet: build.internet,
        script: build.script,
        parts: build.parts,
        mcp: build.mcp,
        compose: record.compose,
        inherit_compose: build.inherit_compose,
        icon: record.icon,
        template_id,
        template_ids,
    }))
}
#[derive(Deserialize, schemars::JsonSchema)]
pub struct RegisterImage {
    pub reference: String,
    pub name: String,
}
pub async fn register_image(
    State(app): State<Arc<App>>,
    Json(body): Json<RegisterImage>,
) -> Result<Json<Value>> {
    image_ref(&body.reference)?;
    if body.name.trim().is_empty() || body.name.chars().count() > 100 {
        return Err(anyhow::anyhow!("Image name must be 1 to 100 characters").into());
    }
    let _guard = app.operations.lock().await;
    let info = app.inspect("image", &body.reference).await?;
    if info["Config"]["Labels"]["thoric.desktop"] != "true" {
        return Err(anyhow::anyhow!("A desktop-compatible image is required").into());
    }
    crate::catalog::check_level(&info["Config"]["Labels"])?;
    let rec = ImageRecord {
        id: uuid::Uuid::new_v4().simple().to_string(),
        name: body.name,
        image_id: info["Id"].as_str().context("Missing image ID")?.into(),
        reference: body.reference,
        parent_image_id: None,
        source_container: None,
        kind: "import".into(),
        created_at: now(),
        description: String::new(),
        desktop: crate::catalog::from_image(&info),
        build: None,
        compose: None,
        icon: None,
        template_ids: Vec::new(),
    };
    app.db.put("image", &rec.id, &rec)?;
    Ok(Json(json!(rec)))
}
/// A build request. `GET /api/images/{id}/config` returns the same shape, so an exported
/// configuration is a request that builds the image again, and a template on the templates page
/// is one too.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BuildImage {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub base_image: String,
    #[serde(default = "default_true")]
    pub internet: bool,
    /// The script of a build of one part, run as root on the base.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub script: String,
    /// Instead of a script, the scripts of templates merged into one build, in the order they
    /// run, each in a layer of its own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<BuildPart>,
    /// The compose the built image gets. Configurations exported before templates were called
    /// composes name it `template`.
    #[serde(default, alias = "template", skip_serializing_if = "Option::is_none")]
    pub compose: Option<crate::compose::Compose>,
    /// Keep the compose of a saved base image before this one (`compose::inherit`).
    #[serde(
        default = "default_true",
        skip_serializing_if = "crate::model::is_true"
    )]
    pub inherit_compose: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<crate::icons::Icon>,
    /// Set by the templates page, so it can tell which of its templates are installed: one
    /// template here, several merged into one build in `template_ids`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub template_ids: Vec<String>,
    /// MCP servers the image declares for its desktops, written to `/etc/thoric/mcp.d`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp: Vec<crate::desktop_mcp::Declaration>,
}
impl BuildImage {
    fn validate(&self) -> anyhow::Result<()> {
        image_ref(&self.base_image)?;
        if self.name.trim().is_empty() || self.name.chars().count() > 100 {
            anyhow::bail!("Invalid image name or script");
        }
        build_scripts(&self.script, &self.parts)?;
        crate::desktop_mcp::check_declarations(&self.mcp)?;
        if self.description.len() > 4096 {
            anyhow::bail!("An image description must be at most 4096 bytes");
        }
        if let Some(compose) = &self.compose {
            compose.validate()?;
        }
        if let Some(icon) = &self.icon {
            icon.validate()?;
        }
        let templates = self.templates();
        if templates.len() > BUILD_PART_LIMIT {
            anyhow::bail!("An image is built from at most {BUILD_PART_LIMIT} templates");
        }
        for id in &templates {
            template_id(id)?;
        }
        Ok(())
    }

    /// The templates the image is built from, each once.
    pub(crate) fn templates(&self) -> Vec<String> {
        let mut ids = Vec::new();
        for id in self.template_id.iter().chain(&self.template_ids) {
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        ids
    }
}
pub async fn build(
    State(app): State<Arc<App>>,
    Json(body): Json<BuildImage>,
) -> Result<Json<Value>> {
    body.validate()?;
    let name = body.name.clone();
    launch_retryable(
        app,
        "build",
        format!("build:{}", uuid::Uuid::new_v4()),
        Some(name),
        retry::Request::Build(body.clone()),
        move |app, id| async move {
            app.plan_stages(&id, &crate::stages::BUILD_STAGES)?;
            app.begin_stage(&id, "base")?;
            app.ensure_catalog_image(&body.base_image, &id).await?;
            let base = app.inspect("image", &body.base_image).await?;
            if base["Config"]["Labels"]["thoric.desktop"] != "true" {
                anyhow::bail!("Base image must be desktop-compatible");
            }
            crate::catalog::check_level(&base["Config"]["Labels"])?;
            let parent = base["Id"]
                .as_str()
                .context("Missing base image ID")?
                .to_owned();
            let (compose, template_ids) =
                crate::compose::inherit::built_on(&app, &id, &body, &parent)?;
            let base_tag = format!("thoric/build-base:{id}");
            app.docker(&["image", "tag", &parent, &base_tag]).await?;
            let dir = app.config.data.join("builds").join(&id);
            tokio::fs::create_dir_all(&dir).await?;
            let script = body.script.replace("\r\n", "\n");
            let parts = body
                .parts
                .iter()
                .map(|part| BuildPart {
                    script: part.script.replace("\r\n", "\n"),
                    ..part.clone()
                })
                .collect::<Vec<_>>();
            // Each part runs in a step of its own, named after it in the build's output, so a
            // failure says whose script it was and Docker's cache keeps the parts before it.
            let files = if parts.is_empty() {
                vec![ScriptPart {
                    id: "setup".into(),
                    name: body.name.clone(),
                    script: script.clone(),
                }]
            } else {
                parts
                    .iter()
                    .map(|part| ScriptPart {
                        id: part.id.clone(),
                        name: part.name.clone(),
                        script: part.script.clone(),
                    })
                    .collect()
            };
            // Bash reads the line trace first; a bind mount leaves nothing of it in the image.
            tokio::fs::write(dir.join(TRACE_NAME), TRACE).await?;
            let mut dockerfile = format!("FROM {base_tag}\nUSER root\n");
            for ScriptPart {
                id: name, script, ..
            } in &files
            {
                tokio::fs::write(dir.join(format!("{name}.sh")), script).await?;
                dockerfile.push_str(&format!(
                    "COPY {name}.sh /tmp/thoric-{name}.sh\nRUN --mount=type=bind,source={TRACE_NAME},target=/tmp/{TRACE_NAME} BASH_ENV=/tmp/{TRACE_NAME} bash /tmp/thoric-{name}.sh && rm /tmp/thoric-{name}.sh\n"
                ));
            }
            let mut progress = ScriptProgress::build(files);
            // After the scripts, so the declarations the build names are the ones that hold.
            if !body.mcp.is_empty() {
                dockerfile
                    .push_str(&crate::desktop_mcp::write_declarations(&dir, &body.mcp).await?);
            }
            dockerfile.push_str(&format!("LABEL thoric.desktop=true\nLABEL {LABEL}=true\n"));
            tokio::fs::write(dir.join("Dockerfile"), dockerfile).await?;
            let reference = format!("thoric/custom:{id}");
            let args = vec![
                "build".into(),
                "--progress=plain".into(),
                "--network".into(),
                if body.internet {
                    "default".into()
                } else {
                    "none".into()
                },
                "-t".into(),
                reference.clone(),
                dir.to_string_lossy().into_owned(),
            ];
            app.begin_stage(&id, "script")?;
            app.append_log(
                &id,
                &if parts.is_empty() {
                    "Running setup script as root. Mounted volumes are excluded from the image."
                        .into()
                } else {
                    format!(
                        "Running the scripts of {} as root, in this order, each in a layer of its own. Mounted volumes are excluded from the image.",
                        parts
                            .iter()
                            .map(|part| part.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                },
            )?;
            app.docker_build_logged(&id, &args, &mut progress).await?;
            // Without BuildKit's markers, the layer stage is skipped rather than guessed.
            app.begin_stage(&id, "image")?;
            let info = app.inspect("image", &reference).await?;
            let rec = ImageRecord {
                id: id.clone(),
                name: body.name,
                image_id: info["Id"].as_str().context("Missing image ID")?.into(),
                reference,
                parent_image_id: Some(parent),
                source_container: None,
                kind: "build".into(),
                created_at: now(),
                description: body.description,
                desktop: crate::catalog::from_image(&info),
                build: Some(BuildConfig {
                    base_image: body.base_image,
                    internet: body.internet,
                    script,
                    parts,
                    mcp: body.mcp,
                    inherit_compose: body.inherit_compose,
                }),
                compose,
                icon: body.icon,
                template_ids,
            };
            app.db.put("image", &id, &rec)?;
            Ok(image_summary(&rec))
        },
    )
}
#[derive(Deserialize, schemars::JsonSchema)]
pub struct Snapshot {
    pub name: String,
    #[serde(default)]
    pub description: String,
}
pub async fn snapshot(
    State(app): State<Arc<App>>,
    Path(env_id): Path<String>,
    Json(body): Json<Snapshot>,
) -> Result<Json<Value>> {
    app.container(&env_id)?;
    if body.name.trim().is_empty()
        || body.name.chars().count() > 100
        || body.description.len() > 4096
    {
        return Err(anyhow::anyhow!("Invalid snapshot name or description").into());
    }
    let name = body.name.clone();
    let request = retry::Request::Snapshot {
        container_id: env_id.clone(),
        name: body.name.clone(),
        description: body.description.clone(),
    };
    launch_retryable(
        app,
        "snapshot",
        format!("container:{env_id}"),
        Some(name),
        request,
        move |app, id| async move {
            app.plan_stages(&id, &crate::stages::SNAPSHOT_STAGES)?;
            app.begin_stage(&id, "check")?;
            let env = app.container(&env_id).map_err(|e| anyhow::anyhow!(e.1))?;
            let container = app.owned(&env).await?;
            let rec = app
                .save_snapshot(&id, &env, &container, body.name, body.description)
                .await?;
            Ok(image_summary(&rec))
        },
    )
}
impl App {
    /// Commit the files of `env`, whose Docker container `container` is, to the image
    /// `thoric/snapshot:<job>` and save its record under the job's ID, in the job's `commit` and
    /// `record` stages. A snapshot and a duplicate take theirs this way.
    pub(crate) async fn save_snapshot(
        &self,
        job_id: &str,
        env: &Container,
        container: &Value,
        name: String,
        description: String,
    ) -> anyhow::Result<ImageRecord> {
        let reference = format!("thoric/snapshot:{job_id}");
        self.begin_stage(job_id, "commit")?;
        self.append_log(job_id,"Committing container changes. Volumes, /run, /tmp and running application state are excluded.")?;
        self.docker_logged(
            job_id,
            &[
                "commit".into(),
                "--change".into(),
                format!("LABEL {LABEL}=true"),
                // The image's history says what this layer holds.
                "--message".into(),
                format!("Thoric snapshot of {}", env.spec.name),
                container["Id"]
                    .as_str()
                    .context("Missing container ID")?
                    .into(),
                reference.clone(),
            ],
        )
        .await?;
        self.begin_stage(job_id, "record")?;
        let info = self.inspect("image", &reference).await?;
        // A container from the snapshot asks for the same parameters again.
        let compose = self.pinned_compose(&env.id);
        let records: Vec<ImageRecord> = self.db.list("image")?;
        let icon = crate::icons::inherited(&records, &env.source_image_id);
        let rec = ImageRecord {
            id: job_id.into(),
            name,
            image_id: info["Id"].as_str().context("Missing image ID")?.into(),
            reference,
            parent_image_id: Some(env.source_image_id.clone()),
            source_container: Some(env.id.clone()),
            kind: "snapshot".into(),
            created_at: now(),
            description,
            desktop: crate::catalog::from_image(&info),
            build: None,
            compose,
            icon,
            template_ids: Vec::new(),
        };
        self.db.put("image", job_id, &rec)?;
        Ok(rec)
    }
    pub async fn cleanup_build(&self, id: &str) -> anyhow::Result<()> {
        if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("Invalid build ID");
        }
        let dir = self.config.data.join("builds").join(id);
        match tokio::fs::remove_dir_all(&dir).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        self.drop_tag(&format!("thoric/build-base:{id}")).await?;
        // The build tags its image before it saves the record, so one that failed, was cancelled
        // or was cut short by a restart in between leaves a tag that no record knows.
        if !self.db.contains("image", id)? {
            self.drop_tag(&format!("thoric/custom:{id}")).await?;
        }
        Ok(())
    }
    /// Remove the tag `tag` when Docker has it.
    async fn drop_tag(&self, tag: &str) -> anyhow::Result<()> {
        let names = self
            .docker(&["image", "ls", "--format", "{{.Repository}}:{{.Tag}}", tag])
            .await?;
        if String::from_utf8_lossy(&names)
            .lines()
            .any(|name| name == tag)
        {
            self.docker(&["image", "rm", tag]).await?;
        }
        Ok(())
    }
}
#[cfg(test)]
#[path = "images_tests.rs"]
mod tests;
