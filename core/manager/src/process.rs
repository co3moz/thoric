use anyhow::{Context, Result};
use std::{
    process::{Output, Stdio},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

#[derive(Clone, Copy)]
pub enum Overflow {
    Reject,
    Tail,
}

/// What [`Overflow::Reject`] fails with, which a caller can tell from other failures.
#[derive(Debug)]
pub struct Exceeded(pub usize);
impl std::fmt::Display for Exceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Docker output exceeded the {}-byte limit", self.0)
    }
}
impl std::error::Error for Exceeded {}

pub(crate) async fn read_limited(
    mut read: impl AsyncRead + Unpin,
    limit: usize,
    overflow: Overflow,
) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    // On the heap: as an array it would sit twice in the future of every Docker command, and a
    // debug build reserves room for such a future in its caller's stack frame, in the MCP
    // dispatch once for every arm of its match (mcp_screenshot_fits_in_half_a_worker_stack).
    let mut chunk = vec![0; 8192];
    loop {
        let n = read.read(&mut chunk).await?;
        if n == 0 {
            return Ok(result);
        }
        if result.len() + n > limit {
            match overflow {
                Overflow::Reject => return Err(Exceeded(limit).into()),
                Overflow::Tail => {
                    let discard = (result.len() + n - limit).min(result.len());
                    result.drain(..discard);
                }
            }
        }
        result.extend_from_slice(&chunk[..n]);
        if result.len() > limit {
            result.drain(..result.len() - limit);
        }
    }
}
pub async fn capture(
    command: Command,
    timeout: Duration,
    limit: usize,
    overflow: Overflow,
) -> Result<Output> {
    capture_input(command, timeout, limit, overflow, None).await
}
pub async fn capture_input(
    command: Command,
    timeout: Duration,
    limit: usize,
    overflow: Overflow,
    input: Option<Vec<u8>>,
) -> Result<Output> {
    run(command, timeout, limit, overflow, input, false).await
}
/// Like [`capture_input`], but stdin stays open after the input until the command has ended, for
/// a helper that takes its closing as the sign that nobody waits for its answer: ending the
/// Docker CLI on a timeout, or with a request that was dropped, closes it and stops the helper
/// inside the container, which killing the CLI alone does not.
pub async fn capture_held(
    command: Command,
    timeout: Duration,
    limit: usize,
    overflow: Overflow,
    input: Vec<u8>,
) -> Result<Output> {
    run(command, timeout, limit, overflow, Some(input), true).await
}
async fn run(
    mut command: Command,
    timeout: Duration,
    limit: usize,
    overflow: Overflow,
    input: Option<Vec<u8>>,
    hold: bool,
) -> Result<Output> {
    let mut child = command
        .kill_on_drop(true)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Could not start Docker CLI")?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let stdin = child.stdin.take();
    let result = tokio::time::timeout(timeout, async {
        let (stdout, stderr, status, _) = tokio::try_join!(
            read_limited(stdout, limit, overflow),
            read_limited(stderr, limit, overflow),
            async { Ok::<_, anyhow::Error>(child.wait().await?) },
            async move {
                // A process that exits before consuming its input closes the pipe. Its exit
                // status and stderr describe that better than the resulting EPIPE.
                let (Some(mut stdin), Some(input)) = (stdin, input) else {
                    return Ok::<_, anyhow::Error>(None);
                };
                let delivered = if hold {
                    stdin.write_all(&input).await
                } else {
                    deliver(&mut stdin, &input).await
                };
                if let Err(e) = delivered {
                    tracing::debug!("Process input was not fully delivered: {e}");
                }
                // Held open by the join until the command has ended, when asked to.
                Ok(hold.then_some(stdin))
            }
        )?;
        Ok::<_, anyhow::Error>(Output {
            status,
            stdout,
            stderr,
        })
    })
    .await;
    match result {
        Ok(Ok(output)) => Ok(output),
        failure => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            match failure {
                Ok(Err(e)) => Err(e),
                Err(_) => Err(timed_out()),
                _ => unreachable!(),
            }
        }
    }
}

/// A Docker command that did not finish within its limit: Docker did not answer in time.
pub(crate) fn timed_out() -> anyhow::Error {
    crate::error::Fault::error(
        axum::http::StatusCode::GATEWAY_TIMEOUT,
        "Docker operation timed out",
    )
}

async fn deliver(stdin: &mut tokio::process::ChildStdin, input: &[u8]) -> std::io::Result<()> {
    stdin.write_all(input).await?;
    stdin.shutdown().await
}

// Preserve split UTF-8 sequences; non-text script output must not abort a successful build.
#[derive(Default)]
pub struct TextDecoder {
    pending: Vec<u8>,
}
impl TextDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut text = String::new();
        let mut offset = 0;
        while offset < self.pending.len() {
            match std::str::from_utf8(&self.pending[offset..]) {
                Ok(s) => {
                    text.push_str(s);
                    offset = self.pending.len();
                }
                Err(e) => {
                    let end = offset + e.valid_up_to();
                    text.push_str(&String::from_utf8_lossy(&self.pending[offset..end]));
                    offset = end;
                    match e.error_len() {
                        Some(n) => {
                            text.push('\u{fffd}');
                            offset += n;
                        }
                        None => break,
                    }
                }
            }
        }
        self.pending.drain(..offset);
        text
    }
    pub fn finish(&mut self) -> String {
        let tail = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        tail
    }
}
#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
