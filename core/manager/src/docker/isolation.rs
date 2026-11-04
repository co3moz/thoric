//! Internal networks without an address on the Docker host. Docker gives an internal bridge
//! network an address on the host all the same, so any program on the host reaches what listens
//! in it, such as a desktop's VNC server, which asks no password, on its private network. In WSL
//! every distribution shares one network namespace, so that is any program in any of the user's
//! distributions. Docker Engine 28.0 (API 1.48) can leave the bridge without an address: gateway
//! mode `isolated`, which only an internal network takes and an older engine refuses. Containers
//! on such a network still reach each other, and the manager reaches a desktop from its own
//! container, so a desktop's private network and a group's network are made that way where the
//! engine can.
//!
//! A manager that does not run in `MANAGER_CONTAINER`, as in some development setups, reaches a
//! desktop through the host's address on its private network, so the networks it makes keep one.
//! It cannot reach a desktop whose network a manager in its container made without, which it says.
//!
//! A network made with an address on the host is made again without one only while nothing that
//! runs needs it, since a desktop's VNC server listens on the address its session found when it
//! started: a desktop's private network when its container is created again, by a reset, a retry
//! or a restore, and before a stopped desktop starts where Docker connects it again with its MAC
//! address (API 1.54), which keeps its identity; a group's network when its first member joins it
//! alone. A private network made again for an existing container keeps the subnet the desktop was
//! told when it was created, also one that went missing, or is not made at all.
use super::networks::private_subnet;
use crate::{App, model::*};
use anyhow::{Context, Result, bail};
use serde_json::Value;

/// The bridge driver's option for the IPv4 gateway, and the mode that leaves the bridge without an
/// address on the host.
const GATEWAY_MODE: &str = "com.docker.network.bridge.gateway_mode_ipv4";
const ISOLATED: &str = "isolated";
/// The first Engine API that knows gateway mode `isolated`: Docker Engine 28.0.
pub(super) const ISOLATING_API: (u32, u32) = (1, 48);
/// The first Engine API that gives a container connected to a network after its creation the MAC
/// address asked for: Docker Engine 29 (see `identity`).
pub(super) const MAC_API: (u32, u32) = (1, 54);
/// Where the manager's process finds its mounts.
const MOUNTINFO: &str = "/proc/self/mountinfo";

/// What a container's network reaches besides the containers on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Reach {
    /// The internet, through the host.
    Internet,
    /// Nothing, but the host has an address on it.
    Host,
    /// Nothing at all: an internal network in gateway mode `isolated`.
    Nothing,
}

/// `docker network create` arguments for a bridge network named `name` with `labels` and, for one
/// made again, the `subnet` it had.
pub(crate) fn create_args(
    name: &str,
    labels: &[String],
    reach: Reach,
    subnet: Option<&str>,
) -> Vec<String> {
    let mut args = vec!["network".to_owned(), "create".to_owned()];
    match reach {
        Reach::Internet => {}
        Reach::Host => args.push("--internal".into()),
        Reach::Nothing => args.extend([
            "--internal".into(),
            "--opt".into(),
            format!("{GATEWAY_MODE}={ISOLATED}"),
        ]),
    }
    for label in labels {
        args.extend(["--label".into(), label.clone()]);
    }
    if let Some(subnet) = subnet {
        args.extend(["--subnet".into(), subnet.into()]);
    }
    args.push(name.into());
    args
}

/// Whether a network from `docker network inspect` has no address on the host.
pub(crate) fn isolated(network: &Value) -> bool {
    network["Options"][GATEWAY_MODE] == ISOLATED
}

/// Whether an Engine API version, such as `1.49`, is `least` or later.
pub(super) fn api_at_least(version: &str, least: (u32, u32)) -> bool {
    let Some((major, minor)) = version.trim().split_once('.') else {
        return false;
    };
    match (major.parse::<u32>(), minor.parse::<u32>()) {
        (Ok(major), Ok(minor)) => (major, minor) >= least,
        _ => false,
    }
}

/// The IPv4 subnet a container's desktop was told to listen in, `DESKTOP_PRIVATE_SUBNET` in its
/// `docker inspect`; none for a container created before the manager told it.
pub(crate) fn told_subnet(container: &Value) -> Option<String> {
    container["Config"]["Env"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find_map(|variable| variable.strip_prefix("DESKTOP_PRIVATE_SUBNET="))
        .filter(|subnet| {
            subnet.split_once('/').is_some_and(|(address, bits)| {
                address.parse::<std::net::Ipv4Addr>().is_ok()
                    && bits.parse::<u8>().is_ok_and(|bits| bits <= 32)
            })
        })
        .map(str::to_owned)
}

/// Whether the process whose `/proc/self/mountinfo` this is runs in the container with the full
/// ID `id`: Docker mounts a container's `/etc/hostname` from that container's own folder.
pub(crate) fn runs_in(mountinfo: &str, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    let wanted = format!("/{id}/hostname");
    mountinfo.lines().any(|line| {
        // The mount's ID, its parent's and its device, then its root and where it is mounted.
        let mut fields = line.split(' ');
        matches!(
            (fields.nth(3), fields.next()),
            (Some(root), Some("/etc/hostname")) if root.ends_with(&wanted)
        )
    })
}

impl App {
    /// The Engine API version of the daemon, empty when Docker does not say.
    async fn engine_api(&self) -> String {
        match self
            .docker(&["version", "--format", "{{.Server.APIVersion}}"])
            .await
        {
            Ok(out) => String::from_utf8_lossy(&out).trim().to_owned(),
            Err(error) => {
                tracing::warn!("Could not read the Docker Engine API version: {error:#}");
                String::new()
            }
        }
    }

    /// How far a new internal network reaches: nowhere, where the engine can make it so.
    pub(crate) async fn internal_reach(&self) -> Reach {
        if api_at_least(&self.engine_api().await, ISOLATING_API) {
            Reach::Nothing
        } else {
            Reach::Host
        }
    }

    /// The IDs of the containers, running or not, that a network is one of the networks of.
    /// Docker's own list of a network's containers leaves the stopped ones out, and removes a
    /// network they are on.
    pub(crate) async fn network_users(&self, network_id: &str) -> Result<Vec<String>> {
        let filter = format!("network={network_id}");
        let out = self
            .docker(&[
                "ps",
                "--all",
                "--no-trunc",
                "--filter",
                &filter,
                "--format",
                "{{.ID}}",
            ])
            .await?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// A desktop's private network, made when it is missing. It gets no address on the host where
    /// the engine can leave it without one and the manager, `manager` in `docker inspect`, runs in
    /// its container, and one made with an address is made again without while nothing that runs
    /// is on it. `container` is the desktop's own container, when there is one.
    pub(crate) async fn private_network(
        &self,
        env: &Container,
        manager: &Value,
        container: Option<&Value>,
    ) -> Result<Value> {
        let manager_id = manager["Id"].as_str().context("Missing manager ID")?;
        let inside =
            std::fs::read_to_string(MOUNTINFO).is_ok_and(|mounts| runs_in(&mounts, manager_id));
        let existing = self
            .optional_inspect("network", &env.private_network)
            .await?;
        if let Some(network) = existing.as_ref() {
            Self::check_network(env, network, true)?;
            if isolated(network) && !inside {
                tracing::warn!(
                    "The manager does not run in {}, and the private network of \"{}\" has no address on the Docker host, so the manager cannot reach the desktop's screen or ports; run the manager in its container",
                    self.config.manager.as_deref().unwrap_or_default(),
                    env.spec.name
                );
            }
            if isolated(network) || !inside {
                return Ok(network.clone());
            }
        }
        let api = self.engine_api().await;
        let reach = if inside && api_at_least(&api, ISOLATING_API) {
            Reach::Nothing
        } else {
            Reach::Host
        };
        let Some(network) = existing else {
            if !inside {
                tracing::warn!(
                    "The manager does not run in {}, so the private network of \"{}\" keeps an address on the Docker host, from where every program reaches the desktop's VNC server",
                    self.config.manager.as_deref().unwrap_or_default(),
                    env.spec.name
                );
            } else if reach == Reach::Host {
                tracing::warn!(
                    "Docker Engine before 28.0 gives the private network of \"{}\" an address on the Docker host, from where every program reaches the desktop's VNC server",
                    env.spec.name
                );
            }
            // Also one removed by hand, or by a move the manager did not finish.
            let subnet = container.and_then(told_subnet);
            return self.make_private(env, reach, subnet.as_deref()).await;
        };
        if reach == Reach::Nothing
            && self
                .movable(env, &network, manager_id, container, &api)
                .await?
        {
            return self.isolate_again(env, &network, manager, container).await;
        }
        Ok(network)
    }

    /// Whether a private network with an address on the host can be made again without one:
    /// nothing but the manager and the desktop's container is on it, that container is stopped,
    /// and Docker connects it again with its MAC address.
    async fn movable(
        &self,
        env: &Container,
        network: &Value,
        manager_id: &str,
        container: Option<&Value>,
        api: &str,
    ) -> Result<bool> {
        let mut container_id = None;
        if let Some(info) = container {
            // A running desktop's VNC server listens on the address its session found.
            if info["State"]["Running"] == true || info["State"]["Restarting"] == true {
                return Ok(false);
            }
            // An older engine, and the CLI, give a container connected again an address of their
            // own, which would make the desktop another machine.
            if env.spec.mac_address.is_some()
                && (self.config.docker_socket.is_none() || !api_at_least(api, MAC_API))
            {
                return Ok(false);
            }
            container_id = Some(info["Id"].as_str().context("Missing container ID")?);
        }
        let network_id = network["Id"].as_str().context("Missing network ID")?;
        Ok(self
            .network_users(network_id)
            .await?
            .iter()
            .all(|id| id == manager_id || Some(id.as_str()) == container_id))
    }

    /// Make a private network again without an address on the host, with the subnet the desktop
    /// was told, and connect the manager and the desktop's stopped container to it again.
    async fn isolate_again(
        &self,
        env: &Container,
        network: &Value,
        manager: &Value,
        container: Option<&Value>,
    ) -> Result<Value> {
        let network_id = network["Id"].as_str().context("Missing network ID")?;
        let manager_id = manager["Id"].as_str().context("Missing manager ID")?;
        let subnet = private_subnet(network).map(str::to_owned);
        let container_id = match container {
            Some(info) => Some(info["Id"].as_str().context("Missing container ID")?),
            None => None,
        };
        if let (Some(info), Some(id)) = (container, container_id)
            && info["NetworkSettings"]["Networks"]
                .get(&env.private_network)
                .is_some()
        {
            self.docker(&["network", "disconnect", network_id, id])
                .await?;
        }
        if network["Containers"].get(manager_id).is_some() {
            self.docker(&["network", "disconnect", network_id, manager_id])
                .await?;
        }
        self.docker(&["network", "rm", network_id]).await?;
        let made = self
            .make_private(env, Reach::Nothing, subnet.as_deref())
            .await?;
        let made_id = made["Id"].as_str().context("Missing network ID")?;
        self.docker(&["network", "connect", made_id, manager_id])
            .await?;
        if let Some(id) = container_id {
            self.connect_network(env, made_id, id).await?;
        }
        if isolated(&made) {
            tracing::info!(
                "Made the private network of \"{}\" again without an address on the Docker host",
                env.spec.name
            );
        }
        // With the manager on it now.
        self.inspect("network", made_id).await
    }

    /// Make a desktop's private network in `subnet`, the one its VNC server was told when its
    /// container was created, or fail: in another subnet the server would find no address of its
    /// own and listen on every interface, the internet network's included. Without an address on
    /// the host where `reach` asks for that and it can be, with one otherwise.
    async fn make_private(
        &self,
        env: &Container,
        reach: Reach,
        subnet: Option<&str>,
    ) -> Result<Value> {
        let name = &env.private_network;
        let Some(subnet) = subnet else {
            return self.create_network(env, name, reach, None).await;
        };
        let made = match self.create_network(env, name, reach, Some(subnet)).await {
            Err(error) if reach == Reach::Nothing => {
                tracing::warn!(
                    "Could not make the private network of \"{}\" without an address on the Docker host: {error:#}",
                    env.spec.name
                );
                self.create_network(env, name, Reach::Host, Some(subnet))
                    .await
            }
            made => made,
        };
        made.with_context(|| {
            format!(
                "Could not make the private network of \"{}\" again in {subnet}, the subnet its VNC server listens in; free it, or reset the container, which gives it a new one",
                env.spec.name
            )
        })
    }

    /// Before a stopped desktop starts: its private network made again without an address on the
    /// host where it can be (see [`App::private_network`]), and the desktop on it.
    pub(crate) async fn before_start(&self, env: &Container, info: &Value) -> Result<()> {
        let Some(manager) = &self.config.manager else {
            return Ok(());
        };
        if info["State"]["Running"] == true {
            return Ok(());
        }
        let manager = self.inspect("container", manager).await?;
        if manager["Config"]["Labels"]["thoric.manager"] != "true" {
            bail!("Manager label does not match");
        }
        let network = self.private_network(env, &manager, Some(info)).await?;
        let network_id = network["Id"].as_str().context("Missing network ID")?;
        let manager_id = manager["Id"].as_str().context("Missing manager ID")?;
        if network["Containers"].get(manager_id).is_none() {
            self.docker(&["network", "connect", network_id, manager_id])
                .await?;
        }
        // Also a desktop an earlier attempt left without it.
        let info = self.owned(env).await?;
        if info["NetworkSettings"]["Networks"]
            .get(&env.private_network)
            .is_none()
        {
            let container_id = info["Id"].as_str().context("Missing container ID")?;
            self.connect_network(env, network_id, container_id).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "isolation_tests.rs"]
mod tests;
