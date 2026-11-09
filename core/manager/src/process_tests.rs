//! Pins down the bounded process capture: UTF-8 split across reads, output limits with
//! tail and reject overflow, and timeouts that cover waiting for the exit.
use super::*;
#[test]
fn utf8_split_and_invalid_bytes_do_not_abort_logs() {
    let mut decoder = TextDecoder::default();
    assert_eq!(decoder.push(&[b'a', 0xc4]), "a");
    assert_eq!(decoder.push(&[0xb0, 0xff, b'b']), "\u{0130}\u{fffd}b");
    assert_eq!(decoder.push(&[0xe2]), "");
    assert_eq!(decoder.finish(), "\u{fffd}");
}
#[tokio::test]
async fn arbitrary_no_newline_output_is_bounded() {
    let bytes = vec![b'x'; 100_000];
    assert!(
        read_limited(&bytes[..], 1024, Overflow::Reject)
            .await
            .is_err()
    );
    let tail = read_limited(&bytes[..], 1024, Overflow::Tail)
        .await
        .unwrap();
    assert_eq!(tail.len(), 1024);
}
#[tokio::test]
async fn command_timeout_includes_waiting_for_exit() {
    #[cfg(windows)]
    let command = {
        let mut c = Command::new("powershell");
        c.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"]);
        c
    };
    #[cfg(not(windows))]
    let command = {
        let mut c = Command::new("sleep");
        c.arg("30");
        c
    };
    let start = std::time::Instant::now();
    assert!(
        capture(command, Duration::from_millis(100), 1024, Overflow::Reject)
            .await
            .unwrap_err()
            .to_string()
            .contains("timed out")
    );
    assert!(start.elapsed() < Duration::from_secs(3));
}

/// A helper that takes the end of its stdin as the sign that nobody waits for it gets its input
/// and an stdin that stays open while it works; the plain capture closes it after the input.
#[cfg(unix)]
#[tokio::test]
async fn held_input_leaves_stdin_open_until_the_command_ends() {
    // Bash's read answers above 128 when its time runs out and 1 at the end of the input.
    let command = || {
        let mut c = Command::new("bash");
        c.args(["-c", "read line; read -t 0.3 rest; echo \"$line $?\""]);
        c
    };
    let held = capture_held(
        command(),
        Duration::from_secs(10),
        1024,
        Overflow::Reject,
        b"request\n".to_vec(),
    )
    .await
    .unwrap();
    assert!(held.status.success());
    let said = String::from_utf8(held.stdout).unwrap();
    let (line, status) = said.trim().split_once(' ').unwrap();
    assert_eq!(line, "request");
    assert!(status.parse::<i32>().unwrap() > 128, "{said}");
    let closed = capture_input(
        command(),
        Duration::from_secs(10),
        1024,
        Overflow::Reject,
        Some(b"request\n".to_vec()),
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(closed.stdout).unwrap().trim(),
        "request 1"
    );
}
