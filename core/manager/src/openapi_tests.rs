//! The OpenAPI document against the router it describes: every route `main.rs` registers has an
//! entry with the same methods and every entry is served, every reference resolves, the
//! envelopes it names are those the code sends, and the route answers to the session as the rest
//! of `/api` does.
use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use std::collections::{BTreeMap, BTreeSet};
use tower::ServiceExt;

/// How a path is served: by these methods, or by any (`axum::routing::any`).
#[derive(Debug, PartialEq)]
enum Served {
    Methods(BTreeSet<String>),
    Any,
}

/// Every path the router serves and how, read from the router itself.
///
/// axum names no route it holds, but its Debug output lists each path with its route ID; that is
/// where the paths come from. The methods come from asking each path with a method nobody serves:
/// axum puts the methods the path has in an `Allow` header, also when `protect` refuses the
/// request first as it does here, and no handler runs. A route that takes any method has no such
/// list and hands the request to its handler, which in this fixture refuses it: the root MCP is
/// off and no token or link exists.
async fn served() -> BTreeMap<String, Served> {
    let router = crate::router(crate::test_support::app(true));
    let debug = format!("{router:?}");
    // The fallback's own paths, `/` and its catch-all, come after the routes'.
    let (routes, _) = debug
        .split_once("fallback_router")
        .expect("axum's Debug output has no fallback_router");
    let paths: BTreeSet<String> = regex::Regex::new(r#"RouteId\(\d+\): "([^"]+)""#)
        .unwrap()
        .captures_iter(routes)
        .map(|found| found[1].to_owned())
        .collect();
    assert!(
        paths.contains("/api/containers") && paths.contains("/healthz") && paths.len() > 100,
        "axum's Debug output no longer lists paths as `RouteId(n): \"path\"`; teach served() its new form: {paths:?}"
    );
    let probe = Method::from_bytes(b"PROBE").unwrap();
    let mut served = BTreeMap::new();
    for path in paths {
        let uri = path
            .split('/')
            .map(|segment| {
                if segment.starts_with('{') {
                    "x"
                } else {
                    segment
                }
            })
            .collect::<Vec<_>>()
            .join("/");
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(probe.clone())
                    .uri(&uri)
                    .header("host", "localhost:8080")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let how = match response.headers().get(header::ALLOW) {
            Some(allow) => {
                let mut methods: BTreeSet<String> = allow
                    .to_str()
                    .unwrap()
                    .split(',')
                    .map(|method| method.trim().to_owned())
                    .filter(|method| !method.is_empty())
                    .collect();
                // axum answers HEAD wherever it answers GET.
                if methods.contains("GET") {
                    methods.remove("HEAD");
                }
                Served::Methods(methods)
            }
            _ => Served::Any,
        };
        served.insert(path, how);
    }
    served
}

#[tokio::test]
async fn every_served_route_is_documented_and_every_documented_route_is_served() {
    let served = served().await;
    let mut problems = Vec::new();
    let mut documented: BTreeMap<String, Served> = BTreeMap::new();
    for route in routes::all() {
        let entry = documented.entry(route.path.to_owned()).or_insert_with(|| {
            if route.any {
                Served::Any
            } else {
                Served::Methods(BTreeSet::new())
            }
        });
        match entry {
            Served::Methods(methods) if !route.any => {
                for method in route.methods {
                    if !methods.insert((*method).to_owned()) {
                        problems.push(format!("{method} {} has two entries", route.path));
                    }
                }
            }
            Served::Any if route.any => {}
            _ => problems.push(format!(
                "{} has entries for some methods and for any",
                route.path
            )),
        }
    }
    for (path, how) in &served {
        match documented.get(path) {
            None => problems.push(format!(
                "{path} is served ({how:?}) but has no entry in openapi/routes.rs"
            )),
            Some(entry) if entry != how => problems.push(format!(
                "{path} is served {how:?} but openapi/routes.rs names {entry:?}"
            )),
            Some(_) => {}
        }
    }
    for path in documented.keys().filter(|path| !served.contains_key(*path)) {
        problems.push(format!(
            "{path} has an entry in openapi/routes.rs but the router does not serve it"
        ));
    }
    assert!(
        problems.is_empty(),
        "The OpenAPI document and the router differ:\n{}",
        problems.join("\n")
    );
}

/// Every `$ref` in `value`.
fn references(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get("$ref") {
                found.push(reference.clone());
            }
            for value in map.values() {
                references(value, found);
            }
        }
        Value::Array(items) => {
            for value in items {
                references(value, found);
            }
        }
        _ => {}
    }
}

#[test]
fn every_reference_resolves_and_every_operation_is_named_once() {
    let document = document();
    assert_eq!(document["openapi"], "3.1.0");
    assert_eq!(document["info"]["version"], env!("CARGO_PKG_VERSION"));
    let mut found = Vec::new();
    references(&document, &mut found);
    assert!(found.len() > 50, "the bodies are described by reference");
    for reference in found {
        let pointer = reference
            .strip_prefix('#')
            .unwrap_or_else(|| panic!("{reference} points outside the document"));
        assert!(
            document.pointer(pointer).is_some(),
            "{reference} does not resolve"
        );
    }
    let tags: BTreeSet<&str> = TAGS.iter().map(|(name, _)| *name).collect();
    let mut ids = BTreeSet::new();
    for (path, item) in document["paths"].as_object().unwrap() {
        let named: BTreeSet<&str> = path_parameters(path).map(|(name, _)| name).collect();
        for (method, operation) in item.as_object().unwrap() {
            let id = operation["operationId"].as_str().unwrap();
            assert!(ids.insert(id.to_owned()), "{method} {path}: {id} is taken");
            let declared: BTreeSet<&str> = operation["parameters"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|parameter| parameter["in"] == "path")
                .map(|parameter| parameter["name"].as_str().unwrap())
                .collect();
            assert_eq!(declared, named, "{method} {path} declares its path");
            assert!(
                tags.contains(operation["tags"][0].as_str().unwrap()),
                "{method} {path} has a tag of TAGS"
            );
        }
    }
}

#[test]
fn request_types_reach_the_document_with_their_fields() {
    let document = document();
    let schemas = &document["components"]["schemas"];
    let create = &document["paths"]["/api/containers"]["post"]["requestBody"]["content"]["application/json"]
        ["schema"];
    assert_eq!(create["$ref"], "#/components/schemas/CreateContainer");
    // The spec's fields come through `flatten`, beside the request's own.
    let fields = schemas["CreateContainer"]["properties"]
        .as_object()
        .unwrap();
    for field in [
        "name",
        "image",
        "gpu",
        "mounts",
        "parameters",
        "proxy_password",
    ] {
        assert!(fields.contains_key(field), "CreateContainer lacks {field}");
    }
    // A query string's fields are parameters of their own.
    let download = &document["paths"]["/api/containers/{id}/files/download"]["get"];
    assert!(
        download["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|parameter| parameter["in"] == "query"
                && parameter["name"] == "path"
                && parameter["required"] == true)
    );
    assert_eq!(download["x-thoric-stream"], "download");
    assert_eq!(
        document["paths"]["/api/containers/{id}/vnc"]["get"]["x-thoric-stream"],
        "websocket"
    );
    assert_eq!(
        document["paths"]["/api/events"]["get"]["x-thoric-stream"],
        "event-stream"
    );
    // A change needs the request header, a read does not.
    let header = json!({"$ref": "#/components/parameters/Request"});
    let has_header = |method: &str, path: &str| {
        document["paths"][path][method]["parameters"]
            .as_array()
            .unwrap()
            .contains(&header)
    };
    assert!(has_header("post", "/api/containers"));
    assert!(has_header("post", "/api/login"));
    assert!(!has_header("get", "/api/containers"));
    assert!(!has_header("get", "/api/session"));
}

#[test]
fn a_create_request_names_no_fixed_default_for_what_the_installation_fills() {
    // What `creation_request` fills from the installation: the fields of a request that leaves
    // everything out that come back different with every default on and with every one off.
    let created = |on: bool| {
        let mut app = crate::test_support::app(false);
        let config = &mut std::sync::Arc::get_mut(&mut app).unwrap().config;
        config.defaults = crate::config::Features {
            gpu: on,
            internet: on,
            sudo: on,
            audio: on,
            microphone: on,
            gamepad: on,
            vpn: on,
            idle_stop_minutes: if on { 30 } else { 0 },
        };
        if on {
            config.proxy = crate::outbound::installation(
                &|key| (key == "PROXY_URL").then(|| "http://proxy.example:3128".to_owned()),
                &mut Vec::new(),
            )
            .unwrap();
        }
        let request =
            json!({"name": "Defaults", "image": "ghcr.io/co3moz/thoric/ubuntu-xfce:24.04"});
        let created = crate::api::creation_request(&app.config, request)
            .unwrap_or_else(|error| panic!("{}", error.1));
        serde_json::to_value(created.spec).unwrap()
    };
    let (on, off) = (created(true), created(false));
    let filled: BTreeSet<&str> = on
        .as_object()
        .unwrap()
        .iter()
        .filter(|(field, value)| off[field.as_str()] != **value)
        .map(|(field, _)| field.as_str())
        .collect();
    assert_eq!(
        filled,
        INSTALLATION_DEFAULTS.iter().copied().collect(),
        "INSTALLATION_DEFAULTS must name what creation_request fills"
    );
    let document = document();
    let properties = &document["components"]["schemas"]["CreateContainer"]["properties"];
    for field in INSTALLATION_DEFAULTS {
        assert!(properties[field].is_object(), "CreateContainer has {field}");
        assert!(
            properties[field].get("default").is_none(),
            "{field} names no fixed default"
        );
    }
}

#[test]
fn every_operation_names_the_error_statuses_of_the_guide() {
    // The statuses of the table under `## Errors` in docs/API.md.
    let guide = include_str!("../../../docs/API.md");
    let (_, errors) = guide.split_once("\n## Errors\n").unwrap();
    let (errors, _) = errors.split_once("\n## ").unwrap();
    let table: BTreeSet<&str> = regex::Regex::new(r"(?m)^\| `(\d{3})` \|")
        .unwrap()
        .captures_iter(errors)
        .map(|found| found.get(1).unwrap().as_str())
        .collect();
    let named: BTreeSet<&str> = ERRORS.iter().map(|(status, _, _)| *status).collect();
    assert_eq!(
        named, table,
        "ERRORS must be the statuses docs/API.md lists"
    );
    let document = document();
    for (path, item) in document["paths"].as_object().unwrap() {
        for (method, operation) in item.as_object().unwrap() {
            for status in ["400", "404", "409", "500", "502", "503", "504"] {
                // The shared answer, or one of the route's own such as the 503 of `/readyz`.
                let response = &operation["responses"][status];
                let resolves = match response["$ref"].as_str() {
                    Some(reference) => reference
                        .strip_prefix('#')
                        .and_then(|pointer| document.pointer(pointer))
                        .is_some(),
                    None => response["description"].is_string(),
                };
                assert!(resolves, "{method} {path} names {status}");
            }
        }
    }
}

#[test]
fn every_answer_names_the_request_id_and_readiness_answers_its_own_503() {
    assert!(
        crate::request_log::REQUEST_ID
            .as_str()
            .eq_ignore_ascii_case(REQUEST_ID),
        "the document names the header request_log uses"
    );
    let document = document();
    let resolve = |response: &Value| -> Value {
        match response["$ref"].as_str() {
            Some(reference) => document.pointer(&reference[1..]).unwrap().clone(),
            None => response.clone(),
        }
    };
    let id = json!({"$ref": "#/components/parameters/RequestId"});
    for (path, item) in document["paths"].as_object().unwrap() {
        for (method, operation) in item.as_object().unwrap() {
            assert!(
                operation["parameters"].as_array().unwrap().contains(&id),
                "{method} {path} takes an ID"
            );
            for (status, response) in operation["responses"].as_object().unwrap() {
                assert!(
                    resolve(response)["headers"][REQUEST_ID].is_object(),
                    "{method} {path} {status} carries the ID"
                );
            }
        }
    }
    let ready = &document["paths"]["/readyz"]["get"];
    assert_eq!(ready["security"], json!([]));
    assert!(ready["responses"]["503"]["$ref"].is_null());
    assert!(ready["responses"]["503"]["content"]["application/json"].is_object());
    assert!(ready["responses"]["401"].is_null());
}

#[tokio::test]
async fn errors_and_queued_jobs_answer_as_the_document_says() {
    let document = document();
    let keys = |value: &Value| -> BTreeSet<String> {
        value.as_object().unwrap().keys().cloned().collect()
    };
    let body = |response: axum::response::Response| async move {
        let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        serde_json::from_slice::<Value>(&bytes).unwrap()
    };
    let error = crate::error::Error(StatusCode::CONFLICT, "Busy".into()).into_response();
    assert_eq!(
        keys(&body(error).await),
        keys(&document["components"]["schemas"]["Error"]["properties"])
    );
    let Ok(axum::Json(queued)) = crate::jobs::launch(
        crate::test_support::app(false),
        "test",
        "test".into(),
        |_, _| async { Ok(json!({})) },
    ) else {
        panic!("the job was not queued");
    };
    assert_eq!(
        keys(&queued),
        keys(&document["components"]["schemas"]["JobStarted"]["properties"])
    );
}

#[tokio::test]
async fn the_document_needs_the_session_as_the_rest_of_the_api_does() {
    let ask = |auth| async move {
        crate::router(crate::test_support::app(auth))
            .oneshot(
                Request::builder()
                    .uri("/api/openapi.json")
                    .header("host", "localhost:8080")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    };
    assert_eq!(ask(true).await.status(), StatusCode::UNAUTHORIZED);
    let response = ask(false).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    let bytes = to_bytes(response.into_body(), 16 << 20).await.unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), document());
    assert!(
        include_str!("../../../docs/API.md").contains("| GET | `/api/openapi.json` |"),
        "docs/API.md must list the route"
    );
}
