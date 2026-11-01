//! Pins down how a desktop's screen is resized over RFB: the screens are asked for through
//! `ExtendedDesktopSize`, `SetDesktopSize` keeps the first screen's ID and flags, pixels, bells
//! and the server's own layout reports on the way are read past, and a refusal says why. After a
//! start, only a desktop created at another size than its settings name, whose screen still has
//! that size, is resized.
use super::*;
use serde_json::json;
use tokio::net::{TcpListener, TcpStream};

/// Greet one client as Xtigervnc does, with a `width` by `height` screen of 32-bit pixels, and
/// check that it shares the desktop.
async fn greet(stream: &mut TcpStream, (width, height): (u16, u16)) {
    stream.write_all(b"RFB 003.008\n").await.unwrap();
    let mut version = [0; 12];
    stream.read_exact(&mut version).await.unwrap();
    stream.write_all(&[1, 1]).await.unwrap();
    assert_eq!(stream.read_u8().await.unwrap(), 1);
    stream.write_all(&0u32.to_be_bytes()).await.unwrap();
    assert_eq!(
        stream.read_u8().await.unwrap(),
        1,
        "the connection is shared"
    );
    let mut init = Vec::new();
    init.extend(width.to_be_bytes());
    init.extend(height.to_be_bytes());
    init.extend([32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]);
    init.extend(4u32.to_be_bytes());
    init.extend(b"test");
    stream.write_all(&init).await.unwrap();
}

/// A VNC server for one client on a free port, which greets it with a `size` screen and then
/// runs `then`. The task fails when one of its checks does.
async fn vnc<F, Fut>(size: (u16, u16), then: F) -> (String, tokio::task::JoinHandle<()>)
where
    F: FnOnce(TcpStream) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        greet(&mut stream, size).await;
        then(stream).await;
    });
    (address, task)
}

/// An `ExtendedDesktopSize` rectangle.
fn layout_rect(
    reason: u16,
    status: u16,
    (width, height): (u16, u16),
    screen: (u32, u32),
) -> Vec<u8> {
    let mut rect = Vec::new();
    for value in [reason, status, width, height] {
        rect.extend(value.to_be_bytes());
    }
    rect.extend((-308i32).to_be_bytes());
    rect.extend([1, 0, 0, 0]);
    rect.extend(screen.0.to_be_bytes());
    rect.extend([0; 4]);
    rect.extend(width.to_be_bytes());
    rect.extend(height.to_be_bytes());
    rect.extend(screen.1.to_be_bytes());
    rect
}

/// A FramebufferUpdate of `rects`.
fn update(rects: &[Vec<u8>]) -> Vec<u8> {
    let mut message = vec![0, 0];
    message.extend((rects.len() as u16).to_be_bytes());
    for rect in rects {
        message.extend(rect);
    }
    message
}

/// The screen Xtigervnc names, with flags a client has to send back as they are.
const SCREEN: (u32, u32) = (1_804_289_383, 0x10);

/// A server at 1280x720 that expects a change to 1600x900 and answers it with `status`.
async fn resizing(status: u16) -> (String, tokio::task::JoinHandle<()>) {
    vnc((1280, 720), move |mut stream| async move {
        let mut asked = [0; 18];
        stream.read_exact(&mut asked).await.unwrap();
        // SetEncodings with ExtendedDesktopSize alone, then a full request for one pixel.
        assert_eq!(
            asked,
            [2, 0, 0, 1, 255, 255, 254, 204, 3, 0, 0, 0, 0, 0, 0, 1, 0, 1]
        );
        // A bell and a pixel come before the screens, which are read past.
        let mut pixel = vec![0, 0, 0, 0, 0, 1, 0, 1];
        pixel.extend(0i32.to_be_bytes());
        pixel.extend([9; 4]);
        let mut reply = vec![2];
        reply.extend(update(&[pixel, layout_rect(0, 0, (1280, 720), SCREEN)]));
        stream.write_all(&reply).await.unwrap();
        let mut set = [0; 34];
        stream.read_exact(&mut set).await.unwrap();
        let mut expected = vec![251, 0, 6, 64, 3, 132, 1, 0];
        expected.extend(SCREEN.0.to_be_bytes());
        expected.extend([0, 0, 0, 0, 6, 64, 3, 132]);
        expected.extend(SCREEN.1.to_be_bytes());
        expected.extend([3, 1, 0, 0, 0, 0, 0, 1, 0, 1]);
        assert_eq!(set.to_vec(), expected);
        // Xtigervnc reports its own steps before it answers the client that asked.
        let mut answer = update(&[layout_rect(0, 0, (1600, 900), SCREEN)]);
        answer.extend(update(&[layout_rect(1, status, (1600, 900), SCREEN)]));
        stream.write_all(&answer).await.unwrap();
    })
    .await
}

#[tokio::test]
async fn the_first_screen_takes_the_whole_new_size() {
    let (address, server) = resizing(0).await;
    let screen = Screen::open(&address).await.unwrap();
    assert_eq!(screen.size(), (1280, 720));
    screen.resize((1600, 900)).await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn a_refusal_says_why() {
    let (address, server) = resizing(1).await;
    let screen = Screen::open(&address).await.unwrap();
    let error = screen.resize((1600, 900)).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "the VNC server does not allow its size to change"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn a_server_that_never_reports_its_screens_is_not_waited_for_forever() {
    let (address, server) = vnc((1280, 720), |mut stream| async move {
        let mut asked = [0; 18];
        stream.read_exact(&mut asked).await.unwrap();
        stream.write_all(&[2; MESSAGES]).await.unwrap();
        // The client gives up and closes the connection without asking for a size.
        let mut rest = Vec::new();
        stream.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty(), "{rest:?}");
    })
    .await;
    let screen = Screen::open(&address).await.unwrap();
    let error = screen.resize((1600, 900)).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "The VNC server did not report its screens"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn a_screen_is_resized_after_a_start_only_while_it_has_the_size_it_was_created_with() {
    let (address, server) = resizing(0).await;
    assert!(settle(&address, (1280, 720), (1600, 900)).await.unwrap());
    server.await.unwrap();
    // A viewer resized this one to their window, so it stays as they left it.
    let (address, server) = vnc((1366, 768), |mut stream| async move {
        let mut rest = Vec::new();
        stream.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty(), "{rest:?}");
    })
    .await;
    assert!(!settle(&address, (1280, 720), (1600, 900)).await.unwrap());
    server.await.unwrap();
}

fn container(resolution: &str) -> Container {
    Container {
        id: "e".into(),
        container: "thoric-e".into(),
        private_network: "thoric-e-private".into(),
        egress_network: "thoric-e-egress".into(),
        source_image_id: "sha256:desktop".into(),
        created_at: 1,
        spec: crate::model::ContainerSpec {
            resolution: resolution.into(),
            ..crate::test_support::spec()
        },
        last_error: None,
        desktop: None,
        working_image: None,
        setup: None,
    }
}

#[tokio::test]
async fn only_a_running_desktop_created_at_another_size_is_looked_at() {
    let created = |env: &[&str], running: bool| json!({"Config": {"Env": env}, "State": {"Running": running}});
    let at_720 = ["PATH=/usr/bin", "DESKTOP_RESOLUTION=1280x720"];
    assert_eq!(created_with(&created(&at_720, true)), Some((1280, 720)));
    assert_eq!(created_with(&created(&["PATH=/usr/bin"], true)), None);
    assert_eq!(created_with(&json!({})), None);
    assert_eq!(size("1600x900").unwrap(), (1600, 900));
    assert!(size("1600").is_err());
    // None of these needs the VNC server, which the container here does not have.
    let app = crate::test_support::app(false);
    for (resolution, info) in [
        ("1280x720", created(&at_720, true)),
        ("1600x900", created(&at_720, false)),
        ("1600x900", created(&["PATH=/usr/bin"], true)),
    ] {
        let kept = app.keep_resolution(&container(resolution), &info).await;
        assert_eq!(kept.unwrap(), None, "{resolution} {info}");
    }
    // One that needs it is looked for on its private network.
    let error = app
        .keep_resolution(&container("1600x900"), &created(&at_720, true))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Missing private network address");
}
