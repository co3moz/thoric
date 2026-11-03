//! One POST through `curl`, the client the manager's image carries for its health check and the
//! release check. The address, the headers and the body reach curl on stdin as its configuration,
//! so a credential is never on a command line, where any process of the host could read it, and
//! only the fixed options are. Like the release check, curl goes straight to the address: the
//! installation's `PROXY_URL` is the desktops' proxy.
use super::{Target, payload::Request};
use anyhow::{Context, bail};
use serde_json::Value;
use std::{process::Stdio, time::Duration};
use tokio::io::AsyncWriteExt;

pub(super) const PROGRAM: &str = "curl";
/// What curl is told on its command line; everything else comes on stdin. It reads no `.curlrc`
/// (`-q`, which has to come first), takes `[]` and `{}` in an address as they are rather than as
/// a list of addresses, and follows no redirect, so a body never goes to another address than
/// the one written.
const OPTIONS: &[&str] = &[
    "-q",
    "--globoff",
    "--config",
    "-",
    "--silent",
    "--show-error",
    "--proto",
    "=http,https",
    "--connect-timeout",
    "5",
    "--max-time",
    "15",
    "--max-filesize",
    "65536",
    "--write-out",
    "\\n%{http_code}",
];
/// curl gives up within its own limits; the process is stopped here in any case.
const LIMIT: Duration = Duration::from_secs(20);
/// How much of an answer or an error is kept, in characters.
const ANSWER_LIMIT: usize = 300;

/// How an address took a notice.
#[derive(Debug, Default)]
pub(super) struct Delivery {
    /// The HTTP status it answered with, when it answered.
    pub status: Option<u16>,
    /// The start of its answer.
    pub answer: String,
    /// Why it did not take the notice, `None` when it answered with a 2xx status.
    pub error: Option<String>,
}

/// POST `request` to `target` with `program`, and say how it went, without any part of the
/// address in what is said.
pub(super) async fn deliver(program: &str, target: &Target, request: &Request) -> Delivery {
    match post(program, request).await {
        Ok((status, answer)) => {
            let answer = short(&target.redact(&answer));
            let error = (!(200..300).contains(&status)).then(|| {
                if answer.is_empty() {
                    format!("{} answered HTTP {status}", target.describe())
                } else {
                    format!("{} answered HTTP {status}: {answer}", target.describe())
                }
            });
            Delivery {
                status: Some(status),
                answer,
                error,
            }
        }
        Err(error) => Delivery {
            error: Some(short(&target.redact(&format!("{error:#}")))),
            ..Delivery::default()
        },
    }
}

async fn post(program: &str, request: &Request) -> anyhow::Result<(u16, String)> {
    let mut child = tokio::process::Command::new(program)
        .args(OPTIONS)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("Could not run curl")?;
    let mut stdin = child.stdin.take().context("curl took no input")?;
    let config = config(request);
    let output = tokio::time::timeout(LIMIT, async move {
        stdin.write_all(config.as_bytes()).await?;
        drop(stdin);
        child.wait_with_output().await
    })
    .await
    .context("The address did not answer in time")?
    .context("Could not run curl")?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        bail!(
            "{}",
            match error.trim() {
                "" => format!("curl ended with {}", output.status),
                error => error.to_owned(),
            }
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (answer, status) = stdout.rsplit_once('\n').unwrap_or(("", &stdout));
    match status.trim().parse::<u16>() {
        Ok(status) if status != 0 => Ok((status, answer.to_owned())),
        _ => bail!("The address gave no HTTP answer"),
    }
}

/// curl's configuration for `request`, as it reads it on stdin: one quoted value a line, with
/// the body as JSON in ASCII alone.
pub(super) fn config(request: &Request) -> String {
    let mut config = format!("url = \"{}\"\n", quoted(&request.url));
    for header in &request.headers {
        config.push_str(&format!("header = \"{}\"\n", quoted(header)));
    }
    // Some receivers never answer 100 Continue, and curl would wait for it.
    config.push_str("header = \"Expect:\"\n");
    config.push_str(&format!(
        "data-binary = \"{}\"\n",
        quoted(&ascii_json(&request.body))
    ));
    config
}

/// A value inside the double quotes of curl's configuration, which reads `\\` and `\"`.
fn quoted(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// `value` as JSON with every character outside printable ASCII escaped as JSON does, so
/// the configuration line carries no byte curl could read otherwise.
fn ascii_json(value: &Value) -> String {
    let text = value.to_string();
    let mut ascii = String::with_capacity(text.len());
    for c in text.chars() {
        if (' '..='~').contains(&c) {
            ascii.push(c);
        } else {
            let mut units = [0; 2];
            for unit in c.encode_utf16(&mut units) {
                ascii.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    ascii
}

/// The start of `text`, on one line.
fn short(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    super::cut(&text, ANSWER_LIMIT)
}

#[cfg(test)]
#[path = "deliver_tests.rs"]
mod tests;
