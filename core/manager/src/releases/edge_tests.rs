//! Which managers follow `edge`, how a look tells a newer build of it from this manager's own,
//! through Docker's answers about the manager's container, its image and what the tag names now,
//! and what a failed look keeps.
use super::*;
use serde_json::json;

const OWN: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const NEXT: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";

#[test]
fn only_a_reference_that_names_the_edge_tag_follows_it() {
    for reference in [
        "ghcr.io/co3moz/thoric:edge",
        "co3moz/thoric:edge",
        "localhost:5000/thoric:edge",
    ] {
        assert!(follows(reference), "{reference}");
    }
    for reference in [
        "ghcr.io/co3moz/thoric:latest",
        "ghcr.io/co3moz/thoric:0.9.3",
        "ghcr.io/co3moz/thoric:sha-0123456789abcdef0123456789abcdef01234567",
        "ghcr.io/co3moz/thoric",
        "ghcr.io/co3moz/thoric:edge@sha256:2222222222222222222222222222222222222222222222222222222222222222",
        "ghcr.io/co3moz/thoric@sha256:2222222222222222222222222222222222222222222222222222222222222222",
        "localhost:5000/edge",
        "ghcr.io/co3moz/thoric:edge-old",
    ] {
        assert!(!follows(reference), "{reference}");
    }
}

#[test]
fn an_image_is_named_by_its_id_and_the_digest_of_each_repository() {
    // The classic image store records the index's digest for the repository it was pulled from.
    let classic = json!({
        "Id": "sha256:aaaa",
        "RepoDigests": [format!("ghcr.io/co3moz/thoric@{OWN}"), format!("co3moz/thoric@{NEXT}")],
    });
    assert_eq!(digests(&classic), ["sha256:aaaa", OWN, NEXT]);
    // The containerd image store names the image by the index's digest itself.
    assert_eq!(digests(&json!({"Id": OWN, "RepoDigests": null})), [OWN]);
    assert!(digests(&json!({})).is_empty());
}

#[test]
fn the_tag_names_the_digest_of_dockers_descriptor() {
    let answer = json!({"Descriptor": {"mediaType": "application/vnd.oci.image.index.v1+json", "digest": NEXT, "size": 856}, "Platforms": []});
    assert_eq!(named(&answer).unwrap(), NEXT);
    for answer in [json!({}), json!({"Descriptor": {"digest": "md5:abc"}})] {
        assert!(named(&answer).is_err(), "{answer}");
    }
}

#[test]
fn a_failed_look_keeps_what_the_one_before_saw() {
    let image = "ghcr.io/co3moz/thoric:edge";
    let seen = |newer| Look::Following {
        image: image.into(),
        newer: Ok(newer),
    };
    let failed = || Look::Following {
        image: image.into(),
        newer: Err(anyhow!("the registry is unreachable")),
    };
    let newer = seen(true).apply(None).unwrap();
    assert_eq!(
        newer,
        Edge {
            image: image.into(),
            newer: true,
            error: None
        }
    );
    let after = failed().apply(Some(newer.clone())).unwrap();
    assert!(after.newer);
    assert_eq!(after.error.as_deref(), Some("the registry is unreachable"));
    // Nothing seen before, nothing newer.
    assert!(!failed().apply(None).unwrap().newer);
    // A manager created from another reference starts over.
    let other = Look::Following {
        image: "co3moz/thoric:edge".into(),
        newer: Err(anyhow!("unreachable")),
    };
    assert!(!other.apply(Some(newer.clone())).unwrap().newer);
    // Without the manager's own container, what was known stays, with the reason.
    let unknown = Look::Unknown(anyhow!("Docker is away")).apply(Some(newer.clone()));
    assert_eq!(
        unknown,
        Some(Edge {
            error: Some("Docker is away".into()),
            ..newer.clone()
        })
    );
    assert_eq!(Look::Unknown(anyhow!("Docker is away")).apply(None), None);
    // A look that finds the same image again clears the offer and the error.
    assert_eq!(
        seen(false).apply(unknown),
        Some(Edge {
            image: image.into(),
            newer: false,
            error: None
        })
    );
    assert_eq!(Look::NotFollowing.apply(Some(newer)), None);
}

/// Docker's Engine API on a socket of its own, answering the three requests of a look: the
/// manager's container, created from `reference` with image `OWN`, that image, pulled from the
/// tag, and the digest the tag names now, or a 404 when `now` is `None`.
#[cfg(unix)]
fn engine(reference: &'static str, manager: bool, now: Option<&'static str>) -> std::path::PathBuf {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let dir = std::env::temp_dir().join(format!("thoric-edge-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let listener = tokio::net::UnixListener::bind(dir.join("docker.sock")).unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut request = [0; 2048];
                let read = stream.read(&mut request).await.unwrap_or_default();
                let request = String::from_utf8_lossy(&request[..read]).into_owned();
                let path = request.split(' ').nth(1).unwrap_or_default().to_owned();
                let labels = if manager {
                    json!({"thoric.manager": "true"})
                } else {
                    json!({})
                };
                let answer = match path.as_str() {
                    "/containers/thoric-manager/json" => Some(json!({
                        "Image": OWN,
                        "Config": {"Image": reference, "Labels": labels},
                    })),
                    _ if path == format!("/images/{OWN}/json") => Some(json!({
                        "Id": OWN,
                        "RepoDigests": [format!("ghcr.io/co3moz/thoric@{OWN}")],
                    })),
                    _ if path == format!("/distribution/{reference}/json") => {
                        now.map(|digest| json!({"Descriptor": {"digest": digest}}))
                    }
                    _ => None,
                };
                let (status, body) = match answer {
                    Some(answer) => ("200 OK", answer.to_string()),
                    None => (
                        "404 Not Found",
                        json!({"message": format!("no such thing: {path}")}).to_string(),
                    ),
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    dir.join("docker.sock")
}

#[cfg(unix)]
fn app_on(socket: Option<std::path::PathBuf>) -> std::sync::Arc<App> {
    let mut app = crate::test_support::app(false);
    let config = &mut std::sync::Arc::get_mut(&mut app).unwrap().config;
    config.manager = Some("thoric-manager".into());
    config.docker_socket = socket;
    app
}

#[cfg(unix)]
#[tokio::test]
async fn a_look_compares_what_the_tag_names_with_the_managers_image() {
    let image = "ghcr.io/co3moz/thoric:edge";
    let newer = |look: Look| match look {
        Look::Following { image, newer } => (image, newer.map_err(|error| format!("{error:#}"))),
        _ => panic!("the manager follows {image}"),
    };
    assert_eq!(
        newer(look(&app_on(Some(engine(image, true, Some(OWN))))).await),
        (image.to_owned(), Ok(false))
    );
    assert_eq!(
        newer(look(&app_on(Some(engine(image, true, Some(NEXT))))).await),
        (image.to_owned(), Ok(true))
    );
    // A registry that does not answer for the tag fails the look, naming the reference.
    let (_, failed) = newer(look(&app_on(Some(engine(image, true, None)))).await);
    let error = failed.unwrap_err();
    assert!(
        error.contains(&format!("Docker could not look up {image}")),
        "{error}"
    );

    // A manager pinned to a release, or a container that is not the manager, follows nothing.
    let pinned = look(&app_on(Some(engine(
        "ghcr.io/co3moz/thoric:0.9.3",
        true,
        Some(NEXT),
    ))))
    .await;
    assert!(matches!(pinned, Look::NotFollowing));
    let stranger = look(&app_on(Some(engine(image, false, Some(NEXT))))).await;
    assert!(matches!(stranger, Look::NotFollowing));
    // Without MANAGER_CONTAINER or a socket the manager cannot tell, and follows nothing.
    let mut outside = app_on(Some(engine(image, true, Some(NEXT))));
    std::sync::Arc::get_mut(&mut outside)
        .unwrap()
        .config
        .manager = None;
    assert!(matches!(look(&outside).await, Look::NotFollowing));
    assert!(matches!(look(&app_on(None)).await, Look::NotFollowing));
    // A Docker that is away leaves it unknown.
    let away = std::env::temp_dir()
        .join("thoric-edge-missing")
        .join("docker.sock");
    assert!(matches!(look(&app_on(Some(away))).await, Look::Unknown(_)));
}
