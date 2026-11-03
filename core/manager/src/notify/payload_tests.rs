//! What each format posts for a notice: the body every receiver expects, the link when one is
//! known, the token as a header, and text that no receiver reads as markup or a mention.
use super::*;
use crate::notify::{Kind, target::targets};

fn target(address: &str) -> Target {
    targets(Some(address)).unwrap().remove(0)
}

fn notice() -> Notice {
    Notice {
        kind: Kind::HumanRequest,
        title: "Unity needs a person".into(),
        body: "Solve the CAPTCHA <!channel> & @everyone".into(),
        container: Some("Unity".into()),
        link: Some("#/desktop/c1".into()),
        key: Some("human_request:r1".into()),
    }
}

const LINK: &str = "https://thoric.lan:8443/#/desktop/c1";

#[test]
fn ntfy_gets_its_topic_title_priority_and_click_with_the_token_as_a_header() {
    let request = request(
        &target("https://ntfy.sh/alerts?token=tk_x"),
        &notice(),
        Some(LINK),
        7,
    );
    assert_eq!(request.url, "https://ntfy.sh/");
    assert_eq!(
        request.headers,
        [
            "Content-Type: application/json",
            "Authorization: Bearer tk_x"
        ]
    );
    assert_eq!(
        request.body,
        json!({
            "topic": "alerts",
            "title": "Unity needs a person",
            "message": "Solve the CAPTCHA <!channel> & @everyone",
            "priority": 4,
            "click": LINK,
        })
    );
    // The address's priority wins over the kind's, and without a link there is no click.
    let request = request_without_link("https://ntfy.sh/alerts?priority=min");
    assert_eq!(request.body["priority"], 1);
    assert!(request.body.get("click").is_none());
    assert_eq!(request.headers, ["Content-Type: application/json"]);
}

fn request_without_link(address: &str) -> Request {
    request(&target(address), &notice(), None, 7)
}

#[test]
fn discord_gets_an_embed_that_mentions_nobody() {
    let request = request(
        &target("https://discord.com/api/webhooks/1/secret"),
        &notice(),
        Some(LINK),
        7,
    );
    assert_eq!(request.url, "https://discord.com/api/webhooks/1/secret");
    assert_eq!(
        request.body,
        json!({
            "username": "Thoric",
            "allowed_mentions": {"parse": []},
            "embeds": [{
                "title": "Unity needs a person",
                "description": "Solve the CAPTCHA <!channel> & @everyone",
                "url": LINK,
            }],
        })
    );
    assert!(
        request_without_link("https://discord.com/api/webhooks/1/secret").body["embeds"][0]
            .get("url")
            .is_none()
    );
}

#[test]
fn slack_gets_text_whose_markup_is_escaped() {
    let request = request(
        &target("https://hooks.slack.com/services/T/B/x"),
        &notice(),
        Some(LINK),
        7,
    );
    assert_eq!(
        request.body,
        json!({"text": format!("*Unity needs a person*\nSolve the CAPTCHA &lt;!channel&gt; &amp; @everyone\n<{LINK}|Open in Thoric>")})
    );
}

#[test]
fn telegram_gets_its_chat_and_plain_text() {
    let request = request(
        &target("https://api.telegram.org/bot1:A/sendMessage?chat_id=-100&message_thread_id=3"),
        &notice(),
        Some(LINK),
        7,
    );
    assert_eq!(request.url, "https://api.telegram.org/bot1:A/sendMessage");
    assert_eq!(
        request.body,
        json!({
            "chat_id": "-100",
            "text": format!("Unity needs a person\n\nSolve the CAPTCHA <!channel> & @everyone\n\n{LINK}"),
            "link_preview_options": {"is_disabled": true},
            "message_thread_id": 3,
        })
    );
}

#[test]
fn any_other_address_gets_the_documented_payload() {
    let sent = request(
        &target("https://example.com/hook?key=1"),
        &notice(),
        Some(LINK),
        1_700_000_000,
    );
    assert_eq!(sent.url, "https://example.com/hook?key=1");
    assert_eq!(
        sent.body,
        json!({
            "kind": "human_request",
            "title": "Unity needs a person",
            "body": "Solve the CAPTCHA <!channel> & @everyone",
            "container": "Unity",
            "link": LINK,
            "time": 1_700_000_000,
        })
    );
    let test = Notice {
        kind: Kind::Test,
        container: None,
        ..notice()
    };
    let sent = request(&target("https://example.com/hook"), &test, None, 1);
    assert_eq!(sent.body["kind"], "test");
    assert_eq!(sent.body["container"], Value::Null);
    assert_eq!(sent.body["link"], Value::Null);
}
