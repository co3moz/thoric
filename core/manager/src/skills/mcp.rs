//! The skills as the MCP Skills extension (SEP-2640) serves them: each `SKILL.md` is a resource at
//! `skill://<name>/SKILL.md`, and `skills/list` and `skills/get` give a skill's frontmatter, size
//! and digest. rmcp routes those two to a server's `list_skills` and `get_skill` once its
//! capabilities declare the extension, and checks every entry against the extension's rules
//! before it answers. Few clients know the extension yet, while most let the model read a
//! resource, so the server's instructions also name each skill's URI and when to read it, the
//! pointer the extension allows.
use super::{MARKDOWN, SKILLS, Server, Skill};
use rmcp::{
    ErrorData,
    model::{
        CustomRequest, ErrorCode, GetSkillResult, ListResourcesResult, ListSkillsResult,
        PaginatedRequestParams, ReadResourceResult, Resource, ResourceContents,
        ResourcesCapability, SKILLS_EXTENSION_ID, ServerCapabilities, Skill as Entry,
        SkillFrontmatter, SkillResource, SkillResources,
    },
};
use serde_json::Map;

/// The extension's name in a server's capabilities.
pub(crate) const EXTENSION: &str = SKILLS_EXTENSION_ID;

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

/// A skill as `skills/list` and `skills/get` give it: every field of its frontmatter and its one
/// file, with that file's digest and size.
fn entry(skill: &Skill) -> Entry {
    let uri = uri(skill);
    let frontmatter = skill
        .frontmatter
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "name" | "description"))
        .fold(
            SkillFrontmatter::new(skill.name, skill.description.clone()),
            |frontmatter, (key, value)| frontmatter.with_field(key.clone(), value.clone()),
        );
    let file = SkillResource::new(uri.clone(), skill.digest.clone(), skill.size());
    Entry::new(uri, frontmatter, SkillResources::Files(vec![file]))
}

/// `skills/list`: every skill the server offers, on one page. rmcp answers method-not-found
/// before this on a server that declares no skills.
pub(crate) fn list(
    skills: &[&Skill],
    request: Option<PaginatedRequestParams>,
) -> Result<ListSkillsResult, ErrorData> {
    if request.is_some_and(|request| request.cursor.is_some()) {
        return Err(ErrorData::invalid_params(
            "This skill list has no next page",
            None,
        ));
    }
    Ok(ListSkillsResult::with_all_items(
        skills.iter().map(|skill| entry(skill)).collect(),
    ))
}

/// `skills/get` of the skill whose `SKILL.md` is at `uri`.
pub(crate) fn get(skills: &[&Skill], uri: &str) -> Result<GetSkillResult, ErrorData> {
    Ok(GetSkillResult::new(entry(named(skills, uri)?)))
}

/// A request whose parameters rmcp could not read as those of a method it knows, such as
/// `skills/get` without a `uri`, which it hands on as a custom request: invalid params, `-32602`,
/// for the extension's methods on a server that offers skills, and method-not-found, as rmcp
/// answers by itself, for anything else.
pub(crate) fn unread(skills: &[&Skill], request: &CustomRequest) -> ErrorData {
    let method = request.method.as_str();
    if !skills.is_empty() && matches!(method, "skills/list" | "skills/get") {
        return ErrorData::invalid_params(format!("The parameters of {method} do not read"), None);
    }
    ErrorData::new(ErrorCode::METHOD_NOT_FOUND, method.to_owned(), None)
}
