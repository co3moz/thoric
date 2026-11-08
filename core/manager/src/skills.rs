//! The skills Thoric ships for the agents that use it, `docs/skills/<name>/SKILL.md`, built into
//! the manager so the copy it serves always matches the tools it offers. The web serves them as
//! the Agent Skills discovery index at `/.well-known/agent-skills/`, without a session since every
//! file is public in the repository, so one command copies a skill into an agent's skills folder;
//! `/api/skills` tells the dashboard where MCP offers each; and `mcp` serves them as the MCP
//! Skills extension (SEP-2640) of `/mcp` and of a scale set's endpoints.
pub(crate) mod mcp;

use crate::error;
use axum::{
    Json,
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

/// The schema the discovery index names, version 0.2.0 of the Agent Skills discovery RFC.
const DISCOVERY_SCHEMA: &str = "https://schemas.agentskills.io/discovery/0.2.0/schema.json";

/// Where the web serves the skills, the folder the discovery RFC names.
pub(crate) const WELL_KNOWN: &str = "/.well-known/agent-skills";

/// The media type of a `SKILL.md`.
pub(crate) const MARKDOWN: &str = "text/markdown";

/// Which of the manager's MCP servers offers a skill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Server {
    /// The administrator's `/mcp`.
    Root,
    /// The endpoint of a scale set.
    Set,
}

/// A skill as the repository holds it, and where the manager's MCP servers offer it.
struct Source {
    /// Its folder under `docs/skills`, which is its name.
    folder: &'static str,
    content: &'static str,
    servers: &'static [Server],
    /// What an agent is about to do when it should read the skill, which the instructions of
    /// the servers that offer it say: "before you ...".
    before: &'static str,
}

/// Includes `docs/skills/<folder>/SKILL.md` under its folder's name.
macro_rules! source {
    ($folder:literal, $servers:expr, $before:literal) => {
        Source {
            folder: $folder,
            content: include_str!(concat!("../../../docs/skills/", $folder, "/SKILL.md")),
            servers: $servers,
            before: $before,
        }
    };
}

/// Every skill of `docs/skills`, which `skills_tests` holds to the folders there.
const SOURCES: &[Source] = &[source!(
    "thoric-scale-sets",
    &[Server::Root, Server::Set],
    "create instances, hand one to a subagent or let several instances write a folder they share"
)];

/// A skill, read and checked once.
pub(crate) struct Skill {
    pub(crate) name: &'static str,
    pub(crate) description: String,
    /// Its YAML frontmatter as JSON, every field the file has.
    pub(crate) frontmatter: Map<String, Value>,
    pub(crate) content: &'static str,
    /// `sha256:` and the hex of the file's bytes, as the discovery RFC and SEP-2640 write it.
    pub(crate) digest: String,
    pub(crate) servers: &'static [Server],
    pub(crate) before: &'static str,
}

/// The skills, in the order of `SOURCES`. A skill that does not read fails `skills_tests`, so
/// none is ever left out here.
pub(crate) static SKILLS: LazyLock<Vec<Skill>> = LazyLock::new(|| {
    SOURCES
        .iter()
        .filter_map(|source| match Skill::read(source) {
            Ok(skill) => Some(skill),
            Err(error) => {
                tracing::error!(skill = source.folder, %error, "a built-in skill does not read");
                None
            }
        })
        .collect()
});

impl Skill {
    fn read(source: &Source) -> Result<Self, String> {
        let frontmatter = frontmatter(source.content)?;
        let text = |key: &str| frontmatter.get(key).and_then(Value::as_str);
        let name = text("name").ok_or("the frontmatter has no name")?;
        if name != source.folder {
            return Err(format!(
                "its name is {name}, not its folder's {}",
                source.folder
            ));
        }
        let valid = (1..=64).contains(&name.len())
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !name.starts_with('-')
            && !name.ends_with('-')
            && !name.contains("--");
        if !valid {
            return Err(format!(
                "{name} is not a skill name: 1 to 64 lowercase letters, digits and single hyphens"
            ));
        }
        let description = text("description")
            .ok_or("the frontmatter has no description")?
            .to_owned();
        if description.chars().count() > 1024 {
            return Err("the description is longer than 1024 characters".into());
        }
        Ok(Self {
            name: source.folder,
            description,
            frontmatter,
            content: source.content,
            digest: digest(source.content.as_bytes()),
            servers: source.servers,
            before: source.before,
        })
    }

    /// Where the web serves its `SKILL.md`, from the root of the manager's address.
    pub(crate) fn url(&self) -> String {
        format!("{WELL_KNOWN}/{}/SKILL.md", self.name)
    }

    /// The length of its `SKILL.md` in bytes.
    pub(crate) fn size(&self) -> u64 {
        self.content.len() as u64
    }
}

/// `sha256:` and the lowercase hex of the bytes' SHA-256.
fn digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    let hex = hash
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hex}")
}

/// The YAML frontmatter of a `SKILL.md` as JSON. Thoric's skills keep to `key: value` lines whose
/// value YAML reads as the same text, and anything else is refused, so what `skills/list` sends
/// as the frontmatter is exactly what a client parses from the file.
fn frontmatter(content: &str) -> Result<Map<String, Value>, String> {
    let mut lines = content.lines();
    if lines.next() != Some("---") {
        return Err("SKILL.md does not begin with its frontmatter, ---".into());
    }
    let mut fields = Map::new();
    for line in lines {
        if line == "---" {
            return Ok(fields);
        }
        let (key, value) = line
            .split_once(": ")
            .ok_or_else(|| format!("{line:?} is not a key: value line"))?;
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
            return Err(format!("{key:?} is not a frontmatter key"));
        }
        // A plain scalar: nothing YAML reads as a quote, a collection, a block, a reference, a
        // comment or another mapping, and no word it reads as a value other than text.
        let typed = matches!(
            value.to_ascii_lowercase().as_str(),
            "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~"
        ) || value.parse::<f64>().is_ok();
        let plain = !value.is_empty()
            && value.trim() == value
            && !value.starts_with([
                '"', '\'', '[', ']', '{', '}', '>', '|', '&', '*', '!', '%', '@', '`', '#', '-',
                '?', ':', ',',
            ])
            && !value.contains(": ")
            && !value.contains(" #")
            && !value.ends_with(':')
            && !typed;
        if !plain {
            return Err(format!(
                "the value of {key} is not a plain YAML scalar that reads as itself"
            ));
        }
        if fields
            .insert(key.to_owned(), Value::String(value.to_owned()))
            .is_some()
        {
            return Err(format!("{key} is in the frontmatter twice"));
        }
    }
    Err("the frontmatter does not end with ---".into())
}

/// The skill named `name`, when the manager has it.
pub(crate) fn find(name: &str) -> Option<&'static Skill> {
    SKILLS.iter().find(|skill| skill.name == name)
}

/// `GET /.well-known/agent-skills/index.json`: every skill as the Agent Skills discovery RFC lists
/// it, for a client that installs skills from an address.
pub async fn index() -> Json<Value> {
    Json(json!({
        "$schema": DISCOVERY_SCHEMA,
        "skills": SKILLS
            .iter()
            .map(|skill| json!({
                "name": skill.name,
                "type": "skill-md",
                "description": skill.description,
                "url": skill.url(),
                "digest": skill.digest,
            }))
            .collect::<Vec<_>>(),
    }))
}

/// `GET /.well-known/agent-skills/{name}/SKILL.md`: one skill's file as it is written.
pub async fn file(Path(name): Path<String>) -> error::Result<Response> {
    let skill = find(&name).ok_or_else(|| {
        error::Error(
            StatusCode::NOT_FOUND,
            format!("Thoric has no skill named {name}"),
        )
    })?;
    Ok((
        [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
        skill.content,
    )
        .into_response())
}

/// A skill as the dashboard lists it.
#[derive(Serialize, JsonSchema)]
pub(crate) struct Listed {
    /// Its name, which is also its folder's in a skills folder.
    name: &'static str,
    /// When an agent uses it, from its frontmatter.
    description: String,
    /// Where the web serves its `SKILL.md`, without a session.
    url: String,
    /// `sha256:` and the hex of the file's SHA-256.
    digest: String,
    /// The file's length in bytes.
    size: u64,
    /// The MCP servers that offer it: `root` for `/mcp`, `set` for a scale set's endpoints.
    servers: Vec<Server>,
}

/// `GET /api/skills`: every skill, where the web serves it and which MCP servers offer it.
pub async fn list() -> Json<Vec<Listed>> {
    Json(
        SKILLS
            .iter()
            .map(|skill| Listed {
                name: skill.name,
                description: skill.description.clone(),
                url: skill.url(),
                digest: skill.digest.clone(),
                size: skill.size(),
                servers: skill.servers.to_vec(),
            })
            .collect(),
    )
}

#[cfg(test)]
#[path = "skills_tests.rs"]
mod tests;
