//! What the settings page is shown of the configuration: every setting Compose passes to the
//! manager, as `.env.example` names it, and never a secret's value. Also how `TEMPLATES_URL`,
//! `VOLUME_FILTER`, the MCP token and the metrics token are read, with the limits of a script
//! request, that the desktop seccomp profile adds only user namespaces to Docker's default, and
//! that the administrator's password is hashed with Argon2id only at startup.
use super::*;
use crate::exec;
use crate::test_support::DESKTOP_CONTEXT;
use axum::{body::Body, http::Request};
use std::{collections::BTreeMap, path::Path, sync::Arc};
use tower::ServiceExt;

/// A file of the repository, two folders above the crate.
fn read(path: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path),
    )
    .unwrap()
}

/// The lines of the `environment:` list of a manifest's manager service, without its comments.
fn environment(manifest: &str) -> Vec<&str> {
    let service = &manifest[manifest.find("\n  manager:\n").unwrap()..];
    let list = &service[service.find("\n    environment:\n").unwrap() + 18..];
    list.lines()
        .take_while(|line| line.starts_with("      "))
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .collect()
}

/// The names in the `environment:` list of a manifest's manager service.
fn passed(manifest: &str) -> Vec<String> {
    environment(manifest)
        .into_iter()
        .map(|line| line.split(':').next().unwrap().to_owned())
        .collect()
}

fn names(config: &Config) -> Vec<String> {
    config
        .shown()
        .iter()
        .map(|setting| setting["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn the_page_shows_every_setting_compose_passes_to_the_manager() {
    let app = crate::test_support::app(true);
    let shown = names(&app.config);
    let example = read(".env.example");
    let (sections, catalog) = (
        read("core/web/src/lib/settings.ts"),
        read("core/web/src/locales/en.ts"),
    );
    for name in &shown {
        assert!(
            example.contains(&format!("\n{name}=")),
            ".env.example must document {name}"
        );
        // Otherwise the page lists it under Other, without a word on what it does.
        assert!(
            sections.contains(&format!("\n  {name}: \"")),
            "core/web/src/lib/settings.ts must give {name} a section"
        );
        assert!(
            catalog.contains(&format!("\"settings.about.{name}\":")),
            "core/web/src/locales/en.ts must say what {name} does"
        );
    }
    for manifest in ["compose.yml", "compose.ghcr.yml"] {
        let passed = passed(&read(manifest));
        assert!(passed.len() > 10, "{manifest}: {passed:?}");
        // The log filter is read by the logger, not the configuration, and a renamed variable
        // only stands in for its new name.
        let renamed = |name: &String| RENAMED.iter().any(|renamed| renamed.old == name);
        for name in passed
            .iter()
            .filter(|name| *name != "RUST_LOG" && !renamed(name))
        {
            assert!(
                shown.contains(name),
                "{manifest} passes {name}, which the settings page does not show"
            );
        }
    }
}

#[test]
fn a_secret_says_only_whether_it_is_set() {
    let mut app = crate::test_support::app(true);
    let config = &mut Arc::get_mut(&mut app).unwrap().config;
    config.password_hash = Some("$argon2id$v=19$stored-password-hash".into());
    config.mcp_token = Some("token-of-the-agent-endpoint".into());
    config.metrics_token = Some("token-of-the-prometheus-scraper".into());
    config.proxy = crate::outbound::installation(
        &|key: &str| {
            Some(
                match key {
                    "PROXY_URL" => "http://proxy.example:3128",
                    "PROXY_USERNAME" => "corp",
                    "PROXY_PASSWORD" => "password-of-the-proxy",
                    _ => return None,
                }
                .into(),
            )
        },
        &mut Vec::new(),
    )
    .unwrap();
    config.notify = crate::notify::settings(&|key: &str| {
        (key == "NOTIFY_URL").then(|| "https://ntfy.sh/topic-of-the-phone?token=tk_notify".into())
    })
    .unwrap();
    let shown = serde_json::to_string(&config.shown()).unwrap();
    assert!(!shown.contains("stored-password-hash"), "{shown}");
    assert!(!shown.contains("token-of-the-agent-endpoint"), "{shown}");
    assert!(
        !shown.contains("token-of-the-prometheus-scraper"),
        "{shown}"
    );
    assert!(!shown.contains("password-of-the-proxy"), "{shown}");
    assert!(!shown.contains("topic-of-the-phone"), "{shown}");
    assert!(!shown.contains("tk_notify"), "{shown}");
    let setting = |config: &Config, name: &str| {
        config
            .shown()
            .into_iter()
            .find(|setting| setting["name"] == name)
            .unwrap()
    };
    assert_eq!(
        setting(config, "MCP_TOKEN"),
        json!({"name": "MCP_TOKEN", "secret": true, "set": true, "editable": false})
    );
    assert_eq!(setting(config, "ADMIN_PASSWORD")["set"], true);
    assert_eq!(
        setting(config, "METRICS_TOKEN"),
        json!({"name": "METRICS_TOKEN", "secret": true, "set": true, "editable": false})
    );
    assert_eq!(setting(config, "PROXY_PASSWORD")["set"], true);
    assert_eq!(setting(config, "PROXY_USERNAME")["value"], "corp");
    // The page replaces this one, still without reading it.
    assert_eq!(
        setting(config, "NOTIFY_URL"),
        json!({"name": "NOTIFY_URL", "secret": true, "set": true, "editable": true})
    );
    assert_eq!(setting(config, "NOTIFY_EVENTS")["value"], "human_request");
    config.mcp_token = None;
    config.password_hash = None;
    config.metrics_token = None;
    assert_eq!(setting(config, "METRICS_TOKEN")["set"], false);
    assert_eq!(setting(config, "MCP_TOKEN")["set"], false);
    assert_eq!(setting(config, "MCP_AUTH_MODE")["value"], "none");
    assert_eq!(setting(config, "ADMIN_PASSWORD")["set"], false);
    // Values are what the manager made of the environment, as .env would spell them.
    config.templates_url = None;
    config.upload_limit = 1024 * 1024 * 1024;
    assert_eq!(setting(config, "TEMPLATES_URL")["value"], "none");
    assert_eq!(setting(config, "UPLOAD_MAX_MB")["value"], 1024);
}

#[test]
fn the_administrator_password_is_hashed_at_startup_only() {
    use argon2::{PasswordHash, PasswordVerifier};
    let get = |key: &str| match key {
        "ADMIN_PASSWORD" => Some("correct horse".to_owned()),
        // The seccomp profile is a file of the repository root, not of the test's folder.
        "DESKTOP_USER_NAMESPACES" => Some("false".to_owned()),
        _ => None,
    };
    let hash = Config::parse(&get, Reading::Startup)
        .unwrap()
        .password_hash
        .unwrap();
    // Argon2id with its default parameters and a salt of 16 bytes, which is 22 base64 letters.
    let parts: Vec<&str> = hash.split('$').collect();
    assert_eq!(
        parts[..4],
        ["", "argon2id", "v=19", "m=19456,t=2,p=1"],
        "{hash}"
    );
    assert_eq!(parts[4].len(), 22, "{hash}");
    let parsed = PasswordHash::new(&hash).unwrap();
    assert!(
        Argon2::default()
            .verify_password(b"correct horse", &parsed)
            .is_ok()
    );
    assert!(
        Argon2::default()
            .verify_password(b"wrong horse", &parsed)
            .is_err()
    );
    // Every start salts anew, and a change from the dashboard never hashes.
    let again = Config::parse(&get, Reading::Startup).unwrap().password_hash;
    assert_ne!(again.as_deref(), Some(hash.as_str()));
    let change = Config::parse(&get, Reading::Change).unwrap().password_hash;
    assert_eq!(change, None);
}

#[tokio::test]
async fn the_configuration_route_needs_a_session_and_is_documented() {
    let response = crate::router(crate::test_support::app(true))
        .oneshot(
            Request::builder()
                .uri("/api/configuration")
                .header("host", "localhost:8080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    let response = crate::router(crate::test_support::app(false))
        .oneshot(
            Request::builder()
                .uri("/api/configuration")
                .header("host", "localhost:8080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["variables"][0]["name"], "AUTH_MODE");
    assert!(read("docs/API.md").contains("| GET | `/api/configuration` |"));
}

#[test]
fn a_renamed_variable_keeps_its_meaning_and_startup_says_to_rename_it() {
    let read = |vars: &[(&str, &str)]| {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        let get = move |key: &str| {
            vars.iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        };
        let mut notices = Vec::new();
        let value = renamed_value(&get, &RENAMED[0], &mut notices);
        (value, notices)
    };
    // The new name alone, or the old one empty as Compose passes it once .env lost it.
    for vars in [
        &[("ROOT_MCP_ENABLED", "true")][..],
        &[("ROOT_MCP_ENABLED", "true"), ("MCP_ENABLED", "")],
    ] {
        let (value, notices) = read(vars);
        assert_eq!(value.unwrap().as_deref(), Some("true"));
        assert!(notices.is_empty(), "{notices:?}");
    }
    assert_eq!(read(&[]).0.unwrap(), None);
    // An .env from before the rename keeps its value and hears of the new name and the release.
    let (value, notices) = read(&[("MCP_ENABLED", "true")]);
    assert_eq!(value.unwrap().as_deref(), Some("true"));
    assert!(
        notices[0].contains("MCP_ENABLED was renamed ROOT_MCP_ENABLED in Thoric 0.6.3"),
        "{notices:?}"
    );
    // As the manifests pass it: the new name falls back to the old one, which is passed too.
    let (value, notices) = read(&[("ROOT_MCP_ENABLED", "true"), ("MCP_ENABLED", "true")]);
    assert_eq!(value.unwrap().as_deref(), Some("true"));
    assert!(notices[0].contains("Remove MCP_ENABLED"), "{notices:?}");
    // Two names that disagree are not guessed between.
    let error = read(&[("ROOT_MCP_ENABLED", "false"), ("MCP_ENABLED", "true")])
        .0
        .unwrap_err()
        .to_string();
    assert!(error.contains("disagree"), "{error}");
}

#[test]
fn the_manifests_carry_a_renamed_variable_over_to_its_new_name() {
    for manifest in ["compose.yml", "compose.ghcr.yml"] {
        let manifest = read(manifest);
        let lines = environment(&manifest);
        for Renamed { old, new, .. } in RENAMED {
            // The old value applies while .env has no new one, and the old name reaches the
            // manager, empty once it is gone, so startup can tell an .env that still has it.
            assert!(
                lines
                    .iter()
                    .any(|line| line.starts_with(&format!("{new}: ${{{new}:-${{{old}:-"))),
                "{lines:?}"
            );
            assert!(
                lines.contains(&format!("{old}: ${{{old}:-}}").as_str()),
                "{lines:?}"
            );
        }
    }
    let example = read(".env.example");
    for Renamed { old, new, .. } in RENAMED {
        assert!(example.contains(&format!("\n{new}=")));
        assert!(!example.contains(&format!("\n{old}=")));
    }
}

#[test]
fn the_page_edits_only_the_settings_it_may_change() {
    let app = crate::test_support::app(true);
    let shown = app.config.shown();
    let names = names(&app.config);
    for name in EDITABLE {
        assert!(
            names.iter().any(|shown| shown == name),
            "{name} is not shown"
        );
    }
    // Sign-in, the root MCP and its token, the other secrets and what Compose reads stay with
    // thoric settings.
    for name in [
        "AUTH_MODE",
        "ADMIN_USERNAME",
        "ADMIN_PASSWORD",
        "MANAGER_CONTAINER",
        "ROOT_MCP_ENABLED",
        "MCP_AUTH_MODE",
        "MCP_TOKEN",
        "METRICS_TOKEN",
        "PROXY_PASSWORD",
        "DESKTOP_HOST",
    ] {
        assert!(!EDITABLE.contains(&name), "{name}");
    }
    for name in EDITABLE_SECRETS {
        assert!(EDITABLE.contains(name), "{name}");
    }
    for entry in &shown {
        let name = entry["name"].as_str().unwrap();
        let editable = EDITABLE.contains(&name);
        assert_eq!(entry["editable"], editable, "{entry}");
        // A secret's text is its value, which the page never gets.
        let secret = entry["secret"] == true;
        assert_eq!(
            secret && editable,
            EDITABLE_SECRETS.contains(&name),
            "{entry}"
        );
        assert_eq!(entry.get("text").is_some(), editable && !secret, "{entry}");
    }
}

#[test]
fn a_value_from_the_dashboard_is_one_line_that_env_keeps_as_it_is() {
    for fine in [
        "",
        "true",
        "1024",
        "https://example.com/templates/index.json",
        "GeForce RTX 4090",
        "team=desktops",
        "desktop.lan,other.lan",
    ] {
        env_value("X", fine).unwrap();
    }
    let long = "x".repeat(2049);
    for refused in [
        "a\nb", "a\rb", "a\tb", " a", "a ", "$HOME", "${X}", "a #b", "'a'", "\"a\"", "a`b`",
        r"a\b", "é", &long,
    ] {
        assert!(env_value("X", refused).is_err(), "{refused:?}");
    }
}

#[test]
fn a_change_is_read_as_the_manager_would_start_with_it() {
    let mut app = crate::test_support::app(true);
    let config = &mut Arc::get_mut(&mut app).unwrap().config;
    // Sign-in stays on, but the password, which only its hash is kept of, is not read again.
    config.environment = BTreeMap::from([
        ("AUTH_MODE".into(), "password".into()),
        ("UPLOAD_MAX_MB".into(), "1024".into()),
        ("ALLOWED_HOSTS".into(), "desktop.lan".into()),
        ("DESKTOP_HOST".into(), "secure.lan".into()),
        ("DESKTOP_CONTEXT".into(), DESKTOP_CONTEXT.into()),
    ]);
    let change = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    };
    let changed = config
        .with_changes(&change(&[
            ("UPLOAD_MAX_MB", "2048"),
            ("ALLOWED_HOSTS", "a.lan"),
        ]))
        .unwrap();
    assert_eq!(changed.upload_limit, 2048 * 1024 * 1024);
    assert!(changed.auth);
    // ALLOWED_HOSTS is what .env says; DESKTOP_HOST comes on its own and is answered as well.
    assert_eq!(changed.allowed_hosts, ["a.lan"]);
    assert!(changed.host_allowed("a.lan") && changed.host_allowed("secure.lan"));
    assert!(!changed.host_allowed("desktop.lan"));
    assert_eq!(changed.environment["UPLOAD_MAX_MB"], "2048");
    // Each setting's problem is its own; a change of several is checked together too.
    let problems = config.check_changes(&change(&[
        ("UPLOAD_MAX_MB", "70000"),
        ("WSL_GPU", "false"),
        ("AUTH_MODE", "none"),
    ]));
    assert_eq!(
        problems.fields.keys().collect::<Vec<_>>(),
        ["AUTH_MODE", "UPLOAD_MAX_MB"]
    );
    assert!(problems.fields["AUTH_MODE"].contains("thoric settings"));
    assert!(problems.together.is_none());
    assert!(problems.to_string().contains("UPLOAD_MAX_MB must be"));
    assert!(
        config
            .check_changes(&change(&[("WSL_GPU", "false")]))
            .is_empty()
    );
    // A pids limit no desktop could start with is refused before the manager restarts.
    let problems = config.check_changes(&change(&[("DESKTOP_PIDS_LIMIT", "100")]));
    assert!(
        problems.fields["DESKTOP_PIDS_LIMIT"].contains("from 1024"),
        "{problems}"
    );
    let changed = config
        .with_changes(&change(&[("DESKTOP_PIDS_LIMIT", "0")]))
        .unwrap();
    assert_eq!((config.pids_limit, changed.pids_limit), (16384, 0));
}

/// The Settings page puts each `<NAME>_DEFAULT` beside its `<NAME>_ENABLED` and switches it off
/// while that one is off, the microphone's also without sound (`settingRows` and `idleSettings`
/// in `core/web/src/lib/settings.ts`), which holds only while a default never gives more.
#[test]
fn a_default_gives_nothing_while_its_feature_is_switched_off() {
    let mut app = crate::test_support::app(true);
    let config = &mut Arc::get_mut(&mut app).unwrap().config;
    config.environment = BTreeMap::from([
        ("AUTH_MODE".into(), "password".into()),
        ("DESKTOP_CONTEXT".into(), DESKTOP_CONTEXT.into()),
    ]);
    let shown = names(config);
    let paired: Vec<&str> = shown
        .iter()
        .filter_map(|name| name.strip_suffix("_DEFAULT"))
        .filter(|base| shown.contains(&format!("{base}_ENABLED")))
        .collect();
    assert_eq!(
        paired,
        ["INTERNET", "SUDO", "AUDIO", "MICROPHONE", "GAMEPAD", "VPN"]
    );
    let defaults = |off: &[&str]| {
        let mut changes: BTreeMap<String, String> = paired
            .iter()
            .map(|base| (format!("{base}_DEFAULT"), "true".into()))
            .collect();
        changes.extend(off.iter().map(|name| (name.to_string(), "false".into())));
        serde_json::to_value(config.with_changes(&changes).unwrap().feature_defaults()).unwrap()
    };
    for base in &paired {
        let feature = base.to_lowercase();
        assert_eq!(defaults(&[])[&feature], true, "{base}_DEFAULT");
        let enabled = format!("{base}_ENABLED");
        assert_eq!(defaults(&[enabled.as_str()])[&feature], false, "{enabled}");
    }
    // The microphone travels over the audio connection.
    assert_eq!(defaults(&["AUDIO_ENABLED"])["microphone"], false);
}

#[test]
fn templates_url_defaults_to_the_wiki_and_none_turns_the_page_off() {
    for unset in [None, Some(""), Some("  ")] {
        assert_eq!(
            templates_url(unset).unwrap().as_deref(),
            Some(TEMPLATES_URL)
        );
    }
    assert_eq!(templates_url(Some("none")).unwrap(), None);
    let mirror = "http://10.0.0.5:8000/thoric/index.json";
    assert_eq!(
        templates_url(Some(mirror)).unwrap().as_deref(),
        Some(mirror)
    );
    // The policy header names the host, so nothing may close the directive or start another.
    for refused in [
        "None",
        "ftp://example.com/index.json",
        "https://",
        "https:///index.json",
        "https://example.com/a b.json",
        "https://example.com;script-src *",
        "https://example.com/index.json'",
        "https://user@example.com/index.json",
        "https://example.com?x/index.json",
    ] {
        assert!(templates_url(Some(refused)).is_err(), "{refused}");
    }
}

#[test]
fn volume_filter_defaults_to_the_managers_label_and_none_lists_every_volume() {
    // Compose hands an unset variable over as an empty one, so empty cannot switch it off.
    for unset in [None, Some(""), Some("  ")] {
        assert_eq!(
            volume_filter(unset).unwrap().as_deref(),
            Some("thoric.managed=true")
        );
    }
    assert_eq!(volume_filter(Some("none")).unwrap(), None);
    for label in [
        "team",
        "team=",
        "com.example.team=unity",
        "thoric.managed",
        "thoric.managed=true",
    ] {
        assert_eq!(volume_filter(Some(label)).unwrap().as_deref(), Some(label));
    }
    // A label the manager could not put on what it creates, or one Docker's filter reads apart.
    for refused in [
        "=true",
        "team name=a",
        "team=a b",
        "team=a,b",
        "team\n=a",
        "thoric.managed=false",
        "thoric.managed=",
    ] {
        assert!(volume_filter(Some(refused)).is_err(), "{refused}");
    }
}

#[test]
fn mcp_token_and_script_validation() {
    // Any length up to 256 is accepted; whitespace and non-ASCII cannot travel in a bearer header.
    for token in [
        "",
        "short token",
        "token\n",
        "şifre",
        "x".repeat(257).as_str(),
    ] {
        assert!(validate_mcp_token(true, token).is_err());
    }
    assert!(validate_mcp_token(false, "").is_ok());
    for token in ["x", "$hort#'\"`%!", "x".repeat(256).as_str()] {
        assert!(validate_mcp_token(true, token).is_ok());
    }
    // Only an explicit MCP_AUTH_MODE=none drops the token; an unset or mistyped mode does not.
    for mode in [None, Some("token")] {
        assert!(mcp_token(true, mode, String::new()).is_err());
    }
    for mode in [Some(""), Some("off"), Some("None")] {
        assert!(mcp_token(true, mode, "0".repeat(32)).is_err());
    }
    assert_eq!(
        mcp_token(true, None, "0".repeat(32)).unwrap(),
        Some("0".repeat(32))
    );
    assert_eq!(mcp_token(true, Some("none"), String::new()).unwrap(), None);
    for body in [
        serde_json::json!({"script":""}),
        serde_json::json!({"script":"echo ok","timeout_seconds":0}),
        serde_json::json!({"script":"echo ok","timeout_seconds":3501}),
        serde_json::json!({"script":"echo\0ok"}),
    ] {
        assert!(
            serde_json::from_value::<exec::Execute>(body)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    let body: exec::Execute = serde_json::from_value(serde_json::json!({"script":"id"})).unwrap();
    assert!(!body.root);
    assert_eq!(body.timeout_seconds, 300);
    assert!(body.validate().is_ok());
}

#[test]
fn the_desktop_seccomp_profile_adds_only_user_namespaces_to_dockers_default() {
    let profile: serde_json::Value =
        serde_json::from_str(include_str!("../../../images/desktop/seccomp.json")).unwrap();
    assert_eq!(profile["defaultAction"], "SCMP_ACT_ERRNO");
    let rules = profile["syscalls"].as_array().unwrap();
    let unconditional = |name: &str| {
        rules.iter().any(|rule| {
            rule["action"] == "SCMP_ACT_ALLOW"
                && rule["names"].as_array().unwrap().iter().any(|n| n == name)
                && rule.get("args").is_none()
                && rule.get("includes").is_none()
        })
    };
    // A sandbox starts by cloning or unsharing into a new user namespace.
    assert!(unconditional("clone") && unconditional("unshare"));
    // Calls Docker reserves for CAP_SYS_ADMIN stay refused, so a process in its own namespace
    // still cannot mount file systems or join another process's namespaces.
    for name in ["mount", "umount2", "fsopen", "open_tree", "setns", "bpf"] {
        assert!(
            !unconditional(name),
            "{name} is allowed without CAP_SYS_ADMIN"
        );
    }
    // glibc falls back from clone3 to clone only when clone3 reports ENOSYS.
    assert!(
        rules
            .iter()
            .any(|rule| rule["names"] == serde_json::json!(["clone3"])
                && rule["errnoRet"] == 38
                && rule["excludes"]["caps"] == serde_json::json!(["CAP_SYS_ADMIN"]))
    );
}

#[test]
fn the_metrics_token_is_off_while_empty_and_one_header_value_otherwise() {
    assert_eq!(metrics_token(None).unwrap(), None);
    assert_eq!(metrics_token(Some("")).unwrap(), None);
    assert_eq!(
        metrics_token(Some("a$b#c-0123")).unwrap().as_deref(),
        Some("a$b#c-0123")
    );
    for refused in [
        "with space".to_owned(),
        "line\nbreak".to_owned(),
        "x".repeat(257),
    ] {
        let error = metrics_token(Some(&refused)).unwrap_err().to_string();
        assert!(error.starts_with("METRICS_TOKEN must"), "{error}");
    }
}
