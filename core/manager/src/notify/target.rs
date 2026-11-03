//! `NOTIFY_URL`: the addresses notices go to, separated by commas. An address's format comes from
//! a prefix, `ntfy+`, `discord+`, `slack+`, `telegram+` or `json+` before `https://` or
//! `http://`, or else from its host: `ntfy.sh`, a Discord webhook on `discord.com`,
//! `hooks.slack.com` and `api.telegram.org`. Any other address gets the JSON payload.
//!
//! Thoric reads a few parameters off some addresses and sends them elsewhere: an ntfy topic's
//! `token` and `priority`, and a Telegram bot's `chat_id` and `message_thread_id`. The rest is
//! sent as it is. An address carries its credentials, so a problem with one names its place in
//! the list, never its text, and [`Target::redact`] takes its parts out of what a target answers.
use anyhow::{Result, bail};
use serde::Serialize;
use std::fmt;

/// The most addresses `NOTIFY_URL` may name.
const MAX_TARGETS: usize = 8;
const URL_LIMIT: usize = 2048;
/// What stands for a part of an address in an answer or an error.
const HIDDEN: &str = "[hidden]";

/// How a notice is put for an address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Format {
    Ntfy,
    Discord,
    Slack,
    Telegram,
    Json,
}

const PREFIXES: [(&str, Format); 5] = [
    ("ntfy+", Format::Ntfy),
    ("discord+", Format::Discord),
    ("slack+", Format::Slack),
    ("telegram+", Format::Telegram),
    ("json+", Format::Json),
];

impl Format {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Format::Ntfy => "ntfy",
            Format::Discord => "Discord",
            Format::Slack => "Slack",
            Format::Telegram => "Telegram",
            Format::Json => "JSON webhook",
        }
    }
}

/// One address of `NOTIFY_URL`, read.
#[derive(Clone, PartialEq)]
pub(crate) struct Target {
    pub format: Format,
    /// The host, with its port when the address names one.
    pub host: String,
    /// Where the POST goes, without what Thoric read off the address.
    pub(super) url: String,
    /// ntfy's topic, which goes in the body, and its access token.
    pub(super) topic: Option<String>,
    pub(super) token: Option<String>,
    /// ntfy's priority from 1 to 5, rather than the kind's own.
    pub(super) priority: Option<u8>,
    /// The Telegram chat, and the topic in it, a message goes to.
    pub(super) chat_id: Option<String>,
    pub(super) thread_id: Option<i64>,
    /// The parts of the address that may be secret, longest first.
    secrets: Vec<String>,
}

/// Only what may be shown of an address.
impl fmt::Debug for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe())
    }
}

impl Target {
    /// The address as it may be shown, such as `ntfy at ntfy.sh`.
    pub(crate) fn describe(&self) -> String {
        format!("{} at {}", self.format.name(), self.host)
    }
    /// `text` with every part of the address that may be secret replaced.
    pub(crate) fn redact(&self, text: &str) -> String {
        let mut text = text.to_owned();
        for secret in &self.secrets {
            text = text.replace(secret.as_str(), HIDDEN);
        }
        text
    }
}

/// The addresses of `NOTIFY_URL`, none when it is empty.
pub(crate) fn targets(value: Option<&str>) -> Result<Vec<Target>> {
    let addresses: Vec<&str> = value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .collect();
    if addresses.len() > MAX_TARGETS {
        bail!("NOTIFY_URL names at most {MAX_TARGETS} addresses");
    }
    addresses
        .iter()
        .enumerate()
        .map(|(index, address)| {
            target(address)
                .map_err(|problem| anyhow::anyhow!("NOTIFY_URL: address {} {problem}", index + 1))
        })
        .collect()
}

/// Whether a byte may stand in an address: visible ASCII without the quotes, backslashes and
/// brackets a URL never needs unencoded, the comma between addresses and a fragment.
fn allowed(byte: u8) -> bool {
    byte.is_ascii_graphic() && !b"\"'`\\<>{}|^#,".contains(&byte)
}

fn host_allowed(host: &str) -> bool {
    !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_:[]".contains(&b))
}

/// The scheme, the authority and what follows it of an http or https address.
fn split(address: &str) -> std::result::Result<(&str, &str, &str), String> {
    let (scheme, rest) = if let Some(rest) = address.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = address.strip_prefix("http://") {
        ("http", rest)
    } else {
        return Err("must start with https://, http:// or a format such as ntfy+https://".into());
    };
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, after) = rest.split_at(end);
    Ok((scheme, authority, after))
}

fn target(text: &str) -> std::result::Result<Target, String> {
    let (forced, address) = PREFIXES
        .iter()
        .find(|(prefix, _)| text.starts_with(prefix))
        .map_or((None, text), |(prefix, format)| {
            (Some(*format), &text[prefix.len()..])
        });
    if address.len() > URL_LIMIT || !address.bytes().all(allowed) {
        return Err(format!(
            "must be at most {URL_LIMIT} characters without spaces, quotes, backslashes, commas or <>{{}}|^#"
        ));
    }
    let (scheme, authority, after) = split(address)?;
    let (userinfo, host) = match authority.rsplit_once('@') {
        Some((userinfo, host)) => (Some(userinfo), host),
        None => (None, authority),
    };
    if !host_allowed(host) {
        return Err("must name a host".into());
    }
    let host = host.to_ascii_lowercase();
    let (path, query) = match after.split_once('?') {
        Some((path, query)) => (path, query),
        None => (after, ""),
    };
    let path = if path.is_empty() { "/" } else { path };
    let format = forced.unwrap_or_else(|| detect(&host, path));
    let mut secrets: Vec<String> = vec![text.to_owned(), address.to_owned()];
    secrets.extend(userinfo.map(str::to_owned));
    if after.len() > 1 {
        secrets.push(after.to_owned());
    }
    // A webhook's path is its credential; the short parts are words such as api or hooks.
    secrets.extend(
        path.split('/')
            .filter(|part| part.len() >= 8)
            .map(str::to_owned),
    );
    let pairs = parameters(query)?;
    secrets.extend(
        pairs
            .iter()
            .filter(|(_, value)| value.len() >= 4)
            .map(|(_, value)| (*value).to_owned()),
    );
    let origin = format!("{scheme}://{authority}");
    let mut target = Target {
        format,
        host,
        url: address.to_owned(),
        topic: None,
        token: None,
        priority: None,
        chat_id: None,
        thread_id: None,
        secrets: Vec::new(),
    };
    match format {
        Format::Ntfy => {
            let (root, topic) = path.rsplit_once('/').unwrap_or(("", path));
            if topic.is_empty()
                || topic.len() > 64
                || !topic
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
            {
                return Err(
                    "must end in an ntfy topic of up to 64 letters, digits, - and _".into(),
                );
            }
            for (key, value) in pairs {
                match key {
                    "token" => target.token = Some(value.to_owned()),
                    "priority" => target.priority = Some(priority(value)?),
                    _ => return Err("takes only token and priority after ? for ntfy".into()),
                }
            }
            // Published as JSON to the server's root, with the topic in the body.
            target.url = format!("{origin}{root}/");
            secrets.push(topic.to_owned());
            target.topic = Some(topic.to_owned());
        }
        Format::Telegram => {
            let bot = path.strip_suffix("/sendMessage").unwrap_or(path);
            let Some(token) = bot
                .strip_prefix("/bot")
                .filter(|token| !token.is_empty() && !token.contains('/'))
            else {
                return Err("must be https://api.telegram.org/bot<token>/sendMessage?chat_id=<chat> for Telegram".into());
            };
            for (key, value) in pairs {
                match key {
                    "chat_id" => target.chat_id = Some(value.to_owned()),
                    "message_thread_id" => {
                        target.thread_id =
                            Some(value.parse().map_err(|_| {
                                "takes a whole number as message_thread_id".to_owned()
                            })?);
                    }
                    _ => {
                        return Err(
                            "takes only chat_id and message_thread_id after ? for Telegram".into(),
                        );
                    }
                }
            }
            if target.chat_id.is_none() {
                return Err("needs ?chat_id=<chat> for Telegram".into());
            }
            target.url = format!("{origin}/bot{token}/sendMessage");
            secrets.push(token.to_owned());
        }
        Format::Discord | Format::Slack | Format::Json => {}
    }
    secrets.retain(|secret| !secret.is_empty());
    // An answer in JSON may write the slashes of a path as `\/`.
    let escaped: Vec<String> = secrets
        .iter()
        .filter(|secret| secret.contains('/'))
        .map(|secret| secret.replace('/', "\\/"))
        .collect();
    secrets.extend(escaped);
    secrets.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    secrets.dedup();
    target.secrets = secrets;
    Ok(target)
}

/// The format of an address without a prefix, from its host.
fn detect(host: &str, path: &str) -> Format {
    let name = if host.starts_with('[') {
        host
    } else {
        host.split(':').next().unwrap_or(host)
    };
    match name {
        "ntfy.sh" => Format::Ntfy,
        "discord.com" | "discordapp.com" | "ptb.discord.com" | "canary.discord.com"
            if path.starts_with("/api/webhooks/") =>
        {
            Format::Discord
        }
        "hooks.slack.com" => Format::Slack,
        "api.telegram.org" => Format::Telegram,
        _ => Format::Json,
    }
}

/// The `key=value` pairs after `?`, as they are written.
fn parameters(query: &str) -> std::result::Result<Vec<(&str, &str)>, String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            pair.split_once('=')
                .filter(|(key, _)| !key.is_empty())
                .ok_or_else(|| "has a parameter without a value after ?".to_owned())
        })
        .collect()
}

/// An ntfy priority, as a number from 1 to 5 or by ntfy's names for them.
fn priority(value: &str) -> std::result::Result<u8, String> {
    match value {
        "1" | "min" => Ok(1),
        "2" | "low" => Ok(2),
        "3" | "default" => Ok(3),
        "4" | "high" => Ok(4),
        "5" | "max" | "urgent" => Ok(5),
        _ => Err("takes a priority from 1 to 5, or min, low, default, high or max".into()),
    }
}

/// `DASHBOARD_URL`: the address the dashboard is reached at, for the links notices carry, or
/// `None` without one.
pub(crate) fn dashboard_url(value: Option<&str>) -> Result<Option<String>> {
    let value = value.unwrap_or_default().trim().trim_end_matches('/');
    if value.is_empty() {
        return Ok(None);
    }
    let valid = value.len() <= URL_LIMIT
        && value.bytes().all(|b| allowed(b) && !b";?".contains(&b))
        && split(value)
            .is_ok_and(|(_, authority, _)| !authority.contains('@') && host_allowed(authority));
    if !valid {
        bail!(
            "DASHBOARD_URL must be the dashboard's http or https address, such as https://thoric.lan:8443, or empty"
        );
    }
    Ok(Some(value.into()))
}

#[cfg(test)]
#[path = "target_tests.rs"]
mod tests;
