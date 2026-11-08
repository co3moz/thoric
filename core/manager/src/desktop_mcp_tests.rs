//! Desktop MCP servers and MCP endpoints: which declarations are accepted, what an endpoint offers
//! for its scopes and nothing more, the names its tools get, which single tools a grant leaves
//! out, and that an endpoint's token opens that endpoint and no other route, also one an earlier
//! release stored the hash of. Also that a server that hung is not handed to the next call.
use super::{
    client::{Exec, MAX_TOOLS, check_tool_list, exec_args, read_line, relayed},
    endpoints::{ENDPOINTS, Endpoint, Grant, GrantRequest, Servers, grants, slug_of, summary},
    registry::{
        Declaration, SERVERS, ServerRecord, ServerSpec, Source, check_declarations, found,
        loopback_url, merged, substitute,
    },
    scopes::{
        ENDPOINT_WORDING, Scope, Target, bound_tool, catalog, exposed_name, exposed_name_hashed,
    },
    *,
};
use crate::{
    model::{CONTAINERS, Container},
    test_support::{app, mcp_app},
};
use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
use tokio::io::BufReader;
use tower::ServiceExt;

const ADMIN: &str = "test-token-with-at-least-32-characters";

pub(super) fn container(id: &str, name: &str) -> Container {
    Container {
        id: id.into(),
        container: format!("thoric-{id}"),
        private_network: format!("thoric-{id}-private"),
        egress_network: format!("thoric-{id}-egress"),
        source_image_id: "sha256:chrome".into(),
        created_at: 1,
        spec: serde_json::from_value(json!({"name": name, "image": "sha256:chrome"})).unwrap(),
        last_error: None,
        desktop: None,
        working_image: None,
        setup: None,
    }
}
fn stdio(command: &str) -> ServerSpec {
    ServerSpec {
        command: Some(command.into()),
        ..Default::default()
    }
}
fn tool(name: &str) -> rmcp::model::Tool {
    let mut tool = rmcp::model::Tool::new(
        name.to_owned(),
        format!("The {name} tool"),
        json!({"type":"object","properties":{"url":{"type":"string"}}})
            .as_object()
            .unwrap()
            .clone(),
    );
    tool.annotations = Some(rmcp::model::ToolAnnotations::new().read_only(true));
    tool
}
pub(super) fn server(app: &App, container_id: &str, name: &str, tools: &[&str]) {
    let record = ServerRecord {
        container_id: container_id.into(),
        name: name.into(),
        source: Source::Image,
        spec: Some(stdio("playwright-mcp")),
        tools: tools.iter().map(|t| tool(t)).collect(),
        tools_at: Some(1),
        error: None,
    };
    app.db
        .put(SERVERS, &ServerRecord::key(container_id, name), &record)
        .unwrap();
}
pub(super) fn grant(container_id: &str, alias: &str, scopes: &[Scope]) -> Grant {
    Grant {
        container_id: container_id.into(),
        set: None,
        alias: alias.into(),
        scopes: scopes.to_vec(),
        servers: Servers::All,
        excluded_tools: Vec::new(),
    }
}
pub(super) fn endpoint(name: &str, grants: Vec<Grant>) -> Endpoint {
    Endpoint {
        name: name.into(),
        automatic: false,
        grants,
        token_hash: None,
        token_created_at: None,
        created_at: 1,
    }
}
pub(super) fn names(app: &App, endpoint: &Endpoint) -> Vec<String> {
    catalog(app, endpoint)
        .into_iter()
        .map(|entry| entry.tool.name.to_string())
        .collect()
}

#[test]
fn a_declaration_is_a_command_or_a_loopback_url_and_never_both() {
    assert!(stdio("playwright-mcp").validate().is_ok());
    let http = ServerSpec {
        url: Some("http://127.0.0.1:8931/mcp".into()),
        headers: BTreeMap::from([("Authorization".into(), "Bearer ${API_TOKEN}".into())]),
        ..Default::default()
    };
    assert!(http.validate().is_ok());
    assert_eq!(http.transport(), "http");
    for refused in [
        ServerSpec::default(),
        ServerSpec {
            url: Some("http://127.0.0.1:1/mcp".into()),
            ..stdio("x")
        },
        // A program takes env, an HTTP server headers; mixing them is a mistake worth naming.
        ServerSpec {
            headers: BTreeMap::from([("X-Key".into(), "1".into())]),
            ..stdio("x")
        },
        ServerSpec {
            env: BTreeMap::from([("DISPLAY".into(), ":1".into())]),
            ..stdio("x")
        },
        ServerSpec {
            env: BTreeMap::from([("PATH".into(), "/tmp".into())]),
            ..stdio("x")
        },
        ServerSpec {
            cwd: Some("relative".into()),
            ..stdio("x")
        },
        ServerSpec {
            url: Some("http://127.0.0.1:8931/mcp".into()),
            headers: BTreeMap::from([("Mcp-Session-Id".into(), "x".into())]),
            ..Default::default()
        },
        ServerSpec {
            url: Some("http://127.0.0.1:8931/mcp".into()),
            headers: BTreeMap::from([("X-Key".into(), "a\r\nHost: evil".into())]),
            ..Default::default()
        },
    ] {
        assert!(refused.validate().is_err(), "{refused:?}");
    }
}

#[test]
fn an_http_server_is_on_the_desktops_own_loopback() {
    for url in [
        "http://127.0.0.1:8931/mcp",
        "https://localhost:443/mcp",
        "http://[::1]:9000/",
        "http://127.0.0.1:8931",
    ] {
        assert!(loopback_url(url, false).is_ok(), "{url}");
    }
    for url in [
        "http://example.com:80/mcp",
        "http://127.0.0.1/mcp",
        "http://127.0.0.1:0/mcp",
        "http://127.0.0.1.example.com:80/mcp",
        "http://user@127.0.0.1:80/mcp",
        "ftp://127.0.0.1:21/",
        "http://127.0.0.1:80/a b",
        "http://10.0.0.1:80/mcp",
    ] {
        assert!(loopback_url(url, false).is_err(), "{url}");
    }
    // A compose value may hold the port, and is checked again once it is known.
    assert!(loopback_url("http://127.0.0.1:${PORT}/mcp", true).is_ok());
    assert!(loopback_url("http://127.0.0.1:${PORT}/mcp", false).is_err());
}

#[test]
fn compose_values_fill_placeholders_and_a_missing_one_is_an_error() {
    let values = BTreeMap::from([
        ("PORT".to_owned(), "8931".to_owned()),
        ("chrome.TOKEN".to_owned(), "s3cret".to_owned()),
    ]);
    assert_eq!(
        substitute("http://127.0.0.1:${PORT}/mcp", &values).unwrap(),
        "http://127.0.0.1:8931/mcp"
    );
    assert_eq!(
        substitute("Bearer ${chrome.TOKEN}", &values).unwrap(),
        "Bearer s3cret"
    );
    assert_eq!(substitute("$HOME and $5", &values).unwrap(), "$HOME and $5");
    assert!(substitute("${MISSING}", &values).is_err());
    // A declaration written for a compose of one step keeps its names once the image's compose
    // has several: a name alone stands for the one step's value that has it.
    assert_eq!(substitute("${TOKEN}", &values).unwrap(), "s3cret");
    let merged = BTreeMap::from([
        ("chrome.PORT".to_owned(), "8931".to_owned()),
        ("vscode.PORT".to_owned(), "3000".to_owned()),
        ("PORT".to_owned(), "1".to_owned()),
    ]);
    assert_eq!(substitute("${PORT}", &merged).unwrap(), "1");
    let both = BTreeMap::from_iter(merged.into_iter().filter(|(key, _)| key != "PORT"));
    let refused = substitute("${PORT}", &both).unwrap_err().to_string();
    assert!(refused.contains("chrome.PORT or vscode.PORT"), "{refused}");
    assert!(substitute("${other.TOKEN}", &values).is_err());
    assert!(substitute("${PORT", &values).is_err());
    assert!(substitute("${a b}", &values).is_err());
    let spec = ServerSpec {
        url: Some("http://127.0.0.1:${PORT}/mcp".into()),
        headers: BTreeMap::from([("Authorization".into(), "Bearer ${chrome.TOKEN}".into())]),
        ..Default::default()
    };
    let resolved = spec.resolve(&values).unwrap();
    assert_eq!(resolved.url.as_deref(), Some("http://127.0.0.1:8931/mcp"));
    assert_eq!(resolved.headers["Authorization"], "Bearer s3cret");
}

#[test]
fn lists_name_variables_and_headers_but_never_show_their_values() {
    let spec = ServerSpec {
        url: Some("http://127.0.0.1:8931/mcp".into()),
        headers: BTreeMap::from([("Authorization".into(), "Bearer hunter2".into())]),
        ..Default::default()
    };
    let summary = spec.summary().to_string();
    assert!(summary.contains("Authorization"));
    assert!(!summary.contains("hunter2"));
}

#[test]
fn a_declaration_has_a_name_and_only_known_fields() {
    let declaration: Declaration = serde_json::from_value(
        json!({"name":"chrome","command":"playwright-mcp","args":["--cdp-endpoint","http://127.0.0.1:9223"]}),
    )
    .unwrap();
    assert_eq!(declaration.name, "chrome");
    assert_eq!(declaration.spec.args.len(), 2);
    assert!(declaration.validate().is_ok());
    assert!(serde_json::from_value::<Declaration>(json!({"command":"x"})).is_err());
    assert!(
        serde_json::from_value::<Declaration>(json!({"name":"x","command":"x","port":1})).is_err()
    );
    for name in ["Chrome", "-x", "a_b", "", &"a".repeat(33)] {
        let declaration = Declaration {
            name: name.into(),
            spec: stdio("x"),
        };
        assert!(declaration.validate().is_err(), "{name}");
    }
    // A build declares each server once.
    let twice = vec![
        Declaration {
            name: "chrome".into(),
            spec: stdio("a"),
        },
        Declaration {
            name: "chrome".into(),
            spec: stdio("b"),
        },
    ];
    assert!(check_declarations(&twice).is_err());
    assert!(check_declarations(&twice[..1]).is_ok());
}

#[test]
fn the_desktop_users_declaration_replaces_the_images_and_bad_files_become_errors() {
    let image = json!({"servers":[
        {"name":"chrome","source":"image","spec":{"command":"playwright-mcp"}},
        {"name":"x","source":"image","error":"x.json is not valid JSON"},
        {"name":"","source":"image","error":"Cannot open /etc/thoric/mcp.d: /etc/thoric can be changed by a user other than root"}
    ]});
    let user = json!({"servers":[
        {"name":"chrome","source":"container","spec":{"command":"my-chrome-mcp"}},
        {"name":"bad","source":"container","spec":{"command":"a","url":"http://127.0.0.1:1/"}},
        {"name":"Not-A-Name","source":"container","spec":{"command":"a"}}
    ]});
    let servers = merged(
        found(&image, Source::Image).unwrap(),
        found(&user, Source::Container).unwrap(),
    );
    assert_eq!(servers.len(), 3);
    assert_eq!(
        servers.iter().find(|s| s.name == "x").unwrap().source,
        Source::Image
    );
    let chrome = servers.iter().find(|s| s.name == "chrome").unwrap();
    assert_eq!(chrome.source, Source::Container);
    assert_eq!(
        chrome.spec.as_ref().unwrap().command.as_deref(),
        Some("my-chrome-mcp")
    );
    assert!(
        servers
            .iter()
            .find(|s| s.name == "x")
            .unwrap()
            .spec
            .is_err()
    );
    assert!(
        servers
            .iter()
            .find(|s| s.name == "bad")
            .unwrap()
            .spec
            .is_err()
    );
}

#[tokio::test]
async fn the_desktop_users_servers_apply_while_the_images_folder_could_not_be_read() {
    let app = app(false);
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome dev"))
        .unwrap();
    server(&app, "c1", "chrome", &["navigate"]);
    let mine = ServerRecord {
        name: "x".into(),
        source: Source::Container,
        ..app.db.get(SERVERS, "c1:chrome").unwrap()
    };
    app.db.put(SERVERS, "c1:x", &mine).unwrap();
    let user = || {
        vec![super::registry::Found {
            name: "y".into(),
            source: Source::Container,
            spec: Ok(stdio("node")),
        }]
    };
    let kept = |key: &str| app.db.get::<ServerRecord>(SERVERS, key).is_ok();
    // The desktop user's listing replaces that user's servers, and the image's stay unknown.
    super::watch::apply_with(&app, "c1", user(), false).await;
    assert!(kept("c1:chrome") && kept("c1:y") && !kept("c1:x"));
    // Once the image's folder is read, a server it no longer declares goes.
    super::watch::apply_with(&app, "c1", user(), true).await;
    assert!(!kept("c1:chrome") && kept("c1:y"));
}

#[test]
fn the_desktop_users_helper_cannot_claim_the_images_folder_for_a_server() {
    // The watcher runs as the desktop user, who can make it print anything.
    let forged = json!({"servers":[
        {"name":"leak","source":"image","spec":{"command":"env"}},
        {"name":"other","spec":{"command":"env"}}
    ], "image": [["leak.json", 1, 2, 3]]});
    let servers = found(&forged, Source::Container).unwrap();
    assert!(
        servers.iter().all(|s| s.source == Source::Container),
        "{servers:?}"
    );
    // So its servers are handed no secret value.
    let app = app(false);
    let pinned: crate::compose::Pinned = serde_json::from_value(json!({
        "compose": {"parameters": [{"name": "API_TOKEN", "secret": true}], "script": "true"},
        "values": {"API_TOKEN": "s3cret"},
    }))
    .unwrap();
    app.save_pinned("c1", &pinned).unwrap();
    let handed = servers[0].source.values(&app, "c1");
    assert!(!handed.contains_key("API_TOKEN"), "{handed:?}");
    assert_eq!(Source::Image.values(&app, "c1")["API_TOKEN"], "s3cret");
    // A record that names no source, or one the manager does not know, gets no secret either.
    for source in [json!("container"), json!("Image"), json!("root"), json!("")] {
        let record: ServerRecord = serde_json::from_value(json!({
            "container_id": "c1", "name": "leak", "source": source, "spec": null,
        }))
        .unwrap();
        assert_eq!(record.source, Source::Container, "{source}");
    }
    let record: ServerRecord = serde_json::from_value(
        json!({"container_id": "c1", "name": "chrome", "source": "image", "spec": null}),
    )
    .unwrap();
    assert_eq!(record.source, Source::Image);
    assert_eq!(serde_json::to_value(record.source).unwrap(), "image");
}

#[test]
fn a_server_runs_as_the_desktop_user_and_secret_values_reach_neither_a_command_line_nor_the_cli() {
    let spec = ServerSpec {
        env: BTreeMap::from([("BROWSER".into(), "chrome".into())]),
        args: vec!["--headless".into()],
        ..stdio("playwright-mcp")
    };
    let Exec { args, first } = exec_args("abc", &spec).unwrap();
    assert_eq!(&args[..4], ["exec", "-i", "--user", "1001:1001"]);
    // The value, which may hold a compose secret, is on no command line and reaches the launcher
    // on stdin, sized on its command line; the Docker CLI's environment is the manager's own.
    assert!(!args.iter().any(|a| a.contains("chrome")));
    let first = first.unwrap();
    let variables: Value = serde_json::from_slice(&first).unwrap();
    assert_eq!(variables, serde_json::json!({"BROWSER": "chrome"}));
    assert!(
        args.iter()
            .any(|a| a.starts_with("PATH=/home/desktop/.local/bin:"))
    );
    let at = args.iter().position(|a| a == "abc").unwrap();
    assert_eq!(&args[at + 1..at + 3], ["python3", "-I"]);
    assert_eq!(
        &args[at + 5..],
        [
            first.len().to_string().as_str(),
            "playwright-mcp",
            "--headless"
        ]
    );
    // Without variables the program runs itself, as before.
    let plain = ServerSpec {
        args: vec!["--headless".into()],
        ..stdio("playwright-mcp")
    };
    let Exec { args, first } = exec_args("abc", &plain).unwrap();
    assert!(first.is_none());
    let at = args.iter().position(|a| a == "abc").unwrap();
    assert_eq!(&args[at + 1..], ["playwright-mcp", "--headless"]);
    let http = ServerSpec {
        url: Some("http://127.0.0.1:8931/mcp".into()),
        headers: BTreeMap::from([("Authorization".into(), "Bearer hunter2".into())]),
        ..Default::default()
    };
    let Exec { args, first, .. } = exec_args("abc", &http).unwrap();
    assert!(!args.iter().any(|a| a.contains("hunter2")));
    let first: Value = serde_json::from_slice(&first.unwrap()).unwrap();
    assert_eq!(first["headers"]["Authorization"], "Bearer hunter2");
    assert_eq!(first["url"], "http://127.0.0.1:8931/mcp");
}

#[tokio::test]
async fn a_desktop_cannot_make_the_manager_hold_an_endless_line() {
    let endless = [
        b"short\n".as_slice(),
        &[b'x'; 100],
        b"\n\xff\xfe not text\n",
        b"last",
    ]
    .concat();
    // A small buffer, so a line arrives over several reads as from a pipe.
    let mut reader = BufReader::with_capacity(16, endless.as_slice());
    let mut line = Vec::new();
    let mut read = Vec::new();
    while let Some(cut) = read_line(&mut reader, &mut line, 10).await.unwrap() {
        read.push((line.clone(), cut));
    }
    assert_eq!(
        read,
        [
            (b"short\n".to_vec(), false),
            (b"xxxxxxxxxx".to_vec(), true),
            (b"\xff\xfe not tex".to_vec(), true),
            (b"last".to_vec(), false),
        ]
    );
}

#[test]
fn exposed_names_are_valid_client_tool_names_of_at_most_64() {
    assert_eq!(
        exposed_name(&["chrome", "browser_navigate"]),
        "chrome__browser_navigate"
    );
    assert_eq!(exposed_name(&["x", "a.b/c"]), "x__a_b_c");
    let long = exposed_name(&["ext-dev", "chrome", &"t".repeat(80)]);
    assert_eq!(long.len(), 64);
    assert_ne!(long, exposed_name(&["ext-dev", "chrome", &"t".repeat(81)]));
    assert!(
        long.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    );
}

#[test]
fn a_hash_an_earlier_release_stored_or_offered_stays_the_same() {
    // An endpoint record keeps only its token's BLAKE2s-256, so a token issued before must still
    // open it. The digest is the one Python's hashlib.blake2s gives.
    let token = "bsk_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let mut stored = endpoint("agents", Vec::new());
    stored.token_hash =
        Some("892a26ef2a6e1206944a4bbd388e09bbbc720cd372a5973664e583a99fd12133".into());
    assert!(stored.opens(token));
    assert!(!stored.opens(&token.replace('f', "e")));
    // The hash tail of a tool's name is what agents call and what a grant leaves out by name.
    assert_eq!(exposed_name_hashed(&["s", "a_b"]), "s__a_b_885b26");
    assert_eq!(
        exposed_name(&["ext-dev", "chrome", &"t".repeat(80)]),
        format!("ext-dev__chrome__{}_5de7f8", "t".repeat(40))
    );
}

#[test]
fn a_bound_tool_takes_no_container_id_and_never_root() {
    let exec = crate::mcp::listed_tool("exec_container").unwrap();
    let bound = bound_tool(&exec, None);
    let properties = bound.input_schema["properties"].as_object().unwrap();
    assert!(!properties.contains_key("container_id"));
    assert!(!properties.contains_key("root"));
    assert_eq!(bound.input_schema["required"], json!(["script"]));
    let several = bound_tool(&exec, Some(&["a".into(), "b".into()]));
    assert_eq!(
        several.input_schema["properties"]["container"]["enum"],
        json!(["a", "b"])
    );
    assert!(
        several.input_schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("container"))
    );
}

/// exec_container on an endpoint said to set root=true, which the endpoint then refused as an
/// unknown argument, and download_file pointed at create_transfer and a dashboard route that no
/// endpoint offers. A bound tool's description names nothing the endpoint lacks.
#[test]
fn a_bound_tool_describes_only_what_the_endpoint_offers() {
    for (name, sentence, _) in ENDPOINT_WORDING {
        let tool = crate::mcp::listed_tool(name).unwrap();
        assert!(
            tool.description.as_deref().unwrap().contains(sentence),
            "{name} no longer says {sentence:?}"
        );
    }
    let aliases = ["a".to_owned(), "b".to_owned()];
    for name in Scope::ALL.iter().flat_map(|scope| scope.tools()) {
        let tool = crate::mcp::listed_tool(name).unwrap();
        for bound in [bound_tool(&tool, None), bound_tool(&tool, Some(&aliases))] {
            let description = bound.description.as_deref().unwrap();
            for word in ["root=true", "container_id", "create_transfer", "/api/"] {
                assert!(!description.contains(word), "{name} says {word}");
            }
        }
    }
    let exec = bound_tool(&crate::mcp::listed_tool("exec_container").unwrap(), None);
    assert!(exec.description.unwrap().contains("never as root"));
}

/// view held save_screenshot, which with replace=true overwrites any file the desktop user may
/// write, so a grant meant to watch a desktop could change its files. The scopes that read hold
/// only tools that say they are read-only.
#[test]
fn the_reading_scopes_change_nothing() {
    for scope in [Scope::View, Scope::FilesRead] {
        for name in scope.tools() {
            let tool = crate::mcp::listed_tool(name).unwrap();
            let read_only = tool.annotations.and_then(|a| a.read_only_hint);
            assert_eq!(read_only, Some(true), "{} offers {name}", scope.id());
        }
    }
    assert!(Scope::FilesWrite.offers("save_screenshot"));
}

#[test]
fn an_endpoint_offers_its_scopes_tools_and_never_the_administrators() {
    let app = app(false);
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome dev"))
        .unwrap();
    let viewer = endpoint("viewer", vec![grant("c1", "chrome-dev", &[Scope::View])]);
    let offered = names(&app, &viewer);
    assert!(offered.contains(&"screenshot".to_owned()));
    for absent in [
        "desktop_input",
        "exec_container",
        "get_job",
        "create_container",
        "delete_container",
        "update_container",
        "reset_container",
        "snapshot",
        "set_internet",
        "list_containers",
        "create_mcp_endpoint",
        "create_transfer",
    ] {
        assert!(!offered.contains(&absent.to_owned()), "{absent}");
    }
    // Every scope together still offers nothing that changes a container's settings or others.
    let everything = endpoint("all", vec![grant("c1", "chrome-dev", &Scope::ALL)]);
    let offered = names(&app, &everything);
    assert!(offered.contains(&"get_job".to_owned()));
    for administrator in [
        "create_container",
        "delete_container",
        "update_container",
        "reset_container",
        "retry_container",
        "snapshot",
        "duplicate_container",
        "set_internet",
        "list_containers",
        "build_image",
        "delete_image",
        "delete_volume",
        "list_mcp_endpoints",
        "call_mcp_server_tool",
    ] {
        assert!(
            !offered.contains(&administrator.to_owned()),
            "{administrator}"
        );
    }
    for name in Scope::ALL.iter().flat_map(|scope| scope.tools()) {
        assert!(
            crate::mcp::listed_tool(name).is_some(),
            "{name} is in a scope but not a Thoric tool"
        );
    }
}

#[test]
fn desktop_server_tools_are_prefixed_and_their_own_claims_dropped() {
    let app = app(false);
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome dev"))
        .unwrap();
    app.db
        .put(CONTAINERS, "c2", &container("c2", "Unity"))
        .unwrap();
    server(&app, "c1", "chrome", &["browser_navigate"]);
    server(&app, "c1", "x", &["reload_extension"]);
    server(&app, "c2", "unity", &["run_tests"]);
    let one = endpoint("one", vec![grant("c1", "chrome-dev", &[])]);
    let entries = catalog(&app, &one);
    let navigate = entries
        .iter()
        .find(|e| e.tool.name == "chrome__browser_navigate")
        .unwrap();
    // A desktop's server cannot make a client run its tool unasked by claiming it is read-only.
    assert!(navigate.tool.annotations.is_none());
    assert!(
        navigate
            .tool
            .description
            .as_deref()
            .unwrap()
            .starts_with("[Chrome dev / chrome]")
    );
    assert_eq!(
        navigate.target,
        Target::Server {
            container_id: "c1".into(),
            server: "chrome".into(),
            tool: "browser_navigate".into()
        }
    );
    assert!(names(&app, &one).contains(&"x__reload_extension".to_owned()));
    assert!(!names(&app, &one).iter().any(|n| n.contains("run_tests")));
    // A chosen list leaves the other servers out, also those added later.
    let mut chosen = one.clone();
    chosen.grants[0].servers = Servers::Named(vec!["x".into()]);
    assert_eq!(names(&app, &chosen), ["x__reload_extension"]);
    // Several containers: tools take the container, server tools carry its alias.
    let two = endpoint(
        "two",
        vec![
            grant("c1", "chrome-dev", &[Scope::View]),
            grant("c2", "unity", &[]),
        ],
    );
    let offered = names(&app, &two);
    assert!(offered.contains(&"chrome-dev__chrome__browser_navigate".to_owned()));
    assert!(offered.contains(&"unity__unity__run_tests".to_owned()));
    let screenshot = catalog(&app, &two)
        .into_iter()
        .find(|e| e.tool.name == "screenshot")
        .unwrap();
    // Only the containers granted the scope are offered for its tools.
    assert_eq!(
        screenshot.tool.input_schema["properties"]["container"]["enum"],
        json!(["chrome-dev"])
    );
    // A server that failed offers nothing.
    let mut failed: ServerRecord = app.db.get(SERVERS, &ServerRecord::key("c1", "x")).unwrap();
    failed.error = Some("crashed".into());
    app.db
        .put(SERVERS, &ServerRecord::key("c1", "x"), &failed)
        .unwrap();
    assert!(!names(&app, &one).contains(&"x__reload_extension".to_owned()));
}

#[test]
fn a_grant_leaves_single_tools_out_under_the_names_the_endpoint_gives_them() {
    let app = app(false);
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome dev"))
        .unwrap();
    app.db
        .put(CONTAINERS, "c2", &container("c2", "Unity"))
        .unwrap();
    server(
        &app,
        "c1",
        "playwright",
        &["browser_navigate", "browser_run_code_unsafe", "a.b", "a_b"],
    );
    // One container: the deny list holds the exposed names, a hash tail included.
    let mut one = endpoint(
        "one",
        vec![grant(
            "c1",
            "chrome-dev",
            &[Scope::View, Scope::Applications],
        )],
    );
    let before = names(&app, &one);
    assert!(before.contains(&"playwright__a_b".to_owned()));
    let hashed = before
        .iter()
        .find(|name| name.starts_with("playwright__a_b_"))
        .unwrap()
        .clone();
    one.grants[0].excluded_tools = vec![
        "application_diagnostics".into(),
        "playwright__browser_run_code_unsafe".into(),
        "playwright__a_b".into(),
    ];
    let after = names(&app, &one);
    for absent in [
        "application_diagnostics",
        "playwright__browser_run_code_unsafe",
        "playwright__a_b",
    ] {
        assert!(!after.contains(&absent.to_owned()), "{absent}");
    }
    for present in [
        "screenshot",
        "launch_application",
        "playwright__browser_navigate",
        hashed.as_str(),
    ] {
        assert!(after.contains(&present.to_owned()), "{present}");
    }
    // Leaving one of two colliding tools out does not rename the other.
    assert_eq!(before.len(), after.len() + 3);
    // Several containers: a container tool stays for the others and takes only their aliases; a
    // server tool is named without the alias, as the container alone would expose it.
    let mut two = endpoint(
        "two",
        vec![
            grant("c1", "chrome-dev", &[Scope::View]),
            grant("c2", "unity", &[Scope::View]),
        ],
    );
    two.grants[0].excluded_tools = vec![
        "screenshot".into(),
        "playwright__browser_run_code_unsafe".into(),
    ];
    let entries = catalog(&app, &two);
    let screenshot = entries
        .iter()
        .find(|entry| entry.tool.name == "screenshot")
        .unwrap();
    assert_eq!(
        screenshot.tool.input_schema["properties"]["container"]["enum"],
        json!(["unity"])
    );
    assert_eq!(
        screenshot.target,
        Target::Thoric {
            tool: "screenshot",
            containers: vec![("unity".into(), "c2".into())]
        }
    );
    let offered: Vec<String> = entries
        .iter()
        .map(|entry| entry.tool.name.to_string())
        .collect();
    assert!(offered.contains(&"chrome-dev__playwright__browser_navigate".to_owned()));
    assert!(!offered.contains(&"chrome-dev__playwright__browser_run_code_unsafe".to_owned()));
    // Once every grant leaves a tool out, it is gone.
    two.grants[1].excluded_tools = vec!["screenshot".into()];
    assert!(!names(&app, &two).contains(&"screenshot".to_owned()));
    // get_job reads only the jobs of the containers whose grants keep it.
    let mut jobs = endpoint(
        "jobs",
        vec![
            grant("c1", "a", &[Scope::Power]),
            grant("c2", "b", &[Scope::Power]),
        ],
    );
    jobs.grants[0].excluded_tools = vec!["get_job".into()];
    let job = catalog(&app, &jobs)
        .into_iter()
        .find(|entry| entry.tool.name == "get_job")
        .unwrap();
    assert_eq!(
        job.target,
        Target::Job {
            containers: vec!["c2".into()],
            set: false,
        }
    );
    jobs.grants[1].excluded_tools = vec!["get_job".into()];
    assert!(!names(&app, &jobs).contains(&"get_job".to_owned()));
}

#[test]
fn excluded_tools_are_checked_against_the_scopes_and_kept_for_servers_not_known_yet() {
    let app = app(false);
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome dev"))
        .unwrap();
    let request = |scopes: &[Scope], excluded: &[&str]| GrantRequest {
        container_id: "c1".into(),
        set: None,
        alias: None,
        scopes: scopes.to_vec(),
        servers: Servers::All,
        excluded_tools: excluded.iter().map(|name| (*name).to_owned()).collect(),
    };
    let made = grants(
        &app,
        vec![request(
            &[Scope::Applications, Scope::Power],
            &[
                "application_diagnostics",
                "playwright__browser_run_code_unsafe",
                "get_job",
                "application_diagnostics",
            ],
        )],
    )
    .unwrap();
    assert_eq!(
        made[0].excluded_tools,
        [
            "application_diagnostics",
            "playwright__browser_run_code_unsafe",
            "get_job"
        ]
    );
    // A container tool outside the granted scopes is a typo worth naming, as is a name no
    // exposed tool can have.
    for (scopes, excluded) in [
        (&[Scope::View][..], &["application_diagnostics"][..]),
        (&[Scope::View], &["get_job"]),
        (&[Scope::View], &["screenshot "]),
        (&[Scope::View], &[""]),
        (&[Scope::View], &["a.b"]),
        (&[Scope::View], &["Screenshot"]),
        (&[Scope::View], &["Chrome__navigate"]),
        (&[Scope::View], &["__navigate"]),
    ] {
        assert!(
            grants(&app, vec![request(scopes, excluded)]).is_err(),
            "{excluded:?}"
        );
    }
    let longest = format!("s__{}", "a".repeat(61));
    assert!(grants(&app, vec![request(&[], &[&longest])]).is_ok());
    assert!(grants(&app, vec![request(&[], &[&format!("{longest}a")])]).is_err());
    let many: Vec<String> = (0..1025).map(|i| format!("s__t{i}")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    assert!(grants(&app, vec![request(&[], &many)]).is_err());
    assert!(grants(&app, vec![request(&[], &many[..1024])]).is_ok());
    // A record from before the list reads as leaving nothing out, and a grant that leaves
    // nothing out is stored as records were, without the field.
    let old: Grant = serde_json::from_value(
        json!({"container_id":"c1","alias":"a","scopes":["view"],"servers":"all"}),
    )
    .unwrap();
    assert!(old.excluded_tools.is_empty());
    assert!(
        !serde_json::to_value(&old)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("excluded_tools")
    );
    // The dashboard and the administrator's tools see the list with each grant.
    let shown = summary(&app, &endpoint("e", made));
    assert_eq!(
        shown["grants"][0]["excluded_tools"],
        json!([
            "application_diagnostics",
            "playwright__browser_run_code_unsafe",
            "get_job"
        ])
    );
    assert_eq!(
        summary(&app, &endpoint("f", vec![old]))["grants"][0]["excluded_tools"],
        json!([])
    );
}

#[test]
fn server_lists_name_each_tool_as_an_endpoint_of_its_container_exposes_it() {
    let app = app(false);
    server(
        &app,
        "c1",
        "playwright",
        &["browser_navigate", "a.b", "a_b", "a/b"],
    );
    let record: ServerRecord = app
        .db
        .get(SERVERS, &ServerRecord::key("c1", "playwright"))
        .unwrap();
    let shown = super::servers::summary(&app, &record, "Chrome dev", true, &[]);
    let tools: Vec<(String, String)> = shown["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| {
            (
                tool["name"].as_str().unwrap().to_owned(),
                tool["exposed"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        tools[0],
        (
            "browser_navigate".to_owned(),
            "playwright__browser_navigate".to_owned()
        )
    );
    assert_eq!(tools[1], ("a.b".to_owned(), "playwright__a_b".to_owned()));
    assert_eq!(tools[2].0, "a_b");
    assert!(tools[2].1.starts_with("playwright__a_b_"));
    // The tail hashes the tool's own name, so a third tool that cleans the same gets its own.
    assert_eq!(tools[3].0, "a/b");
    assert!(tools[3].1.starts_with("playwright__a_b_") && tools[3].1 != tools[2].1);
    assert_eq!(tools.len(), 4);
    // The list's names are the catalog's on an endpoint of this container alone.
    let offered = names(&app, &endpoint("one", vec![grant("c1", "chrome-dev", &[])]));
    assert_eq!(
        offered,
        tools
            .iter()
            .map(|(_, exposed)| exposed.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn grants_name_existing_containers_once_with_their_own_aliases() {
    let app = app(false);
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome Dev!"))
        .unwrap();
    app.db
        .put(CONTAINERS, "c2", &container("c2", "Chrome Dev"))
        .unwrap();
    let request = |id: &str, alias: Option<&str>| GrantRequest {
        container_id: id.into(),
        set: None,
        alias: alias.map(str::to_owned),
        scopes: vec![Scope::View, Scope::View],
        servers: Servers::All,
        excluded_tools: Vec::new(),
    };
    let made = grants(&app, vec![request("c1", None), request("c2", None)]).unwrap();
    assert_eq!(made[0].alias, "chrome-dev");
    assert_eq!(made[1].alias, "chrome-dev-2");
    assert_eq!(made[0].scopes, [Scope::View]);
    assert!(grants(&app, vec![]).is_err());
    assert!(grants(&app, vec![request("missing", None)]).is_err());
    assert!(grants(&app, vec![request("c1", None), request("c1", None)]).is_err());
    assert!(grants(&app, vec![request("c1", Some("Bad Alias"))]).is_err());
    assert!(
        grants(
            &app,
            vec![request("c1", Some("same")), request("c2", Some("same"))]
        )
        .is_err()
    );
    assert_eq!(
        slug_of("  Ünity 6 / Editor  ", 24, "desktop"),
        "nity-6-editor"
    );
    assert_eq!(slug_of("***", 24, "desktop"), "desktop");
    assert_eq!(
        serde_json::from_value::<Servers>(json!("all")).unwrap(),
        Servers::All
    );
    assert!(serde_json::from_value::<Servers>(json!("some")).is_err());
    assert_eq!(json!(Servers::Named(vec!["x".into()])), json!(["x"]));
}

#[tokio::test]
async fn a_new_containers_endpoint_is_named_after_it_and_leaves_with_it() {
    let app = app(false);
    let one = container("c1", "Chrome dev");
    let two = container("c2", "Chrome dev");
    app.db.put(CONTAINERS, "c1", &one).unwrap();
    app.db.put(CONTAINERS, "c2", &two).unwrap();
    assert_eq!(automatic_endpoint(&app, &one).await.unwrap(), "chrome-dev");
    assert_eq!(
        automatic_endpoint(&app, &two).await.unwrap(),
        "chrome-dev-2"
    );
    let own: Endpoint = app.db.get(ENDPOINTS, "chrome-dev").unwrap();
    assert!(own.automatic && own.token_hash.is_none());
    assert_eq!(own.grants[0].scopes, Scope::DEFAULT);
    assert!(!own.grants[0].scopes.contains(&Scope::Commands));
    // A custom endpoint covering both keeps the other container when one is deleted.
    app.db
        .put(
            ENDPOINTS,
            "both",
            &endpoint("both", vec![grant("c1", "a", &[]), grant("c2", "b", &[])]),
        )
        .unwrap();
    server(&app, "c1", "chrome", &["browser_navigate"]);
    // A record this release cannot read goes with it; one of a container whose id only begins
    // the same way stays.
    app.db
        .put(
            SERVERS,
            &ServerRecord::key("c1", "broken"),
            &json!({"name": 7}),
        )
        .unwrap();
    server(&app, "c10", "unity", &["run_tests"]);
    forget_container(&app, "c1").unwrap();
    assert_eq!(
        app.db.ids(SERVERS).unwrap(),
        [ServerRecord::key("c10", "unity")]
    );
    assert!(app.db.get::<Endpoint>(ENDPOINTS, "chrome-dev").is_err());
    assert!(app.db.get::<Endpoint>(ENDPOINTS, "chrome-dev-2").is_ok());
    let both: Endpoint = app.db.get(ENDPOINTS, "both").unwrap();
    assert_eq!(both.grants.len(), 1);
    assert_eq!(both.grants[0].container_id, "c2");
    assert!(
        app.db
            .get::<ServerRecord>(SERVERS, &ServerRecord::key("c1", "chrome"))
            .is_err()
    );
}

/// One JSON-RPC message to `/mcp/{name}`, with a bearer token and a session when given.
async fn call(
    app: Arc<App>,
    path: &str,
    token: Option<&str>,
    session: Option<&str>,
    message: Value,
) -> (StatusCode, Option<String>, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header("host", "localhost:8080")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-11-25");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    if let Some(session) = session {
        request = request.header("mcp-session-id", session);
    }
    let response = crate::router(app)
        .oneshot(request.body(Body::from(message.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let session = response
        .headers()
        .get("mcp-session-id")
        .map(|v| v.to_str().unwrap().to_owned());
    assert!(!response.headers().contains_key("content-length"));
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    // Sessions answer on an event stream; the reply is its message with data.
    let text = String::from_utf8_lossy(&bytes);
    let body = serde_json::from_str(&text).unwrap_or_else(|_| {
        text.lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
            .next_back()
            .unwrap_or(Value::Null)
    });
    (status, session, body)
}
async fn open_session(app: &Arc<App>, path: &str, token: &str) -> String {
    let (status, session, body) = call(
        app.clone(),
        path,
        Some(token),
        None,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["result"]["capabilities"]["tools"]["listChanged"] == true,
        "{body}"
    );
    let session = session.expect("a session for this protocol version");
    let (status, _, _) = call(
        app.clone(),
        path,
        Some(token),
        Some(&session),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    session
}

#[tokio::test]
async fn an_endpoint_tells_its_agents_that_only_gpu_run_reaches_the_gpu() {
    // An agent on an endpoint ran glxinfo without gpu-run, read llvmpipe and found no /dev/dri,
    // and reported that a desktop with the WSL GPU had none.
    let app = mcp_app();
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Game"))
        .unwrap();
    let Json(created) = create_endpoint(
        State(app.clone()),
        Json(
            serde_json::from_value(json!({"name":"game","grants":[{"container_id":"c1","scopes":["applications","commands"],"servers":"all"}]}))
                .unwrap(),
        ),
    )
    .await
    .unwrap();
    let token = created["token"].as_str().unwrap().to_owned();
    let (status, _, body) = call(
        app.clone(),
        "/mcp/game",
        Some(&token),
        None,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let instructions = body["result"]["instructions"].as_str().unwrap();
    assert!(
        instructions.contains("gpu-run")
            && instructions.contains("llvmpipe")
            && instructions.contains("/dev/dxg"),
        "{instructions}"
    );
    let session = open_session(&app, "/mcp/game", &token).await;
    let (_, _, body) = call(
        app.clone(),
        "/mcp/game",
        Some(&token),
        Some(&session),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    )
    .await;
    let tools = body["result"]["tools"].as_array().unwrap();
    for name in ["exec_container", "launch_application"] {
        let tool = tools.iter().find(|t| t["name"] == name).unwrap();
        let description = tool["description"].as_str().unwrap();
        assert!(
            description.contains("gpu-run") && description.contains("llvmpipe"),
            "{description}"
        );
    }
}

#[tokio::test]
async fn an_endpoint_token_opens_its_endpoint_and_nothing_else() {
    let app = mcp_app();
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome dev"))
        .unwrap();
    server(&app, "c1", "chrome", &["browser_navigate"]);
    let Json(created) = create_endpoint(
        State(app.clone()),
        Json(
            serde_json::from_value(json!({"name":"chrome-dev","grants":[{"container_id":"c1","scopes":["view"],"servers":"all"}]}))
                .unwrap(),
        ),
    )
    .await
    .unwrap();
    let token = created["token"].as_str().unwrap().to_owned();
    assert_eq!(created["endpoint"]["path"], "/mcp/chrome-dev");
    assert_eq!(created["endpoint"]["has_token"], true);
    // The token is stored as a hash only.
    let stored: Endpoint = app.db.get(ENDPOINTS, "chrome-dev").unwrap();
    assert!(!serde_json::to_string(&stored).unwrap().contains(&token));

    let session = open_session(&app, "/mcp/chrome-dev", &token).await;
    let (status, _, body) = call(
        app.clone(),
        "/mcp/chrome-dev",
        Some(&token),
        Some(&session),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let tools: Vec<&str> = body["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(tools.contains(&"screenshot") && tools.contains(&"chrome__browser_navigate"));
    assert!(!tools.contains(&"delete_container") && !tools.contains(&"desktop_input"));
    // A tool outside the endpoint's scopes is unknown here, whatever /mcp offers. The refusal
    // sends the agent back to tools/list, since a tool may have gone with a desktop server.
    let (_, _, body) = call(
        app.clone(),
        "/mcp/chrome-dev",
        Some(&token),
        Some(&session),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"delete_container","arguments":{"container_id":"c1"}}}),
    )
    .await;
    let message = body["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("Unknown tool") && message.contains("list the tools again"),
        "{body}"
    );
    // The container is the endpoint's own: an argument naming another is refused, as a tool
    // error the agent reads.
    let (_, _, body) = call(
        app.clone(),
        "/mcp/chrome-dev",
        Some(&token),
        Some(&session),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"get_container","arguments":{"container_id":"other"}}}),
    )
    .await;
    assert_eq!(body["result"]["isError"], true, "{body}");
    assert_eq!(
        body["result"]["content"][0]["text"],
        "Unknown argument: container_id"
    );

    // The endpoint's token is not the administrator's: not /mcp, not the API, not ports.
    let (status, _, _) = call(
        app.clone(),
        "/mcp",
        Some(&token),
        None,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    for (method, uri) in [
        ("GET", "/api/containers"),
        ("GET", "/api/mcp/endpoints"),
        ("GET", "/api/containers/c1/ports/8080/"),
    ] {
        let response = crate::router(app.clone())
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("host", "localhost:8080")
                    .header("authorization", format!("Bearer {token}"))
                    .header("x-thoric-request", "1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{uri}");
    }
    // Another endpoint's token does not open this one; the administrator's does.
    let Json(other) = create_endpoint(
        State(app.clone()),
        Json(
            serde_json::from_value(
                json!({"name":"other","grants":[{"container_id":"c1","scopes":[],"servers":[]}]}),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    let (status, _, _) = call(
        app.clone(),
        "/mcp/chrome-dev",
        other["token"].as_str(),
        None,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    open_session(&app, "/mcp/chrome-dev", ADMIN).await;
    // A replaced token stops working; a revoked one leaves only the administrator's.
    let Json(replaced) = super::token(State(app.clone()), Path("chrome-dev".into()))
        .await
        .unwrap();
    let (status, _, _) = call(
        app.clone(),
        "/mcp/chrome-dev",
        Some(&token),
        None,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    open_session(&app, "/mcp/chrome-dev", replaced["token"].as_str().unwrap()).await;
    let Json(revoked) = revoke_token(State(app.clone()), Path("chrome-dev".into()))
        .await
        .unwrap();
    assert_eq!(revoked["has_token"], false);
    let (status, _, _) = call(
        app.clone(),
        "/mcp/chrome-dev",
        replaced["token"].as_str(),
        None,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_excluded_tool_is_unknown_on_the_endpoint() {
    let app = mcp_app();
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome dev"))
        .unwrap();
    server(
        &app,
        "c1",
        "chrome",
        &["browser_navigate", "browser_run_code_unsafe"],
    );
    let Json(created) = create_endpoint(
        State(app.clone()),
        Json(
            serde_json::from_value(json!({"name":"chrome-dev","grants":[{"container_id":"c1","scopes":["view"],"servers":"all","excluded_tools":["screenshot","chrome__browser_run_code_unsafe"]}]}))
                .unwrap(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        created["endpoint"]["grants"][0]["excluded_tools"],
        json!(["screenshot", "chrome__browser_run_code_unsafe"])
    );
    let token = created["token"].as_str().unwrap().to_owned();
    let session = open_session(&app, "/mcp/chrome-dev", &token).await;
    let (_, _, body) = call(
        app.clone(),
        "/mcp/chrome-dev",
        Some(&token),
        Some(&session),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    )
    .await;
    let tools: Vec<&str> = body["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(
        tools.contains(&"list_windows") && tools.contains(&"chrome__browser_navigate"),
        "{tools:?}"
    );
    assert!(
        !tools.contains(&"screenshot") && !tools.contains(&"chrome__browser_run_code_unsafe"),
        "{tools:?}"
    );
    // Called anyway, an excluded tool is as unknown as one outside the scopes.
    for name in ["screenshot", "chrome__browser_run_code_unsafe"] {
        let (_, _, body) = call(
            app.clone(),
            "/mcp/chrome-dev",
            Some(&token),
            Some(&session),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":name,"arguments":{}}}),
        )
        .await;
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Unknown tool"),
            "{body}"
        );
    }
    // A container tool of a scope the grant lacks is refused, so a typo cannot pass for one.
    let refused = update_endpoint(
        State(app.clone()),
        Path("chrome-dev".into()),
        Json(
            serde_json::from_value(json!({"grants":[{"container_id":"c1","scopes":["view"],"servers":"all","excluded_tools":["desktop_input"]}]}))
                .unwrap(),
        ),
    )
    .await;
    let Err(error) = refused else {
        panic!("a tool outside the scopes was accepted as excluded");
    };
    assert!(error.1.contains("desktop_input"), "{}", error.1);
    let kept: Endpoint = app.db.get(ENDPOINTS, "chrome-dev").unwrap();
    assert_eq!(kept.grants[0].excluded_tools.len(), 2);
}

#[tokio::test]
async fn endpoints_answer_only_from_the_dashboards_origin() {
    let app = mcp_app();
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome dev"))
        .unwrap();
    app.db
        .put(
            ENDPOINTS,
            "chrome-dev",
            &endpoint(
                "chrome-dev",
                vec![grant("c1", "chrome-dev", &[Scope::View])],
            ),
        )
        .unwrap();
    let (status, _, _) = call(
        app.clone(),
        "/mcp/missing",
        Some(ADMIN),
        None,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let response = crate::router(app.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp/chrome-dev")
                .header("host", "localhost:8080")
                .header("origin", "https://evil.example")
                .header("authorization", format!("Bearer {ADMIN}"))
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn endpoints_answer_with_their_tokens_while_the_root_mcp_is_off() {
    // ROOT_MCP_ENABLED=false, without an administrator's token.
    let app = app(true);
    assert!(!app.config.root_mcp_enabled);
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome dev"))
        .unwrap();
    let Json(created) = create_endpoint(
        State(app.clone()),
        Json(
            serde_json::from_value(json!({"name":"chrome-dev","grants":[{"container_id":"c1","scopes":["view"],"servers":"all"}]}))
                .unwrap(),
        ),
    )
    .await
    .unwrap();
    let token = created["token"].as_str().unwrap().to_owned();
    open_session(&app, "/mcp/chrome-dev", &token).await;
    let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}});
    // The endpoint still needs its token, and the empty administrator's token opens nothing.
    for bearer in [None, Some(""), Some(ADMIN)] {
        let (status, _, _) = call(
            app.clone(),
            "/mcp/chrome-dev",
            bearer,
            None,
            initialize.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{bearer:?}");
    }
    // The root /mcp stays off, also for the endpoint's token.
    let (status, _, _) = call(app.clone(), "/mcp", Some(&token), None, initialize.clone()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // MCP_AUTH_MODE=none opens the endpoints without a token only while /mcp is on: with the root
    // channel off, nothing answers an agent without a token.
    let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}});
    let mut closed = app_with_endpoint_but_root_mcp_off();
    Arc::get_mut(&mut closed).unwrap().config.mcp_token = None;
    let (status, _, body) = call(closed, "/mcp/chrome-dev", None, None, initialize.clone()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    let mut open = app_with_endpoint_but_root_mcp_off();
    let config = &mut Arc::get_mut(&mut open).unwrap().config;
    config.mcp_token = None;
    config.root_mcp_enabled = true;
    let (status, _, body) = call(open, "/mcp/chrome-dev", None, None, initialize).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["result"]["serverInfo"].is_object(), "{body}");
}
fn app_with_endpoint_but_root_mcp_off() -> Arc<App> {
    let app = app(true);
    app.db
        .put(
            ENDPOINTS,
            "chrome-dev",
            &endpoint("chrome-dev", vec![grant("c1", "chrome-dev", &[])]),
        )
        .unwrap();
    app
}

#[tokio::test]
async fn get_job_reads_only_the_jobs_the_endpoint_started_in_its_containers() {
    let app = mcp_app();
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome dev"))
        .unwrap();
    for (id, resource) in [
        ("mine", "container:c1"),
        ("administrators", "container:c1"),
        ("theirs", "container:c2"),
    ] {
        app.db
            .put(
                "job",
                id,
                &crate::model::Job {
                    id: id.into(),
                    kind: "exec".into(),
                    resource: Some(resource.into()),
                    status: "completed".into(),
                    created_at: 1,
                    finished_at: Some(2),
                    log: String::new(),
                    result: None,
                    progress: None,
                    stages: Vec::new(),
                    script: None,
                    name: None,
                },
            )
            .unwrap();
    }
    app.db
        .put(
            ENDPOINTS,
            "chrome-dev",
            &endpoint(
                "chrome-dev",
                vec![grant("c1", "chrome-dev", &[Scope::Power])],
            ),
        )
        .unwrap();
    // The endpoint started "mine" and, before its grant on c2 was taken back, "theirs"; the
    // administrator's root script in c1 is not the endpoint's to read.
    app.desktop_mcp.started("chrome-dev", "mine");
    app.desktop_mcp.started("chrome-dev", "theirs");
    app.desktop_mcp.started("other", "administrators");
    let session = open_session(&app, "/mcp/chrome-dev", ADMIN).await;
    let read = |job: &'static str| {
        call(
            app.clone(),
            "/mcp/chrome-dev",
            Some(ADMIN),
            Some(&session),
            json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"get_job","arguments":{"job_id":job}}}),
        )
    };
    let (_, _, body) = read("mine").await;
    assert_eq!(body["result"]["structuredContent"]["id"], "mine", "{body}");
    for job in ["administrators", "theirs"] {
        let (_, _, body) = read(job).await;
        assert_eq!(body["result"]["isError"], true, "{job}: {body}");
    }
    // A job the endpoint's own call starts is its to read.
    let (_, _, body) = call(
        app.clone(),
        "/mcp/chrome-dev",
        Some(ADMIN),
        Some(&session),
        json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"container_action","arguments":{"action":"stop"}}}),
    )
    .await;
    let started = body["result"]["structuredContent"]["job_id"]
        .as_str()
        .unwrap_or_else(|| panic!("{body}"))
        .to_owned();
    let (_, _, body) = call(
        app.clone(),
        "/mcp/chrome-dev",
        Some(ADMIN),
        Some(&session),
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"get_job","arguments":{"job_id":started}}}),
    )
    .await;
    assert_eq!(body["result"]["structuredContent"]["id"], started, "{body}");
}

#[test]
fn every_route_and_tool_is_documented() {
    let api = include_str!("../../../docs/API.md");
    for route in [
        "`/api/mcp/servers`",
        "`/api/mcp/scopes`",
        "`/api/mcp/inspect`",
        "`/api/mcp/endpoints`",
        "`/api/mcp/endpoints/{name}`",
        "`/api/mcp/endpoints/{name}/token`",
        "`/api/mcp/endpoints/{name}/tools`",
        "`/api/containers/{id}/mcp/servers`",
        "`/api/containers/{id}/mcp/servers/{name}`",
        "`/api/containers/{id}/mcp/servers/{name}/refresh`",
        "`/mcp/{name}`",
    ] {
        assert!(api.contains(route), "docs/API.md must list {route}");
    }
    let guide = include_str!("../../../docs/MCP.md");
    for scope in Scope::ALL {
        assert!(
            guide.contains(&format!("`{}`", scope.id())),
            "docs/MCP.md must explain the {} scope",
            scope.id()
        );
    }
}

#[tokio::test]
async fn a_scoped_endpoint_reports_no_host_gpu() {
    let app = mcp_app();
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome dev"))
        .unwrap();
    app.usage.record(
        "c1",
        &crate::usage::stats::Measure {
            cpu: 0.5,
            memory: 1 << 20,
            memory_limit: 0,
            online_cpus: 4,
            disk_read: 0.0,
            disk_write: 0.0,
            net_rx: 0.0,
            net_tx: 0.0,
        },
    );
    let Json(created) = create_endpoint(
        State(app.clone()),
        Json(
            serde_json::from_value(json!({"name":"chrome-dev","grants":[{"container_id":"c1","scopes":["view"],"servers":"all"}]}))
                .unwrap(),
        ),
    )
    .await
    .unwrap();
    let token = created["token"].as_str().unwrap().to_owned();
    let session = open_session(&app, "/mcp/chrome-dev", &token).await;
    let (status, _, body) = call(
        app.clone(),
        "/mcp/chrome-dev",
        Some(&token),
        Some(&session),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_usage","arguments":{}}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result = &body["result"];
    assert_ne!(result["isError"], true, "{body}");
    let usage = &result["structuredContent"];
    assert!(usage["containers"].is_array(), "{body}");
    // The host GPU is outside what the grant of one container covers, in both forms.
    assert!(usage.get("gpu").is_none(), "{body}");
    assert!(
        !serde_json::to_string(&result["content"])
            .unwrap()
            .contains("\"gpu\"")
    );
}

#[test]
fn two_tools_that_clean_to_one_name_are_both_listed() {
    let app = app(false);
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome dev"))
        .unwrap();
    server(&app, "c1", "s", &["a.b", "a_b", "plain"]);
    let one = endpoint("one", vec![grant("c1", "chrome-dev", &[])]);
    let offered = names(&app, &one);
    assert!(offered.contains(&"s__a_b".to_owned()), "{offered:?}");
    assert!(offered.contains(&"s__plain".to_owned()), "{offered:?}");
    let hashed = exposed_name_hashed(&["s", "a_b"]);
    assert!(hashed.starts_with("s__a_b_") && hashed.len() == "s__a_b_".len() + 6);
    assert!(offered.contains(&hashed), "{offered:?}");
    assert_eq!(offered.len(), 3);
    // The hash tail comes from the raw parts, so it does not change with the clean name.
    assert_ne!(exposed_name_hashed(&["s", "a.b"]), hashed);
}

#[tokio::test]
async fn an_endpoints_existence_is_told_only_to_a_caller_with_a_token() {
    let app = mcp_app();
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome dev"))
        .unwrap();
    app.db
        .put(
            ENDPOINTS,
            "chrome-dev",
            &endpoint(
                "chrome-dev",
                vec![grant("c1", "chrome-dev", &[Scope::View])],
            ),
        )
        .unwrap();
    let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}});
    // Without a token, a name that exists and one that does not answer the same.
    for name in ["/mcp/chrome-dev", "/mcp/missing"] {
        let (status, _, _) = call(app.clone(), name, None, None, initialize.clone()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{name}");
        let (status, _, _) = call(
            app.clone(),
            name,
            Some("not-a-token"),
            None,
            initialize.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{name}");
    }
    let (status, _, _) = call(app.clone(), "/mcp/missing", Some(ADMIN), None, initialize).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[test]
fn a_tool_list_an_endpoint_cannot_carry_is_refused() {
    let names: Vec<String> = (0..MAX_TOOLS).map(|i| format!("tool_{i}")).collect();
    let tools: Vec<rmcp::model::Tool> = names.iter().map(|name| tool(name)).collect();
    assert!(check_tool_list(&tools).is_ok());
    let mut one_more = tools.clone();
    one_more.push(tool("tool_more"));
    let error = check_tool_list(&one_more).unwrap_err().to_string();
    assert!(error.contains("more than 256 tools"), "{error}");
    // Few tools, but described at a length no endpoint should send with every list.
    let mut wide = tool("wide");
    wide.description = Some("x".repeat(1 << 20).into());
    let error = check_tool_list(&[wide]).unwrap_err().to_string();
    assert!(error.contains("larger than 1 MiB"), "{error}");
}

#[tokio::test]
async fn request_human_comes_with_input_and_asks_only_for_the_endpoints_container() {
    let app = mcp_app();
    for (id, name) in [("c1", "Chrome dev"), ("c2", "Unity")] {
        app.db.put(CONTAINERS, id, &container(id, name)).unwrap();
    }
    let viewer = endpoint("viewer", vec![grant("c1", "chrome-dev", &[Scope::View])]);
    assert!(!names(&app, &viewer).contains(&"request_human".to_owned()));
    let driver = endpoint(
        "chrome-dev",
        vec![grant("c1", "chrome-dev", &[Scope::Input])],
    );
    let entry = catalog(&app, &driver)
        .into_iter()
        .find(|entry| entry.tool.name == "request_human")
        .expect("input offers request_human");
    assert!(
        !entry.tool.input_schema["properties"]
            .as_object()
            .unwrap()
            .contains_key("container_id")
    );
    let annotations = entry.tool.annotations.unwrap();
    assert_eq!(annotations.read_only_hint, Some(false));
    assert_eq!(annotations.destructive_hint, Some(false));
    app.db.put(ENDPOINTS, "chrome-dev", &driver).unwrap();
    let session = open_session(&app, "/mcp/chrome-dev", ADMIN).await;
    let ask = |arguments: Value| {
        call(
            app.clone(),
            "/mcp/chrome-dev",
            Some(ADMIN),
            Some(&session),
            json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"request_human","arguments":arguments}}),
        )
    };
    let (_, _, body) = ask(json!({"message": "Solve the CAPTCHA", "wait_seconds": 0})).await;
    let asked = &body["result"]["structuredContent"];
    assert_eq!(asked["status"], "pending", "{body}");
    let pending = app.human.pending();
    assert_eq!(pending[0].container_id, "c1");
    assert_eq!(pending[0].endpoint.as_deref(), Some("chrome-dev"));
    let (_, _, body) = ask(json!({"request_id": asked["request_id"], "wait_seconds": 0})).await;
    assert_eq!(
        body["result"]["structuredContent"]["status"], "pending",
        "{body}"
    );
    // Another container's request is not this endpoint's to wait on, nor its container_id.
    let other = app
        .human
        .ask(
            &app.container("c2").unwrap(),
            "Accept the license",
            None,
            &app.events,
        )
        .unwrap();
    let (_, _, body) = ask(json!({"request_id": other.id, "wait_seconds": 0})).await;
    assert_eq!(body["result"]["isError"], true, "{body}");
    let (_, _, body) = ask(json!({"container_id": "c2", "message": "Elsewhere"})).await;
    assert_eq!(body["result"]["isError"], true, "{body}");
    assert_eq!(
        body["result"]["content"][0]["text"],
        "Unknown argument: container_id"
    );
}

#[test]
fn a_relayed_result_carries_the_result_type_a_newer_client_requires() {
    // A server of the 2025 revisions answers without resultType; a 2026-07-28 client refuses a
    // result without it, so the endpoint says what the absence means.
    let older: rmcp::model::CallToolResult =
        serde_json::from_value(json!({"content":[{"type":"text","text":"ok"}]})).unwrap();
    assert!(older.result_type.is_none());
    let relayed_result = relayed(older);
    assert_eq!(
        serde_json::to_value(&relayed_result).unwrap()["resultType"],
        "complete"
    );
    // A result that says so already is passed on as it is.
    let newer: rmcp::model::CallToolResult =
        serde_json::from_value(json!({"resultType":"complete","content":[]})).unwrap();
    assert_eq!(relayed(newer.clone()), newer);
}

#[tokio::test]
async fn an_endpoints_calls_are_recorded_with_the_endpoint_and_a_servers_arguments_by_size() {
    let app = mcp_app();
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome dev"))
        .unwrap();
    server(&app, "c1", "chrome", &["browser_navigate"]);
    let Json(created) = create_endpoint(
        State(app.clone()),
        Json(
            serde_json::from_value(json!({"name":"chrome-dev","grants":[{"container_id":"c1","scopes":["view"],"servers":"all"}]}))
                .unwrap(),
        ),
    )
    .await
    .unwrap();
    let token = created["token"].as_str().unwrap().to_owned();
    let session = open_session(&app, "/mcp/chrome-dev", &token).await;
    let tool_call = |id: u64, name: &str, arguments: Value| json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}});
    for message in [
        tool_call(2, "get_usage", json!({})),
        tool_call(
            3,
            "chrome__browser_navigate",
            json!({"url": "https://example.com/?key=secret-value-123"}),
        ),
        tool_call(4, "delete_container", json!({})),
    ] {
        call(
            app.clone(),
            "/mcp/chrome-dev",
            Some(&token),
            Some(&session),
            message,
        )
        .await;
    }
    // The administrator's token opens the endpoint too, and says so.
    let admin = open_session(&app, "/mcp/chrome-dev", ADMIN).await;
    call(
        app.clone(),
        "/mcp/chrome-dev",
        Some(ADMIN),
        Some(&admin),
        tool_call(5, "list_windows", json!({})),
    )
    .await;
    let rows = crate::activity::recorded(&app, 4).await;
    let row = |tool: &str| {
        rows.iter()
            .find(|row| row.tool == tool)
            .unwrap_or_else(|| panic!("{tool}: {rows:?}"))
    };

    let usage = row("get_usage");
    assert_eq!(usage.endpoint.as_deref(), Some("chrome-dev"));
    assert_eq!(usage.credential, "endpoint_token");
    // The endpoint binds its one container, which the call does not name.
    assert_eq!(usage.container_id.as_deref(), Some("c1"));
    assert_eq!(usage.container_name.as_deref(), Some("Chrome dev"));
    assert_eq!(
        (usage.source.as_str(), usage.kind.as_str()),
        ("tool", "read")
    );

    // A server's tool by its own name, its arguments as their size whatever it says of itself.
    let navigate = row("browser_navigate");
    assert_eq!(navigate.source, "server");
    assert_eq!(navigate.server.as_deref(), Some("chrome"));
    assert_eq!(navigate.container_id.as_deref(), Some("c1"));
    assert_eq!(
        navigate.kind, "change",
        "a server's read-only claim is not trusted"
    );
    assert_eq!(
        navigate.arguments,
        json!({"url": {"$masked": "string", "length": 41}})
    );
    assert!(
        !serde_json::to_string(navigate)
            .unwrap()
            .contains("secret-value-123")
    );

    // A tool outside the endpoint's scopes is refused, and shown as what it would have been.
    let refused = row("delete_container");
    assert_eq!(
        (refused.outcome.as_str(), refused.kind.as_str()),
        ("refused", "destructive")
    );

    assert_eq!(row("list_windows").credential, "mcp_token");
}

/// A server that finishes its handshake and never answers a call leaves the pool once a call gave
/// up on it, so the next call connects anew instead of waiting as long again.
#[cfg(unix)]
#[tokio::test]
async fn a_server_that_hung_is_not_handed_to_the_next_call() {
    let mut app = app(false);
    let dir = std::env::temp_dir().join(format!("thoric-hung-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let docker = dir.join("docker");
    crate::test_support::write_script(
        &docker,
        r#"#!/bin/sh
case "$1 $2" in
  'container inspect') echo '[{"Id":"c0ffee","Config":{"Labels":{"thoric.managed":"true","thoric.environment":"e"}},"State":{"Running":true}}]' ;;
  'exec -i')
    echo started >> "$(dirname "$0")/connections"
    while IFS= read -r line; do
      case "$line" in
        *'"initialize"'*)
          id=$(printf '%s' "$line" | sed 's/.*"id":\([0-9]*\).*/\1/')
          printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"hung","version":"1"}}}\n' "$id" ;;
      esac
    done ;;
  *) exit 1 ;;
esac
"#,
    );
    Arc::get_mut(&mut app).unwrap().config.docker_bin = docker.display().to_string();
    let env = container("e", "Work");
    let record = ServerRecord {
        container_id: "e".into(),
        name: "hung".into(),
        source: Source::Image,
        spec: Some(stdio("hung")),
        tools: Vec::new(),
        tools_at: None,
        error: None,
    };
    for _ in 0..2 {
        let error = super::client::call_within(
            &app,
            "agents",
            &env,
            &record,
            "wait",
            Default::default(),
            std::time::Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert!(
            error.to_string().contains("did not answer in 1 seconds"),
            "{error:#}"
        );
    }
    let started = std::fs::read_to_string(dir.join("connections")).unwrap();
    assert_eq!(
        started.lines().count(),
        2,
        "each call started its own server"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// /mcp could not call a desktop server's tool, so an administrator's agent had to create an
/// endpoint first. call_mcp_server_tool reaches it by its own name; a server or a tool that is not
/// known is refused with where to look, and the log keeps the tool's arguments by name and size.
#[tokio::test]
async fn the_administrator_calls_a_desktop_servers_tool_by_its_own_name() {
    let app = mcp_app();
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome dev"))
        .unwrap();
    server(&app, "c1", "chrome", &["browser_navigate"]);
    let call = |arguments: Value| {
        crate::test_support::mcp_request(
            app.clone(),
            "tools/call",
            json!({"name": "call_mcp_server_tool", "arguments": arguments}),
        )
    };
    for (arguments, refusal) in [
        (
            json!({"container_id": "c1", "server": "notes", "tool": "read"}),
            "Chrome dev has no MCP server called notes",
        ),
        (
            json!({"container_id": "c1", "server": "chrome", "tool": "browser_click"}),
            "The server chrome has no tool called browser_click",
        ),
        (
            json!({"container_id": "c1", "server": "chrome", "tool": "browser_navigate", "arguments": "https://example.com"}),
            "arguments must be an object",
        ),
    ] {
        let (_, body) = call(arguments).await;
        assert_eq!(body["result"]["isError"], true, "{body}");
        let text = body["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains(refusal), "{body}");
    }
    // A known tool is called through docker exec, which this test has no Docker for.
    let (_, body) = call(json!({"container_id": "c1", "server": "chrome", "tool": "browser_navigate", "arguments": {"url": "https://example.com/?key=secret-value-123"}})).await;
    assert_eq!(body["result"]["isError"], true, "{body}");
    let rows = crate::activity::recorded(&app, 4).await;
    let called = rows
        .iter()
        .find(|row| row.arguments["tool"] == "browser_navigate")
        .unwrap_or_else(|| panic!("{rows:?}"));
    assert_eq!(called.tool, "call_mcp_server_tool");
    assert_eq!(called.container_id.as_deref(), Some("c1"));
    assert_eq!(
        called.arguments["arguments"],
        json!({"url": {"$masked": "string", "length": 41}})
    );
    assert!(
        !serde_json::to_string(&rows)
            .unwrap()
            .contains("secret-value-123")
    );
}

#[test]
fn one_endpoints_jobs_never_push_out_anothers() {
    let mcp = DesktopMcp::default();
    mcp.started("quiet", "theirs");
    for number in 0..1000 {
        mcp.started("busy", &format!("job-{number}"));
    }
    assert!(mcp.started_by("quiet", "theirs"));
    assert!(mcp.started_by("busy", "job-999"));
    assert!(!mcp.started_by("busy", "job-0"));
    assert!(!mcp.started_by("busy", "theirs"));
}

#[tokio::test]
async fn a_deleted_endpoint_leaves_nothing_to_one_made_later_with_its_name() {
    let app = mcp_app();
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome dev"))
        .unwrap();
    app.db
        .put(
            ENDPOINTS,
            "dev",
            &endpoint("dev", vec![grant("c1", "dev", &[Scope::Commands])]),
        )
        .unwrap();
    app.desktop_mcp.started("dev", "job-1");
    let terminal = crate::model::Terminal {
        id: "t1".into(),
        container_id: "c1".into(),
        root: false,
        cwd: "/home/desktop".into(),
        created_at: 1,
        endpoint: Some("dev".into()),
    };
    app.db.put("terminal", "t1", &terminal).unwrap();
    let Json(deleted) = super::endpoints::delete_endpoint(State(app.clone()), Path("dev".into()))
        .await
        .unwrap();
    assert_eq!(deleted["deleted"], true);
    assert!(!app.desktop_mcp.started_by("dev", "job-1"));
    // The terminal is the administrator's alone now.
    let kept: crate::model::Terminal = app.db.get("terminal", "t1").unwrap();
    assert_eq!(kept.endpoint, None);
}

#[test]
fn the_desktop_users_declarations_name_no_secret_value() {
    let app = app(false);
    let pinned: crate::compose::Pinned = serde_json::from_value(json!({
        "compose": {
            "parameters": [{"name": "EMAIL"}, {"name": "API_TOKEN", "secret": true}],
            "script": "echo set up",
        },
        "values": {"EMAIL": "a@example.com", "API_TOKEN": "s3cret"},
    }))
    .unwrap();
    app.save_pinned("c1", &pinned).unwrap();
    let spec: ServerSpec = serde_json::from_value(json!({
        "command": "server",
        "args": ["--user", "${EMAIL}", "--token", "${API_TOKEN}"],
    }))
    .unwrap();
    // The image's folder, which only a build writes, gets every value.
    let image = spec.resolve(&Source::Image.values(&app, "c1")).unwrap();
    assert_eq!(image.args[3], "s3cret");
    // The desktop user writes their own folder, and their servers run as them.
    let error = spec
        .resolve(&Source::Container.values(&app, "c1"))
        .unwrap_err();
    assert!(format!("{error:#}").contains("API_TOKEN"), "{error:#}");
    let public: ServerSpec =
        serde_json::from_value(json!({"command": "server", "args": ["${EMAIL}"]})).unwrap();
    let resolved = public
        .resolve(&Source::Container.values(&app, "c1"))
        .unwrap();
    assert_eq!(resolved.args, ["a@example.com"]);
}

#[tokio::test]
async fn a_containers_servers_are_read_by_its_own_ids_alone() {
    let app = app(false);
    app.db
        .put(CONTAINERS, "a", &container("a", "Zeta"))
        .unwrap();
    app.db
        .put(CONTAINERS, "ab", &container("ab", "Alpha"))
        .unwrap();
    server(&app, "a", "web", &["open"]);
    server(&app, "ab", "unity", &["run_tests"]);
    server(&app, "a", "chrome", &["browser_navigate"]);
    // A container the manager no longer has, and a record of `a` this release cannot read.
    server(&app, "gone", "old", &["old_tool"]);
    app.db
        .put(
            SERVERS,
            &ServerRecord::key("a", "broken"),
            &json!({"name": 7}),
        )
        .unwrap();
    let names = |records: Vec<ServerRecord>| {
        records
            .into_iter()
            .map(|record| record.name)
            .collect::<Vec<_>>()
    };
    // `a` is the start of `ab`, but not of its ids, which go on with the colon.
    assert_eq!(names(servers::records(&app, "a")), ["chrome", "web"]);
    assert_eq!(names(servers::records(&app, "ab")), ["unity"]);
    assert!(servers::records(&app, "b").is_empty());
    // The dashboard's list reads them all once: by container, then by server, and only those of
    // the containers the manager has.
    let Json(all) = all_servers(State(app.clone())).await.unwrap();
    let listed = all["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| format!("{}/{}", item["container_name"], item["name"]))
        .collect::<Vec<_>>();
    assert_eq!(
        listed,
        [
            r#""Alpha"/"unity""#,
            r#""Zeta"/"chrome""#,
            r#""Zeta"/"web""#
        ]
    );
}

/// A scale set `re` of `instances`, numbers with their containers, with the shared volume
/// `analysis` and a folder each instance gets of its own.
fn scale_set(app: &App, instances: &[(u32, &str)], max: u32) {
    let mut spec = crate::test_support::spec();
    spec.mounts = vec![
        crate::model::Mount {
            volume: "analysis".into(),
            target: "/home/desktop/analysis".into(),
            readonly: false,
        },
        crate::model::Mount {
            volume: "profile".into(),
            target: "/home/desktop/.config/chrome-agent".into(),
            readonly: false,
        },
    ];
    let set = json!({
        "name": "re", "created_at": 1, "template": spec, "min": 1, "max": max,
        "volumes": {"/home/desktop/.config/chrome-agent": "new"},
        "instances": instances.iter().map(|(number, id)| (number.to_string(), json!({"container_id": id}))).collect::<serde_json::Map<_, _>>(),
    });
    app.db.put(crate::scale::SETS, "re", &set).unwrap();
}

fn set_grant(scopes: &[Scope]) -> Grant {
    Grant {
        container_id: String::new(),
        set: Some("re".into()),
        alias: "re".into(),
        scopes: scopes.to_vec(),
        servers: Servers::All,
        excluded_tools: Vec::new(),
    }
}

#[test]
fn an_endpoint_of_a_scale_set_takes_instance_and_offers_each_server_tool_once() {
    let app = app(false);
    for (id, name) in [("c1", "re-1"), ("c2", "re-2")] {
        app.db.put(CONTAINERS, id, &container(id, name)).unwrap();
        server(&app, id, "ghidra", &["decompile"]);
    }
    // A server's tool with an `instance` of its own takes the instance by another name.
    let mut own = tool("start");
    own.input_schema = Arc::new(
        json!({"type":"object","properties":{"instance":{"type":"string"}},"required":["instance"]})
            .as_object()
            .unwrap()
            .clone(),
    );
    let record = ServerRecord {
        container_id: "c1".into(),
        name: "qemu".into(),
        source: Source::Image,
        spec: Some(stdio("qemu-mcp")),
        tools: vec![own],
        tools_at: Some(1),
        error: None,
    };
    app.db
        .put(SERVERS, &ServerRecord::key("c1", "qemu"), &record)
        .unwrap();
    scale_set(&app, &[(1, "c1"), (2, "c2")], 4);
    let set = endpoint(
        "re",
        vec![set_grant(&[Scope::View, Scope::Power, Scope::Scale])],
    );
    let entries = catalog(&app, &set);
    let offered = names(&app, &set);
    for name in [
        "list_instances",
        "screenshot",
        "container_action",
        "create_instance",
        "delete_instance",
        "get_job",
        "ghidra__decompile",
        "qemu__start",
    ] {
        assert!(offered.contains(&name.to_owned()), "{name}: {offered:?}");
    }
    assert_eq!(
        offered
            .iter()
            .filter(|name| name.contains("decompile"))
            .count(),
        1,
        "{offered:?}"
    );
    let schema = |name: &str| {
        let entry = entries.iter().find(|e| e.tool.name == name).unwrap();
        Value::Object((*entry.tool.input_schema).clone())
    };
    let screenshot = schema("screenshot");
    assert_eq!(screenshot["properties"]["instance"]["type"], "integer");
    assert!(
        screenshot["required"]
            .as_array()
            .unwrap()
            .contains(&json!("instance"))
    );
    assert!(screenshot["properties"].get("container_id").is_none());
    assert!(screenshot["properties"].get("container").is_none());
    let create = schema("create_instance");
    assert!(create["properties"].get("set").is_none());
    assert!(
        !create["required"]
            .as_array()
            .unwrap()
            .contains(&json!("set"))
    );
    let decompile = schema("ghidra__decompile");
    assert!(
        decompile["required"]
            .as_array()
            .unwrap()
            .contains(&json!("instance"))
    );
    let start = schema("qemu__start");
    assert_eq!(start["properties"]["instance"]["type"], "string");
    assert_eq!(start["properties"]["thoric_instance"]["type"], "integer");
    let started = entries
        .iter()
        .find(|e| e.tool.name == "qemu__start")
        .unwrap();
    assert_eq!(
        started.target,
        Target::InstanceServer {
            set: "re".into(),
            server: "qemu".into(),
            tool: "start".into(),
            argument: "thoric_instance",
        }
    );
    // Without scale, an agent lists the instances and creates none.
    let viewer = endpoint("re-view", vec![set_grant(&[Scope::View])]);
    let offered = names(&app, &viewer);
    assert!(offered.contains(&"list_instances".to_owned()));
    for absent in [
        "create_instance",
        "delete_instance",
        "get_job",
        "create_container",
    ] {
        assert!(!offered.contains(&absent.to_owned()), "{absent}");
    }
}

#[test]
fn the_scale_scope_is_for_an_endpoint_of_a_scale_set_alone() {
    let app = app(false);
    app.db
        .put(CONTAINERS, "c1", &container("c1", "re-1"))
        .unwrap();
    scale_set(&app, &[(1, "c1")], 4);
    let request = |container: &str, set: Option<&str>, scopes: &[Scope]| GrantRequest {
        container_id: container.into(),
        set: set.map(str::to_owned),
        alias: None,
        scopes: scopes.to_vec(),
        servers: Servers::All,
        excluded_tools: vec!["list_instances".into()],
    };
    let made = grants(
        &app,
        vec![request("", Some("re"), &[Scope::View, Scope::Scale])],
    )
    .unwrap();
    assert_eq!(made[0].set.as_deref(), Some("re"));
    assert_eq!(made[0].container_id, "");
    assert_eq!(made[0].excluded_tools, ["list_instances"]);
    let refused = |requests: Vec<GrantRequest>| grants(&app, requests).unwrap_err().to_string();
    let mut container_grant = request("c1", None, &[Scope::View, Scope::Scale]);
    container_grant.excluded_tools.clear();
    assert!(refused(vec![container_grant.clone()]).contains("scale scope"));
    container_grant.scopes = vec![Scope::View];
    assert!(
        refused(vec![
            request("", Some("re"), &[Scope::View]),
            container_grant
        ])
        .contains("nothing else")
    );
    assert!(refused(vec![request("c1", Some("re"), &[Scope::View])]).contains("not both"));
    assert!(refused(vec![request("", Some("gone"), &[Scope::View])]).contains("not found"));
    // A container's endpoint never offers a set's tools, whatever it was granted.
    let everything = endpoint("all", vec![grant("c1", "re-1", &Scope::ALL)]);
    let offered = names(&app, &everything);
    for absent in ["create_instance", "delete_instance", "list_instances"] {
        assert!(!offered.contains(&absent.to_owned()), "{absent}");
    }
}

#[tokio::test]
async fn a_set_endpoint_works_on_the_instance_its_number_names() {
    let app = mcp_app();
    for (id, name) in [("c1", "re-1"), ("c2", "re-2")] {
        app.db.put(CONTAINERS, id, &container(id, name)).unwrap();
    }
    scale_set(&app, &[(1, "c1"), (2, "c2")], 2);
    app.db
        .put(
            ENDPOINTS,
            "re",
            &endpoint("re", vec![set_grant(&[Scope::View, Scope::Scale])]),
        )
        .unwrap();
    let (status, session, body) = call(
        app.clone(),
        "/mcp/re",
        Some(ADMIN),
        None,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The agent is told what its instances share and what each has of its own.
    let instructions = body["result"]["instructions"].as_str().unwrap();
    assert!(
        instructions.contains("/home/desktop/analysis")
            && instructions.contains("each instance's own: /home/desktop/.config/chrome-agent")
            && instructions.contains("create_instance"),
        "{instructions}"
    );
    let session = session.unwrap();
    let (status, _, _) = call(
        app.clone(),
        "/mcp/re",
        Some(ADMIN),
        Some(&session),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let tool = |id: u32, name: &str, arguments: Value| {
        call(
            app.clone(),
            "/mcp/re",
            Some(ADMIN),
            Some(&session),
            json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}}),
        )
    };
    let (_, _, body) = tool(2, "get_container", json!({"instance": 2})).await;
    assert_eq!(body["result"]["structuredContent"]["id"], "c2", "{body}");
    let (_, _, body) = tool(3, "get_container", json!({"instance": 9})).await;
    assert_eq!(body["result"]["isError"], true, "{body}");
    assert!(
        body["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("no instance 9")
    );
    // The set is the endpoint's own: another one cannot be named.
    let (_, _, body) = tool(4, "list_instances", json!({"set": "other"})).await;
    assert_eq!(body["result"]["isError"], true, "{body}");
    let (_, _, body) = tool(5, "list_instances", json!({})).await;
    let listed = &body["result"]["structuredContent"];
    assert_eq!(listed["name"], "re", "{body}");
    assert_eq!(listed["instances"][1]["instance"], 2);
    assert_eq!(listed["free"], 0);
    // The set has its most instances, and the one it keeps is not an agent's to delete.
    let (_, _, body) = tool(6, "create_instance", json!({})).await;
    assert!(
        body["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("most instances"),
        "{body}"
    );
    let (_, _, body) = tool(7, "delete_instance", json!({"instance": 1})).await;
    assert!(
        body["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("keeps"),
        "{body}"
    );
}

/// A set's endpoint serves the scale set skill and names it in its instructions, so an agent that
/// never installed it still reads it before it grows the set; a container's endpoint has no use
/// for it and declares no resources.
#[tokio::test]
async fn a_set_endpoint_serves_the_scale_set_skill_and_a_container_endpoint_none() {
    let app = mcp_app();
    app.db
        .put(CONTAINERS, "c1", &container("c1", "re-1"))
        .unwrap();
    scale_set(&app, &[(1, "c1")], 2);
    app.db
        .put(
            ENDPOINTS,
            "re",
            &endpoint("re", vec![set_grant(&[Scope::View])]),
        )
        .unwrap();
    app.db
        .put(
            ENDPOINTS,
            "one",
            &endpoint("one", vec![grant("c1", "re-1", &[Scope::View])]),
        )
        .unwrap();
    let uri = "skill://thoric-scale-sets/SKILL.md";
    let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}});
    // Opens a session on `path`, answered with it and the result of `initialize`.
    let open = |path: &'static str| {
        let app = app.clone();
        let initialize = initialize.clone();
        async move {
            let (status, session, body) =
                call(app.clone(), path, Some(ADMIN), None, initialize).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let session = session.unwrap();
            let (status, _, _) = call(
                app,
                path,
                Some(ADMIN),
                Some(&session),
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            )
            .await;
            assert_eq!(status, StatusCode::ACCEPTED);
            (session, body)
        }
    };
    let ask = |path: &'static str, session: String, id: u32, method: &str, params: Value| {
        let message = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let app = app.clone();
        async move {
            call(app, path, Some(ADMIN), Some(&session), message)
                .await
                .2
        }
    };

    let (session, body) = open("/mcp/re").await;
    let result = &body["result"];
    assert!(
        result["capabilities"]["extensions"][crate::skills::mcp::EXTENSION].is_object(),
        "{body}"
    );
    assert!(
        result["instructions"].as_str().unwrap().contains(uri),
        "{body}"
    );
    let body = ask(
        "/mcp/re",
        session.clone(),
        2,
        "resources/read",
        json!({"uri": uri}),
    )
    .await;
    assert_eq!(
        body["result"]["contents"][0]["text"],
        crate::skills::find("thoric-scale-sets").unwrap().content,
        "{body}"
    );
    let body = ask("/mcp/re", session, 3, "skills/list", json!({})).await;
    assert_eq!(body["result"]["skills"][0]["uri"], uri, "{body}");

    let (session, body) = open("/mcp/one").await;
    let result = &body["result"];
    assert!(result["capabilities"].get("resources").is_none(), "{body}");
    assert!(result["capabilities"].get("extensions").is_none(), "{body}");
    assert!(
        !result["instructions"].as_str().unwrap().contains(uri),
        "{body}"
    );
    let body = ask(
        "/mcp/one",
        session.clone(),
        2,
        "resources/read",
        json!({"uri": uri}),
    )
    .await;
    assert_eq!(body["error"]["code"], -32602, "{body}");
    let body = ask("/mcp/one", session, 3, "skills/list", json!({})).await;
    assert_eq!(body["error"]["code"], -32601, "{body}");
}

/// Make the set `re` run `server` in instance 1 alone.
fn run_once(app: &App, server: &str) {
    let mut set: crate::scale::ScaleSet = app.db.get(crate::scale::SETS, "re").unwrap();
    set.servers
        .insert(server.into(), crate::scale::ServerPolicy::First);
    app.db.put(crate::scale::SETS, "re", &set).unwrap();
}

#[test]
fn a_server_the_set_runs_in_instance_one_is_offered_from_there_without_instance() {
    let app = app(false);
    for (id, name) in [("c1", "re-1"), ("c2", "re-2")] {
        app.db.put(CONTAINERS, id, &container(id, name)).unwrap();
        server(&app, id, "chrome", &["navigate"]);
    }
    server(&app, "c1", "ghidra", &["decompile"]);
    // Instance 2 read its tools before the set ran the server once; they are not its to offer.
    server(&app, "c2", "ghidra", &["stale"]);
    scale_set(&app, &[(1, "c1"), (2, "c2")], 4);
    run_once(&app, "ghidra");
    let set = endpoint("re", vec![set_grant(&[Scope::View])]);
    let entries = catalog(&app, &set);
    let offered = names(&app, &set);
    assert!(
        offered.contains(&"ghidra__decompile".to_owned()),
        "{offered:?}"
    );
    assert!(
        !offered.contains(&"ghidra__stale".to_owned()),
        "{offered:?}"
    );
    let entry = |name: &str| entries.iter().find(|e| e.tool.name == name).unwrap();
    let decompile = entry("ghidra__decompile");
    assert_eq!(
        decompile.target,
        Target::HomeServer {
            set: "re".into(),
            server: "ghidra".into(),
            tool: "decompile".into(),
            number: 1,
        }
    );
    let schema = Value::Object((*decompile.tool.input_schema).clone());
    assert!(schema["properties"].get("instance").is_none(), "{schema}");
    assert!(schema.get("required").is_none(), "{schema}");
    assert!(
        decompile
            .tool
            .description
            .as_deref()
            .unwrap()
            .starts_with("[re / ghidra, instance 1 for every instance]")
    );
    // A server every instance runs still takes the instance.
    let navigate = Value::Object((*entry("chrome__navigate").tool.input_schema).clone());
    assert_eq!(navigate["properties"]["instance"]["type"], "integer");
    // An endpoint of instance 2 alone does not offer what instance 1 runs for it; one of instance
    // 1 does.
    let second = endpoint("re-2", vec![grant("c2", "re-2", &[Scope::View])]);
    assert!(
        !names(&app, &second)
            .iter()
            .any(|name| name.starts_with("ghidra"))
    );
    let first = endpoint("re-1", vec![grant("c1", "re-1", &[Scope::View])]);
    assert!(names(&app, &first).contains(&"ghidra__decompile".to_owned()));
}

#[tokio::test]
async fn every_instance_reaches_the_server_instance_one_runs_and_no_other_starts_it() {
    let app = mcp_app();
    for (id, name) in [("c1", "re-1"), ("c2", "re-2")] {
        app.db.put(CONTAINERS, id, &container(id, name)).unwrap();
        server(&app, id, "ghidra", &["decompile"]);
    }
    scale_set(&app, &[(1, "c1"), (2, "c2")], 2);
    run_once(&app, "ghidra");
    app.db
        .put(
            ENDPOINTS,
            "re",
            &endpoint("re", vec![set_grant(&[Scope::View])]),
        )
        .unwrap();
    let (_, session, body) = call(
        app.clone(),
        "/mcp/re",
        Some(ADMIN),
        None,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
    )
    .await;
    // The agent is told the server is one program in instance 1 for all of them.
    let instructions = body["result"]["instructions"].as_str().unwrap();
    assert!(
        instructions.contains("run in instance 1 alone") && instructions.contains(": ghidra."),
        "{instructions}"
    );
    let session = session.unwrap();
    call(
        app.clone(),
        "/mcp/re",
        Some(ADMIN),
        Some(&session),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    // The call names no instance and goes to instance 1, which does not run here.
    let (_, _, body) = call(
        app.clone(),
        "/mcp/re",
        Some(ADMIN),
        Some(&session),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ghidra__decompile","arguments":{}}}),
    )
    .await;
    assert_eq!(body["result"]["isError"], true, "{body}");
    let text = body["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("re runs the desktop MCP server ghidra in instance 1 alone")
            && text.contains("instance 1 does not run"),
        "{text}"
    );
    // The administrator's /mcp is sent to instance 1 as well.
    let refused =
        super::servers::call_server_tool(&app, "c2", "ghidra", "decompile", Default::default())
            .await
            .unwrap_err();
    assert_eq!(refused.0, StatusCode::CONFLICT);
    assert!(refused.1.contains("in instance 1 alone"), "{}", refused.1);
    // A read of its tools does not start it in instance 2, while a server instance 2 runs itself
    // is started there, which fails without Docker.
    server(&app, "c2", "chrome", &[]);
    for name in ["ghidra", "chrome"] {
        let key = ServerRecord::key("c2", name);
        let mut record: ServerRecord = app.db.get(SERVERS, &key).unwrap();
        record.tools_at = None;
        app.db.put(SERVERS, &key, &record).unwrap();
        super::watch::fetch(&app, "c2", name).await;
    }
    let read = |name: &str| -> ServerRecord {
        app.db.get(SERVERS, &ServerRecord::key("c2", name)).unwrap()
    };
    assert!(read("ghidra").error.is_none());
    assert!(read("chrome").error.is_some());
    // Nor does the inspector, in either instance.
    for target in ["c1", "c2"] {
        let refused = super::inspect::open(
            &app,
            super::inspect::Target::Server {
                container: target.into(),
                name: "ghidra".into(),
            },
        )
        .await
        .err()
        .unwrap();
        assert_eq!(refused.0, StatusCode::CONFLICT, "{}", refused.1);
    }
    // Lists and the desktop show where it runs.
    let sets = crate::scale::all(&app).unwrap();
    let listed = super::servers::summary(&app, &read("ghidra"), "re-2", true, &sets);
    assert_eq!(listed["state"], "elsewhere");
    assert_eq!(listed["runs_in"], 1);
    let report = super::report::report(&app, "c2");
    let ghidra = report["servers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|server| server["name"] == "ghidra")
        .unwrap();
    assert_eq!(ghidra["state"], "elsewhere");
    assert_eq!(ghidra["runs_in"], 1);
}

#[tokio::test]
async fn a_server_the_set_starts_running_once_leaves_the_other_instances_and_comes_back() {
    let app = app(false);
    for (id, name) in [("c1", "re-1"), ("c2", "re-2")] {
        app.db.put(CONTAINERS, id, &container(id, name)).unwrap();
        server(&app, id, "ghidra", &["decompile"]);
    }
    scale_set(&app, &[(1, "c1"), (2, "c2")], 2);
    let before: crate::scale::ScaleSet = app.db.get(crate::scale::SETS, "re").unwrap();
    run_once(&app, "ghidra");
    let after: crate::scale::ScaleSet = app.db.get(crate::scale::SETS, "re").unwrap();
    servers_changed(&app, &before.servers, &after);
    let read = |id: &str| -> ServerRecord {
        app.db
            .get(SERVERS, &ServerRecord::key(id, "ghidra"))
            .unwrap()
    };
    assert!(read("c2").tools.is_empty() && read("c2").tools_at.is_none());
    assert_eq!(read("c1").tools.len(), 1);
    // A listing of instance 2's servers keeps it without tools and reads none.
    super::watch::apply_with(
        &app,
        "c2",
        vec![super::registry::Found {
            name: "ghidra".into(),
            source: Source::Image,
            spec: Ok(stdio("playwright-mcp")),
        }],
        true,
    )
    .await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(read("c2").tools_at.is_none() && read("c2").error.is_none());
    // Back to every instance: instance 2 is no longer refused.
    app.db.put(crate::scale::SETS, "re", &before).unwrap();
    servers_changed(&app, &after.servers, &before);
    assert!(
        super::placement::placement(&app, "c2", "ghidra")
            .refusal("ghidra")
            .is_none()
    );
}

#[tokio::test]
async fn a_set_endpoint_reads_the_job_of_an_instance_that_is_gone_since() {
    let app = mcp_app();
    app.db
        .put(CONTAINERS, "c1", &container("c1", "re-1"))
        .unwrap();
    scale_set(&app, &[(1, "c1")], 4);
    app.db
        .put(
            ENDPOINTS,
            "re",
            &endpoint("re", vec![set_grant(&[Scope::View, Scope::Scale])]),
        )
        .unwrap();
    // A creation the endpoint started that failed, and a deletion that finished: neither
    // container is an instance now, and both are what the agent waits for.
    for (id, resource, kind) in [
        (
            "creation",
            "container:c2",
            crate::scale::instances::SCALE_UP,
        ),
        (
            "deletion",
            "container:c3",
            crate::scale::instances::SCALE_DOWN,
        ),
        ("others", "container:c9", "exec"),
    ] {
        app.db
            .put(
                "job",
                id,
                &crate::model::Job {
                    id: id.into(),
                    kind: kind.into(),
                    resource: Some(resource.into()),
                    status: "failed".into(),
                    created_at: 1,
                    finished_at: Some(2),
                    log: String::new(),
                    result: None,
                    progress: None,
                    stages: Vec::new(),
                    script: None,
                    name: None,
                },
            )
            .unwrap();
    }
    app.desktop_mcp.started("re", "creation");
    app.desktop_mcp.started("re", "deletion");
    app.desktop_mcp.started("other", "others");
    let session = open_session(&app, "/mcp/re", ADMIN).await;
    let read = |job: &'static str| {
        call(
            app.clone(),
            "/mcp/re",
            Some(ADMIN),
            Some(&session),
            json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"get_job","arguments":{"job_id":job}}}),
        )
    };
    for job in ["creation", "deletion"] {
        let (_, _, body) = read(job).await;
        assert_eq!(body["result"]["structuredContent"]["id"], job, "{body}");
    }
    // One another caller started stays out of reach.
    let (_, _, body) = read("others").await;
    assert_eq!(body["result"]["isError"], true, "{body}");
}

#[tokio::test]
async fn without_the_administrators_token_an_endpoints_calls_are_recorded_as_its_tokens() {
    let mut app = app(true);
    let config = &mut Arc::get_mut(&mut app).unwrap().config;
    config.root_mcp_enabled = false;
    config.mcp_token = None;
    app.db
        .put(CONTAINERS, "c1", &container("c1", "Chrome dev"))
        .unwrap();
    let Json(created) = create_endpoint(
        State(app.clone()),
        Json(
            serde_json::from_value(json!({"name":"chrome-dev","grants":[{"container_id":"c1","scopes":["view"],"servers":"all"}]}))
                .unwrap(),
        ),
    )
    .await
    .unwrap();
    let token = created["token"].as_str().unwrap().to_owned();
    let session = open_session(&app, "/mcp/chrome-dev", &token).await;
    call(
        app.clone(),
        "/mcp/chrome-dev",
        Some(&token),
        Some(&session),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_usage","arguments":{}}}),
    )
    .await;
    let rows = crate::activity::recorded(&app, 1).await;
    // The endpoint opened with its own token, which the log says: `none` is for an
    // installation that opens /mcp without one.
    assert_eq!(rows[0].credential, "endpoint_token", "{rows:?}");
}
