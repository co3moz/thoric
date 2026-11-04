//! What the Docker Engine lacks of what Thoric uses, which the Management page names. An older
//! engine works, and the manager does without what it cannot have and logs that, but nobody reads
//! the log. Who closes the gap depends on who keeps Docker: in the WSL package `thoric update`
//! keeps Docker Engine current, anywhere else whoever runs the Docker host does.
use super::isolation::{ISOLATING_API, MAC_API, api_at_least};
use crate::App;
use serde_json::{Value, json};
use std::time::Duration;

/// How long the page waits for Docker to name its version; one that does not answer by then
/// is reported elsewhere.
const LIMIT: Duration = Duration::from_secs(5);

/// What an engine of an API version cannot do of what Thoric uses, by the name the page shows it
/// under: `isolation`, a desktop's private network without an address on the Docker host (Docker
/// Engine 28.0), and `mac_address`, a desktop's MAC address on its internet network when it starts
/// again (Docker Engine 29). An API version that does not read lacks both.
pub(crate) fn lacking(api: &str) -> Vec<&'static str> {
    [("isolation", ISOLATING_API), ("mac_address", MAC_API)]
        .into_iter()
        .filter(|(_, least)| !api_at_least(api, *least))
        .map(|(name, _)| name)
        .collect()
}

impl App {
    /// `docker` in `GET /api/configuration`: the engine's `version` and `api`, what it `lacks`, and
    /// whether the WSL `package` keeps Docker Engine current; `None` when it lacks nothing or does
    /// not answer.
    pub(crate) async fn engine_gaps(&self) -> Option<Value> {
        let format = [
            "version",
            "--format",
            "{{.Server.Version}} {{.Server.APIVersion}}",
        ];
        let out = tokio::time::timeout(LIMIT, self.docker(&format))
            .await
            .ok()?
            .ok()?;
        let out = String::from_utf8_lossy(&out);
        let (version, api) = out.trim().split_once(' ')?;
        let lacks = lacking(api);
        (!lacks.is_empty()).then(|| {
            json!({
                "version": version,
                "api": api,
                "lacks": lacks,
                "package": crate::releases::installable(&self.config.update_dir),
            })
        })
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
