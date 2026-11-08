//! Pins down the skills the manager serves: every folder of `docs/skills` is built in and reads,
//! the frontmatter goes out exactly as YAML reads it, the web serves the discovery index and the
//! files without a session, and `/mcp` serves them as the MCP Skills extension with the pointer
//! in its instructions.
use super::*;
use crate::{
    router,
    test_support::{app, mcp_app, mcp_request},
};
use axum::{body::Body, http::Request};
use tower::ServiceExt;

/// A GET of `path` on the manager, answered with its status, content type and body.
async fn get(app: std::sync::Arc<crate::App>, path: &str) -> (StatusCode, String, String) {
    let response = router(app)
        .oneshot(
            Request::builder()
                .uri(path)
                .header("host", "localhost:8080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let kind = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, kind, String::from_utf8(bytes.to_vec()).unwrap())
}

#[test]
fn every_folder_of_docs_skills_is_built_in_and_reads() {
    let folder = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/skills");
    let mut found = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_type().unwrap().is_dir())
        .map(|entry| entry.file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    found.sort();
    let mut built_in = SOURCES
        .iter()
        .map(|source| source.folder)
        .collect::<Vec<_>>();
    built_in.sort();
    assert_eq!(
        found, built_in,
        "every folder of docs/skills goes into SOURCES in skills.rs, so the manager serves it"
    );
    for source in SOURCES {
        let skill =
            Skill::read(source).unwrap_or_else(|error| panic!("{}: {error}", source.folder));
        assert!(
            !skill.servers.is_empty(),
            "{} is offered nowhere",
            skill.name
        );
        assert!(
            !skill.before.ends_with('.') && !skill.before.starts_with("before"),
            "{}: `before` follows \"before you\" in a sentence",
            skill.name
        );
        // Within the limits SEP-2640 holds hosts to.
        assert!(skill.size() <= 16 * 1024 * 1024);
    }
    assert_eq!(SKILLS.len(), SOURCES.len());
}

#[test]
fn the_frontmatter_goes_out_as_yaml_reads_it_or_is_refused() {
    let read = |head: &str| frontmatter(&format!("---\n{head}\n---\n# Body\n"));
    let fields =
        read("name: some-skill\ndescription: Does a thing, (\"quoted\") with `code`.").unwrap();
    assert_eq!(fields["name"], "some-skill");
    assert_eq!(
        fields["description"],
        "Does a thing, (\"quoted\") with `code`."
    );
    // Windows line endings read the same.
    assert_eq!(
        frontmatter("---\r\nname: a\r\n---\r\n").unwrap()["name"],
        "a"
    );
    for refused in [
        "description: \"quoted\"",
        "description: 'quoted'",
        "description: has: a colon",
        "description: a comment # here",
        "description: true",
        "description: 12",
        "description: - a list",
        "description: [a, b]",
        "description: > folded",
        "metadata:",
        "  nested: value",
        "Name: capital",
        "name: a\nname: b",
    ] {
        assert!(read(refused).is_err(), "{refused:?} must be refused");
    }
    assert!(frontmatter("# No frontmatter\n").is_err());
    assert!(frontmatter("---\nname: a\n").is_err());
}

#[test]
fn a_skill_is_named_as_the_agent_skills_specification_names_them() {
    let skill = |folder: &'static str, content: &'static str| {
        Skill::read(&Source {
            folder,
            content,
            servers: &[Server::Root],
            before: "work",
        })
    };
    assert!(skill("a-skill", "---\nname: a-skill\ndescription: Works.\n---\n").is_ok());
    assert!(
        skill("other", "---\nname: a-skill\ndescription: Works.\n---\n")
            .err()
            .unwrap()
            .contains("folder")
    );
    for name in ["-a", "a-", "a--b", "A"] {
        let content = format!("---\nname: {name}\ndescription: Works.\n---\n").leak();
        let folder = name.to_owned().leak();
        assert!(skill(folder, content).is_err(), "{name}");
    }
    assert!(skill("a", "---\nname: a\n---\n").is_err());
}

#[test]
fn a_digest_is_the_sha256_of_the_bytes_in_lowercase_hex() {
    assert_eq!(
        digest(b"abc"),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[tokio::test]
async fn the_web_serves_the_index_and_each_skill_without_a_session() {
    // Sign-in is on and the request has no session.
    let app = app(true);
    let (status, kind, body) = get(app.clone(), "/.well-known/agent-skills/index.json").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(kind.starts_with("application/json"), "{kind}");
    let index: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(index["$schema"], DISCOVERY_SCHEMA);
    let entries = index["skills"].as_array().unwrap();
    assert_eq!(entries.len(), SKILLS.len());
    for entry in entries {
        assert_eq!(entry["type"], "skill-md");
        let url = entry["url"].as_str().unwrap();
        let (status, kind, file) = get(app.clone(), url).await;
        assert_eq!(status, StatusCode::OK, "{url}");
        assert!(kind.starts_with("text/markdown"), "{kind}");
        // A client checks what it downloads against the index.
        assert_eq!(entry["digest"], digest(file.as_bytes()));
        assert_eq!(entry["name"], frontmatter(&file).unwrap()["name"], "{url}");
    }
    let (status, _, _) = get(app.clone(), "/.well-known/agent-skills/nothing/SKILL.md").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The dashboard's list needs the session, as the rest of /api does.
    let (status, _, _) = get(app, "/api/skills").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_dashboard_lists_where_each_skill_is_served() {
    let (status, _, body) = get(app(false), "/api/skills").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed: Value = serde_json::from_str(&body).unwrap();
    let scale = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|skill| skill["name"] == "thoric-scale-sets")
        .expect("the scale set skill");
    assert_eq!(
        scale["url"],
        "/.well-known/agent-skills/thoric-scale-sets/SKILL.md"
    );
    assert_eq!(scale["servers"], json!(["root", "set"]));
    assert_eq!(scale["size"], find("thoric-scale-sets").unwrap().size());
}

#[tokio::test]
async fn the_root_mcp_serves_its_skills_as_the_skills_extension() {
    let app = mcp_app();
    let skill = find("thoric-scale-sets").unwrap();
    let uri = "skill://thoric-scale-sets/SKILL.md";
    let (status, body) = mcp_request(
        app.clone(),
        "initialize",
        json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let capabilities = &body["result"]["capabilities"];
    assert!(capabilities["resources"].is_object(), "{capabilities}");
    assert!(
        capabilities["extensions"][mcp::EXTENSION].is_object(),
        "{capabilities}"
    );
    // A client that does not know the extension still learns where the skill is and when to
    // read it.
    let instructions = body["result"]["instructions"].as_str().unwrap();
    assert!(
        instructions.contains("Read the skill thoric-scale-sets before you create instances")
            && instructions.contains(uri),
        "{instructions}"
    );

    let (_, body) = mcp_request(app.clone(), "resources/list", json!({})).await;
    let resources = body["result"]["resources"].as_array().unwrap();
    let listed = resources
        .iter()
        .find(|resource| resource["uri"] == uri)
        .unwrap_or_else(|| panic!("{body}"));
    assert_eq!(listed["mimeType"], MARKDOWN);
    assert_eq!(listed["size"], skill.size());

    let (_, body) = mcp_request(app.clone(), "resources/read", json!({"uri": uri})).await;
    let contents = &body["result"]["contents"][0];
    assert_eq!(contents["text"], skill.content, "{body}");
    assert_eq!(contents["mimeType"], MARKDOWN);

    let (_, body) = mcp_request(app.clone(), "skills/list", json!({})).await;
    let entry = &body["result"]["skills"][0];
    assert_eq!(entry["uri"], uri, "{body}");
    assert_eq!(
        entry["frontmatter"],
        Value::Object(skill.frontmatter.clone())
    );
    assert_eq!(
        entry["resources"],
        json!([{"uri": uri, "digest": skill.digest, "size": skill.size()}])
    );
    let (_, body) = mcp_request(app.clone(), "skills/get", json!({"uri": uri})).await;
    assert_eq!(body["result"]["skill"], *entry, "{body}");

    // An unknown skill is -32602, the error SEP-2640 names; an unknown method stays unknown.
    for (method, params) in [
        ("skills/get", json!({"uri": "skill://nothing/SKILL.md"})),
        ("skills/get", json!({})),
        ("resources/read", json!({"uri": "skill://nothing/SKILL.md"})),
    ] {
        let (_, body) = mcp_request(app.clone(), method, params).await;
        assert_eq!(body["error"]["code"], -32602, "{method}: {body}");
    }
    let (_, body) = mcp_request(app, "skills/nothing", json!({})).await;
    assert_eq!(body["error"]["code"], -32601, "{body}");
}
