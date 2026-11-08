//! The skills as the MCP Skills extension (SEP-2640) serves them: each `SKILL.md` is a resource at
//! `skill://<name>/SKILL.md`, and `skills/list` and `skills/get` give a skill's frontmatter, size
//! and digest. Few clients know the extension yet, while most let the model read a resource, so
//! the server's instructions also name each skill's URI and when to read it, the pointer the
//! extension allows.
use super::{MARKDOWN, SKILLS, Server, Skill};
use rmcp::{
    ErrorData,
    model::{
        CustomRequest, CustomResult, ErrorCode, ListResourcesResult, ReadResourceResult, Resource,
        ResourceContents, ResourcesCapability, ServerCapabilities,
    },
};
use serde_json::{Map, Value, json};

/// The extension's name in a server's capabilities.
pub(crate) const EXTENSION: &str = "io.modelcontextprotocol/skills";

/// The skills `server` offers.
pub(crate) fn offered(server: Server) -> Vec<&'static Skill> {
    SKILLS
        .iter()
        .filter(|skill| skill.servers.contains(&server))
        .collect()
}

/// The URI of a skill's `SKILL.md`.
pub(crate) fn uri(skill: &Skill) -> String {
    format!("skill://{}/SKILL.md", skill.name)
}

/// What a server that offers `skills` declares beside its tools: resources, which carry them, and
/// the extension, without its optional directory listing. A server that offers none declares
/// neither.
pub(crate) fn declare(capabilities: &mut ServerCapabilities, skills: &[&Skill]) {
    if skills.is_empty() {
        return;
    }
    capabilities.resources = Some(ResourcesCapability::default());
    capabilities
        .extensions
        .get_or_insert_default()
        .insert(EXTENSION.to_owned(), Map::new());
}

/// The sentences a server's instructions add for `skills`: which resource to read, and before
/// what.
pub(crate) fn instructions(skills: &[&Skill]) -> String {
    skills
        .iter()
        .map(|skill| {
            format!(
                " Read the skill {name} before you {before}: it is this server's MCP resource {uri}, which your client's tool for reading MCP resources, or resources/read, returns.",
                name = skill.name,
                before = skill.before,
                uri = uri(skill),
            )
        })
        .collect()
}

/// `resources/list`: every `SKILL.md` the server offers.
pub(crate) fn resources(skills: &[&Skill]) -> ListResourcesResult {
    ListResourcesResult::with_all_items(
        skills
            .iter()
            .map(|skill| {
                Resource::new(uri(skill), skill.name)
                    .with_description(skill.description.clone())
                    .with_mime_type(MARKDOWN)
                    .with_size(skill.size())
            })
            .collect(),
    )
}

/// The offered skill whose `SKILL.md` is at `uri`, or the error SEP-2640 asks for, `-32602`.
fn named<'a>(skills: &[&'a Skill], uri: &str) -> Result<&'a Skill, ErrorData> {
    skills
        .iter()
        .copied()
        .find(|skill| self::uri(skill) == uri)
        .ok_or_else(|| ErrorData::invalid_params(format!("No skill is served at {uri}"), None))
}

/// `resources/read` of a skill's `SKILL.md`.
pub(crate) fn read(skills: &[&Skill], uri: &str) -> Result<ReadResourceResult, ErrorData> {
    let skill = named(skills, uri)?;
    Ok(ReadResourceResult::new(vec![
        ResourceContents::text(skill.content, uri).with_mime_type(MARKDOWN),
    ]))
}

/// A skill as `skills/list` and `skills/get` give it: its frontmatter and its one file, with
/// that file's digest and size.
fn entry(skill: &Skill) -> Value {
    let uri = uri(skill);
    json!({
        "uri": uri,
        "frontmatter": skill.frontmatter,
        "resources": [{"uri": uri, "digest": skill.digest, "size": skill.size()}],
    })
}

/// A request rmcp does not know: `skills/list` and `skills/get` on a server that offers skills,
/// and method-not-found for anything else, as without the extension.
pub(crate) fn request(
    skills: &[&Skill],
    request: CustomRequest,
) -> Result<CustomResult, ErrorData> {
    let missing = || ErrorData::new(ErrorCode::METHOD_NOT_FOUND, request.method.clone(), None);
    if skills.is_empty() {
        return Err(missing());
    }
    match request.method.as_str() {
        "skills/list" => Ok(CustomResult::new(json!({
            "resultType": "complete",
            "skills": skills.iter().map(|skill| entry(skill)).collect::<Vec<_>>(),
        }))),
        "skills/get" => {
            let uri = request
                .params
                .as_ref()
                .and_then(|params| params.get("uri"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ErrorData::invalid_params(
                        "skills/get takes the uri of a skill's SKILL.md",
                        None,
                    )
                })?;
            Ok(CustomResult::new(json!({
                "resultType": "complete",
                "skill": entry(named(skills, uri)?),
            })))
        }
        _ => Err(missing()),
    }
}
