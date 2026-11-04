//! A container's two networks, the internal private one it shares with the manager alone and the
//! one that reaches the internet, and its addresses there. `isolation` keeps the private one off
//! the Docker host.
use super::{address_pool::DESKTOP_PREFIX, isolation::Reach};
use crate::{App, model::*};
use anyhow::{Context, Result, bail};
use serde_json::Value;

impl App {
    pub fn check_network(env: &Container, info: &Value, internal: bool) -> Result<()> {
        if info["Labels"][LABEL] != "true"
            || info["Labels"]["thoric.environment"] != env.id
            || info["Internal"] != internal
            || info["Driver"] != "bridge"
        {
            bail!("Network ownership or type does not match");
        }
        Ok(())
    }
    pub(super) async fn ensure_network(
        &self,
        env: &Container,
        name: &str,
        reach: Reach,
    ) -> Result<Value> {
        if let Some(info) = self.optional_inspect("network", name).await? {
            Self::check_network(env, &info, reach != Reach::Internet)?;
            return Ok(info);
        }
        self.create_network(env, name, reach, None).await
    }
    /// Create one of a container's networks, labeled as its own, in `subnet` or in a subnet of
    /// `DESKTOP_ADDRESS_POOL` of its own.
    pub(super) async fn create_network(
        &self,
        env: &Container,
        name: &str,
        reach: Reach,
        subnet: Option<&str>,
    ) -> Result<Value> {
        let labels = [
            format!("{LABEL}=true"),
            format!("thoric.environment={}", env.id),
        ];
        self.make_network(name, &labels, reach, subnet, DESKTOP_PREFIX)
            .await
    }
    pub async fn set_internet(&self, env: &mut Container, enabled: bool) -> Result<()> {
        let info = self.owned(env).await?;
        let connected = info["NetworkSettings"]["Networks"]
            .get(&env.egress_network)
            .is_some();
        let network = self.inspect("network", &env.egress_network).await?;
        Self::check_network(env, &network, false)?;
        let container_id = info["Id"].as_str().context("Missing container ID")?;
        let network_id = network["Id"].as_str().context("Missing network ID")?;
        if enabled && !connected {
            self.connect_network(env, network_id, container_id).await?;
        } else if !enabled && connected {
            self.docker(&["network", "disconnect", network_id, container_id])
                .await?;
        }
        env.spec.internet = enabled;
        self.db.put(CONTAINERS, &env.id, env)?;
        Ok(())
    }
    pub async fn address(&self, env: &Container) -> Result<String> {
        let info = self.owned(env).await?;
        if info["State"]["Running"] != true {
            bail!("Container is not running");
        }
        Ok(format!("{}:5900", private_ip(env, &info)?))
    }
}

/// The desktop's address on its private network, which only the manager shares with it, from
/// `docker inspect` of its container.
pub(crate) fn private_ip(env: &Container, info: &Value) -> Result<std::net::Ipv4Addr> {
    info["NetworkSettings"]["Networks"][&env.private_network]["IPAddress"]
        .as_str()
        .context("Missing private network address")?
        .parse()
        .context("Invalid Docker IP address")
}
/// The IPv4 subnet of a desktop's private network, from `docker network inspect`. The desktop's
/// VNC server listens only on its address there, since VNC asks no password and the private
/// network is the one only the manager shares with it.
pub(crate) fn private_subnet(network: &Value) -> Option<&str> {
    network["IPAM"]["Config"]
        .as_array()?
        .iter()
        .filter_map(|config| config["Subnet"].as_str())
        .find(|subnet| {
            subnet
                .split_once('/')
                .is_some_and(|(address, _)| address.parse::<std::net::Ipv4Addr>().is_ok())
        })
}
#[cfg(test)]
#[path = "networks_tests.rs"]
mod tests;
