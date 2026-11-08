//! Newer builds for a manager that follows `edge`, the tag every build of `master` moves.
//!
//! An edge build carries the version of the release before it, so the release check finds no
//! newer release while `master` moves on. A manager whose container was created from a reference
//! that names the `edge` tag, such as `ghcr.io/co3moz/thoric:edge`, asks Docker at each look which
//! image that tag names now: the Engine API's `/distribution/<reference>/json` resolves it in the
//! registry, with the daemon's own proxy and credentials, as the pull of `thoric update` does, and
//! downloads nothing. When the digest it names is not this manager's image, a newer build is out,
//! and the dashboard offers the update, which pulls the same reference again. A manager pinned to
//! one build, by `sha-<commit>` or a digest, follows no tag, and neither does one outside the
//! container `MANAGER_CONTAINER` names.
use crate::App;
use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use std::{path::Path, time::Duration};

/// The tag every build of `master` moves.
pub(crate) const TAG: &str = "edge";
/// Docker answers each request within this, resolving the tag in the registry included.
const LIMIT: Duration = Duration::from_secs(30);

/// What the looks found for a manager that follows `edge`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Edge {
    /// The reference the manager's container was created from.
    pub image: String,
    /// Whether the tag names another image than this manager's; a failed look keeps the answer
    /// of the one before.
    pub newer: bool,
    /// Why the last look failed.
    pub error: Option<String>,
}

/// What one look found.
pub(crate) enum Look {
    /// The manager's container was not created from an `edge` tag, or the manager runs outside
    /// the container `MANAGER_CONTAINER` names.
    NotFollowing,
    /// Whether `image`, which names the tag, names another image now, or why Docker could not
    /// tell.
    Following { image: String, newer: Result<bool> },
    /// The manager's own container did not read, so whether it follows the tag is not known.
    Unknown(anyhow::Error),
}

impl Look {
    /// What is known after this look, given what was known before it.
    pub(crate) fn apply(self, before: Option<Edge>) -> Option<Edge> {
        match self {
            Look::NotFollowing => None,
            Look::Following {
                image,
                newer: Ok(newer),
            } => Some(Edge {
                image,
                newer,
                error: None,
            }),
            Look::Following {
                image,
                newer: Err(error),
            } => {
                let newer = before.is_some_and(|edge| edge.image == image && edge.newer);
                Some(Edge {
                    image,
                    newer,
                    error: Some(format!("{error:#}")),
                })
            }
            Look::Unknown(error) => before.map(|edge| Edge {
                error: Some(format!("{error:#}")),
                ..edge
            }),
        }
    }
}

/// Whether a reference names the `edge` tag. One pinned to a digest names that image alone, also
/// when it names the tag too.
pub(crate) fn follows(reference: &str) -> bool {
    !reference.contains('@')
        && reference
            .rsplit_once(':')
            .is_some_and(|(_, tag)| tag == TAG)
}

/// The digests that name an image, from its inspect: its ID, which is the digest of the index in
/// the containerd image store, and the digest of each repository it was pulled from, which
/// Docker records as the index's digest for an image of several platforms.
pub(crate) fn digests(image: &Value) -> Vec<&str> {
    let repositories = image["RepoDigests"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|repository| repository.rsplit_once('@').map(|(_, digest)| digest));
    image["Id"]
        .as_str()
        .into_iter()
        .chain(repositories)
        .collect()
}

/// The digest of the image a tag names, from Docker's `/distribution` answer.
pub(crate) fn named(answer: &Value) -> Result<&str> {
    answer["Descriptor"]["digest"]
        .as_str()
        .filter(|digest| digest.starts_with("sha256:"))
        .context("Docker did not name the image the tag points to")
}

/// Ask Docker what this manager was created from and what that tag names now.
pub(crate) async fn look(app: &App) -> Look {
    let (Some(manager), Some(socket)) = (&app.config.manager, &app.config.docker_socket) else {
        return Look::NotFollowing;
    };
    let path = format!("/containers/{}/json", crate::pulls::query_value(manager));
    let container = match get(socket, &path).await {
        Ok(container) => container,
        Err(error) => {
            return Look::Unknown(error.context("Could not read the manager's container"));
        }
    };
    if container["Config"]["Labels"]["thoric.manager"] != "true" {
        return Look::NotFollowing;
    }
    let Some(image) = container["Config"]["Image"]
        .as_str()
        .filter(|image| follows(image))
    else {
        return Look::NotFollowing;
    };
    let newer = async {
        let id = container["Image"].as_str().unwrap_or_default();
        let path = format!("/images/{}/json", crate::pulls::query_value(id));
        let own = get(socket, &path)
            .await
            .context("Could not read the manager's image")?;
        let path = format!("/distribution/{}/json", crate::pulls::query_value(image));
        let now = get(socket, &path)
            .await
            .with_context(|| format!("Docker could not look up {image}"))?;
        let now = named(&now)?;
        anyhow::Ok(!digests(&own).contains(&now))
    }
    .await;
    Look::Following {
        image: image.into(),
        newer,
    }
}

/// One Engine API request over its own connection, within [`LIMIT`].
async fn get(socket: &Path, path: &str) -> Result<Value> {
    let io = crate::pulls::connect(socket)
        .await
        .with_context(|| format!("Could not reach Docker at {}", socket.display()))?;
    tokio::time::timeout(LIMIT, crate::usage::engine::get(io, path))
        .await
        .map_err(|_| anyhow!("Docker did not answer within {} seconds", LIMIT.as_secs()))?
}

#[cfg(test)]
#[path = "edge_tests.rs"]
mod tests;
