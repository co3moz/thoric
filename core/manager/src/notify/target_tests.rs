//! How `NOTIFY_URL` is read: each address's format from its prefix or its host, the parameters
//! Thoric reads off ntfy and Telegram addresses, what is refused, and that an address is never
//! repeated by an error, a debug line or an answer, and `DASHBOARD_URL`.
use super::*;

fn one(address: &str) -> Target {
    let mut targets = targets(Some(address)).unwrap();
    assert_eq!(targets.len(), 1);
    targets.remove(0)
}

#[test]
fn the_format_comes_from_the_host_or_a_prefix() {
    let cases = [
        ("https://ntfy.sh/alerts", Format::Ntfy, "ntfy.sh"),
        (
            "https://discord.com/api/webhooks/123456/discordsecret",
            Format::Discord,
            "discord.com",
        ),
        (
            "https://hooks.slack.com/services/T1/B2/slacksecret",
            Format::Slack,
            "hooks.slack.com",
        ),
        (
            "https://api.telegram.org/bot123:ABC/sendMessage?chat_id=42",
            Format::Telegram,
            "api.telegram.org",
        ),
        (
            "https://example.com/hooks/thoric",
            Format::Json,
            "example.com",
        ),
        // Discord's host without a webhook path is any address.
        ("https://discord.com/other", Format::Json, "discord.com"),
        (
            "ntfy+http://ntfy.lan:8080/alerts",
            Format::Ntfy,
            "ntfy.lan:8080",
        ),
        (
            "slack+https://chat.example/hooks/abc",
            Format::Slack,
            "chat.example",
        ),
        (
            "discord+https://hooks.example/api/x",
            Format::Discord,
            "hooks.example",
        ),
        ("json+https://ntfy.sh/not-ntfy", Format::Json, "ntfy.sh"),
        (
            "telegram+https://bots.example/bot9:Z?chat_id=-100",
            Format::Telegram,
            "bots.example",
        ),
    ];
    for (address, format, host) in cases {
        let target = one(address);
        assert_eq!(
            (target.format, target.host.as_str()),
            (format, host),
            "{address}"
        );
    }
}

#[test]
fn ntfy_publishes_to_the_root_with_the_topic_token_and_priority_read_off() {
    let target = one("https://ntfy.sh/thoric-alerts?token=tk_abcdef&priority=high");
    assert_eq!(target.url, "https://ntfy.sh/");
    assert_eq!(target.topic.as_deref(), Some("thoric-alerts"));
    assert_eq!(target.token.as_deref(), Some("tk_abcdef"));
    assert_eq!(target.priority, Some(4));
    let own = one("ntfy+https://user:pass@push.example/ntfy/alerts?priority=1");
    assert_eq!(own.url, "https://user:pass@push.example/ntfy/");
    assert_eq!(own.host, "push.example");
    assert_eq!((own.token, own.priority), (None, Some(1)));
    for refused in [
        "https://ntfy.sh/",
        "https://ntfy.sh/bad.topic",
        "https://ntfy.sh/alerts?tags=x",
        "https://ntfy.sh/alerts?priority=6",
        "https://ntfy.sh/alerts?token",
    ] {
        assert!(targets(Some(refused)).is_err(), "{refused}");
    }
}

#[test]
fn telegram_needs_its_bot_and_chat() {
    let target = one(
        "https://api.telegram.org/bot123:ABC-def/sendMessage?chat_id=@thoric&message_thread_id=7",
    );
    assert_eq!(
        target.url,
        "https://api.telegram.org/bot123:ABC-def/sendMessage"
    );
    assert_eq!(target.chat_id.as_deref(), Some("@thoric"));
    assert_eq!(target.thread_id, Some(7));
    // sendMessage may be left out.
    assert_eq!(
        one("https://api.telegram.org/bot123:ABC?chat_id=42").url,
        "https://api.telegram.org/bot123:ABC/sendMessage"
    );
    for refused in [
        "https://api.telegram.org/bot123:ABC/sendMessage",
        "https://api.telegram.org/sendMessage?chat_id=42",
        "https://api.telegram.org/bot123:ABC/sendMessage?chat_id=42&parse_mode=HTML",
        "https://api.telegram.org/bot123:ABC?chat_id=42&message_thread_id=x",
    ] {
        assert!(targets(Some(refused)).is_err(), "{refused}");
    }
}

#[test]
fn webhooks_post_to_the_address_as_written() {
    let address = "https://example.com/hooks/thoric?token=abc&x=1";
    assert_eq!(one(address).url, address);
    let discord = "https://discord.com/api/webhooks/123/secret?thread_id=9";
    assert_eq!(one(discord).url, discord);
}

#[test]
fn several_addresses_are_separated_by_commas() {
    let targets = targets(Some(
        " https://ntfy.sh/a , https://example.com/hook,,https://hooks.slack.com/services/x ",
    ))
    .unwrap();
    let formats: Vec<Format> = targets.iter().map(|target| target.format).collect();
    assert_eq!(formats, [Format::Ntfy, Format::Json, Format::Slack]);
    assert!(super::targets(Some("")).unwrap().is_empty());
    assert!(super::targets(None).unwrap().is_empty());
    let many = ["https://example.com/hook"; MAX_TARGETS + 1].join(",");
    assert!(super::targets(Some(&many)).is_err());
}

#[test]
fn a_refused_address_is_named_by_its_place_alone() {
    for (value, problem) in [
        (
            "https://ntfy.sh/a,ftp://secret-host/secret-path",
            "address 2 must start with",
        ),
        ("https://ntfy.sh/a b-secret", "address 1 must be at most"),
        (
            "https://example.com/\"secret\"",
            "address 1 must be at most",
        ),
        ("https:///secret-path", "address 1 must name a host"),
        (
            "https://example.com/#secret-fragment",
            "address 1 must be at most",
        ),
    ] {
        let error = format!("{:#}", targets(Some(value)).unwrap_err());
        assert!(
            error.starts_with("NOTIFY_URL: ") && error.contains(problem),
            "{error}"
        );
        assert!(!error.contains("secret"), "{error}");
    }
}

#[test]
fn an_address_shows_its_format_and_host_and_hides_the_rest() {
    let address = "https://user:password-of-it@example.com/hooks/abcdefghij?key=keyvalue";
    let target = one(address);
    assert_eq!(format!("{target:?}"), "JSON webhook at example.com");
    assert_eq!(target.describe(), "JSON webhook at example.com");
    let answer = format!(
        "posting {address} failed; path /hooks/abcdefghij, key keyvalue, user user:password-of-it"
    );
    let redacted = target.redact(&answer);
    for secret in ["password-of-it", "abcdefghij", "keyvalue"] {
        assert!(!redacted.contains(secret), "{redacted}");
    }
    assert!(redacted.contains("[hidden]"), "{redacted}");
    // A path of short parts is hidden whole, also as JSON writes its slashes.
    let short = one("https://example.com/h/k1/z9?x=1");
    assert_eq!(
        short.redact("{\"path\":\"\\/h\\/k1\\/z9?x=1\"} /h/k1/z9?x=1"),
        "{\"path\":\"[hidden]\"} [hidden]"
    );
    let ntfy = one("https://ntfy.sh/topic9?token=tk_9");
    assert_eq!(
        ntfy.redact("{\"topic\":\"topic9\",\"token\":\"tk_9\"}"),
        "{\"topic\":\"[hidden]\",\"token\":\"[hidden]\"}"
    );
    let telegram = one("https://api.telegram.org/bot123:ABC/sendMessage?chat_id=42");
    assert_eq!(
        telegram.redact("bot 123:ABC said no"),
        "bot [hidden] said no"
    );
}

#[test]
fn dashboard_url_is_an_address_without_a_slash_at_the_end() {
    assert_eq!(dashboard_url(None).unwrap(), None);
    assert_eq!(dashboard_url(Some("  ")).unwrap(), None);
    assert_eq!(
        dashboard_url(Some("https://thoric.lan:8443/"))
            .unwrap()
            .as_deref(),
        Some("https://thoric.lan:8443")
    );
    assert_eq!(
        dashboard_url(Some("http://192.168.1.20:8080/thoric"))
            .unwrap()
            .as_deref(),
        Some("http://192.168.1.20:8080/thoric")
    );
    for refused in [
        "thoric.lan",
        "ftp://thoric.lan",
        "https://",
        "https://thoric.lan/#/desktop",
        "https://thoric.lan/?a=b",
        "https://user@thoric.lan",
        "https://thoric.lan/a b",
    ] {
        assert!(dashboard_url(Some(refused)).is_err(), "{refused}");
    }
}
