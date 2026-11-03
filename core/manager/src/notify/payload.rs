//! What each format posts: ntfy's JSON publishing with the topic in the body, a Discord embed, a
//! Slack message, a Telegram `sendMessage`, and the JSON payload any other address gets. A notice
//! is plain text everywhere: nothing an agent wrote is read as markup or mentions anyone.
use super::{Format, Notice, Target};
use serde_json::{Value, json};

/// A POST: where it goes, its headers and its JSON body.
pub(super) struct Request {
    pub url: String,
    pub headers: Vec<String>,
    pub body: Value,
}

/// The request that brings `notice` to `target`, with `link` as where to look and `time` when it
/// happened, in seconds since the epoch.
pub(super) fn request(target: &Target, notice: &Notice, link: Option<&str>, time: u64) -> Request {
    let mut headers = vec!["Content-Type: application/json".to_owned()];
    let body = match target.format {
        Format::Ntfy => {
            if let Some(token) = &target.token {
                headers.push(format!("Authorization: Bearer {token}"));
            }
            let mut body = json!({
                "topic": target.topic,
                "title": notice.title,
                "message": notice.body,
                "priority": target.priority.unwrap_or(notice.kind.priority()),
            });
            if let Some(link) = link {
                body["click"] = json!(link);
            }
            body
        }
        Format::Discord => {
            let mut embed = json!({"title": notice.title, "description": notice.body});
            if let Some(link) = link {
                embed["url"] = json!(link);
            }
            json!({
                "username": "Thoric",
                "allowed_mentions": {"parse": []},
                "embeds": [embed],
            })
        }
        Format::Slack => {
            let mut text = format!("*{}*\n{}", slack(&notice.title), slack(&notice.body));
            if let Some(link) = link {
                text.push_str(&format!("\n<{link}|Open in Thoric>"));
            }
            json!({"text": text})
        }
        Format::Telegram => {
            let mut text = format!("{}\n\n{}", notice.title, notice.body);
            if let Some(link) = link {
                text.push_str(&format!("\n\n{link}"));
            }
            let mut body = json!({
                "chat_id": target.chat_id,
                "text": text,
                "link_preview_options": {"is_disabled": true},
            });
            if let Some(thread) = target.thread_id {
                body["message_thread_id"] = json!(thread);
            }
            body
        }
        Format::Json => json!({
            "kind": notice.kind.name(),
            "title": notice.title,
            "body": notice.body,
            "container": notice.container,
            "link": link,
            "time": time,
        }),
    };
    Request {
        url: target.url.clone(),
        headers,
        body,
    }
}

/// Text Slack shows as it is: its markup and mentions are made of `<`, `>` and `&`.
fn slack(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
#[path = "payload_tests.rs"]
mod tests;
