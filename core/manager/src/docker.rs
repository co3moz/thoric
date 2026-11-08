//! The Docker CLI the manager runs for every Docker operation, and the check that a container it
//! finds is the one its record names. `lifecycle` creates, provisions, retries, resets and
//! deletes a container, `create_args` holds every setting Docker fixes when it creates one,
//! `networks` a container's private and internet networks, `address_pool` gives every network
//! Thoric makes a small subnet of `DESKTOP_ADDRESS_POOL`, `isolation` keeps internal networks
//! without an address on the Docker host, `engine` names what an older Docker Engine lacks of
//! that, and `logged` runs a command whose output goes into a job's log.
mod address_pool;
mod create_args;
mod engine;
mod isolation;
mod lifecycle;
pub(crate) mod logged;
mod networks;

pub(crate) use address_pool::{GROUP_PREFIX, Subnet, address_pool};
pub(crate) use isolation::{Reach, isolated};
pub(crate) use lifecycle::{ComposeChoice, Creation, image_volumes_mounted};
pub(crate) use logged::LOGGED_LIMIT;
pub(crate) use networks::{private_ip, private_subnet};

use crate::{App, model::*};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::time::Duration;
use tokio::process::Command;

impl App {
    pub fn command(&self, args: &[String]) -> Command {
        let mut cmd = Command::new(&self.config.docker_bin);
        cmd.args(args).kill_on_drop(true);
        cmd
    }
    pub async fn docker(&self, args: &[&str]) -> Result<Vec<u8>> {
        let args = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        self.docker_args(&args).await
    }
    pub async fn docker_args(&self, args: &[String]) -> Result<Vec<u8>> {
        let out = crate::process::capture(
            self.command(args),
            Duration::from_secs(120),
            4 * 1024 * 1024,
            crate::process::Overflow::Reject,
        )
        .await?;
        if !out.status.success() {
            return Err(crate::error::docker_failure(&out.stderr, out.status.code()));
        }
        Ok(out.stdout)
    }
    pub async fn inspect(&self, kind: &str, name: &str) -> Result<Value> {
        let data = self.docker(&[kind, "inspect", name]).await?;
        let mut values: Vec<Value> = serde_json::from_slice(&data)?;
        values.pop().context("Docker object not found")
    }
    pub async fn owned(&self, env: &Container) -> Result<Value> {
        let info = self.inspect("container", &env.container).await?;
        if info["Config"]["Labels"][LABEL] != "true"
            || info["Config"]["Labels"]["thoric.environment"] != env.id
        {
            bail!("Container ownership does not match");
        }
        Ok(info)
    }
    pub async fn optional_inspect(&self, kind: &str, name: &str) -> Result<Option<Value>> {
        match self.inspect(kind, name).await {
            Ok(info) => Ok(Some(info)),
            Err(error) => {
                // Listing must succeed before an inspect failure can mean 'absent'.
                let args = match kind {
                    "container" => vec!["ps", "-a", "--format", "{{.Names}}"],
                    // An image's listing has no `Name`; a reference is its repository and tag.
                    "image" => vec!["image", "ls", "--format", "{{.Repository}}:{{.Tag}}", name],
                    _ => vec![kind, "ls", "--format", "{{.Name}}"],
                };
                let names = self.docker(&args).await?;
                if String::from_utf8_lossy(&names).lines().any(|n| n == name) {
                    return Err(error);
                }
                Ok(None)
            }
        }
    }
    pub fn container(&self, id: &str) -> crate::error::Result<Container> {
        self.db
            .get(CONTAINERS, id)
            .map_err(crate::error::Error::from)
    }
}
