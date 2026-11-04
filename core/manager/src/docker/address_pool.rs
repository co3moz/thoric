//! Where the networks Thoric makes get their addresses. Docker gives a network made without a
//! subnet a whole one of its default address pools, a /16 or a /20, and a stock daemon has 31 of
//! them: the two networks of every desktop used them up at about 14 desktops, sooner with group
//! networks or other projects on the same daemon, and Docker then refused the next one with "all
//! predefined address pools have been fully subnetted".
//!
//! So the manager gives each network it makes a small subnet of `DESKTOP_ADDRESS_POOL`,
//! [`DEFAULT_POOL`] unless set: a /28 for a desktop's private and internet networks, which hold
//! the desktop, the manager and a gateway, and a /24 for a group's, the first one that no Docker
//! network overlaps. One network is made at a time, and a subnet Docker refuses all the same, such
//! as one a network made meanwhile by someone else took, is passed over. `docker` leaves the choice
//! to Docker's own pools, as before the setting.
//!
//! A network made before keeps its subnet, and so does a private network made again in the subnet
//! its desktop was told (see `isolation`). The private and group networks have no address on the
//! Docker host where the engine can leave them without one, so it is the internet networks that
//! put the range on the host's routes, and the range should be one the LAN and VPNs do not use.
use super::isolation::{Reach, create_args};
use crate::App;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::net::Ipv4Addr;

/// The prefix of a desktop's private and internet networks: 16 addresses, of which the desktop,
/// the manager and the gateway take three.
pub(crate) const DESKTOP_PREFIX: u8 = 28;
/// The prefix of a group's network, for up to 253 members besides its gateway, and so the
/// smallest range `DESKTOP_ADDRESS_POOL` may be.
pub(crate) const GROUP_PREFIX: u8 = 24;
/// The largest range `DESKTOP_ADDRESS_POOL` may be.
const WIDEST: u8 = 8;
/// `DESKTOP_ADDRESS_POOL` unless it is set: room for 4096 networks of a desktop, two for each,
/// outside Docker's own pools (172.17.0.0/16 to 172.31.0.0/16 and 192.168.0.0/16) and the
/// 172.16.0.0/12 WSL takes its network from.
pub(crate) const DEFAULT_POOL: Subnet = Subnet {
    base: u32::from_be_bytes([10, 213, 0, 0]),
    prefix: 16,
};
/// The value of `DESKTOP_ADDRESS_POOL` that leaves the subnets to Docker's own pools.
const DOCKER: &str = "docker";
/// The lock that keeps two networks from being given the same subnet.
const LOCK: &str = "address-pool";
/// How many subnets Docker may refuse before a network is not made.
const ATTEMPTS: usize = 8;

/// An IPv4 subnet, such as `10.213.0.0/28`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Subnet {
    base: u32,
    prefix: u8,
}

impl Subnet {
    /// A subnet as Docker writes one, its address cut to the prefix: one with host bits set still
    /// covers what its prefix says.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let (address, prefix) = text.trim().split_once('/')?;
        let address = address.parse::<Ipv4Addr>().ok()?;
        let prefix = prefix.parse::<u8>().ok().filter(|prefix| *prefix <= 32)?;
        Some(Self {
            base: u32::from(address) & mask(prefix),
            prefix,
        })
    }
    fn first(self) -> u64 {
        u64::from(self.base)
    }
    fn last(self) -> u64 {
        self.first() + size(self.prefix) - 1
    }
    fn overlaps(self, other: Self) -> bool {
        self.first() <= other.last() && other.first() <= self.last()
    }
}

impl std::fmt::Display for Subnet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", Ipv4Addr::from(self.base), self.prefix)
    }
}

/// The bits of an address a prefix keeps.
fn mask(prefix: u8) -> u32 {
    u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0)
}

/// How many addresses a subnet with this prefix holds.
fn size(prefix: u8) -> u64 {
    1 << (32 - u32::from(prefix))
}

/// `DESKTOP_ADDRESS_POOL`: the range the networks Thoric makes take their subnets from, or `None`
/// for Docker's own pools. Empty is [`DEFAULT_POOL`].
pub(crate) fn address_pool(value: Option<&str>) -> Result<Option<Subnet>> {
    let text = value.unwrap_or_default();
    match text {
        "" => return Ok(Some(DEFAULT_POOL)),
        DOCKER => return Ok(None),
        _ => {}
    }
    let refused = || {
        format!(
            "DESKTOP_ADDRESS_POOL must be an IPv4 range from /{WIDEST} to /{GROUP_PREFIX} written with its first address, such as {DEFAULT_POOL}, or {DOCKER} for Docker's own address pools"
        )
    };
    let pool = Subnet::parse(text).with_context(refused)?;
    if pool.to_string() != text || !(WIDEST..=GROUP_PREFIX).contains(&pool.prefix) {
        bail!(refused());
    }
    let first = pool.base >> 24;
    if first == 0 || first == 127 || first >= 224 {
        bail!(
            "DESKTOP_ADDRESS_POOL must be a range of unicast addresses, outside 0.0.0.0/8, 127.0.0.0/8 and 224.0.0.0/3"
        );
    }
    Ok(Some(pool))
}

/// The first subnet with `prefix` in `pool` that overlaps none of `taken`.
pub(crate) fn free_subnet(pool: Subnet, prefix: u8, taken: &[Subnet]) -> Option<Subnet> {
    let size = size(prefix);
    let mut start = pool.first();
    while start + size - 1 <= pool.last() {
        let candidate = Subnet {
            base: u32::try_from(start).ok()?,
            prefix,
        };
        let Some(end) = taken
            .iter()
            .filter(|subnet| subnet.overlaps(candidate))
            .map(|subnet| subnet.last())
            .max()
        else {
            return Some(candidate);
        };
        // The next subnet of this size after the last one in the way.
        start = (end + 1).div_ceil(size) * size;
    }
    None
}

/// The IPv4 subnets in `docker network inspect --format` output of [`App::network_subnets`].
pub(crate) fn listed_subnets(output: &str) -> Vec<Subnet> {
    output
        .split_whitespace()
        .filter_map(Subnet::parse)
        .collect()
}

/// What a full `DESKTOP_ADDRESS_POOL` answers a network that needs a subnet of it.
pub(crate) fn pool_full(pool: Subnet, prefix: u8, name: &str) -> String {
    format!(
        "DESKTOP_ADDRESS_POOL ({pool}) has no /{prefix} left that no Docker network uses, for the network {name}; set DESKTOP_ADDRESS_POOL to a larger range, or delete containers you no longer need"
    )
}

/// A `docker network create` error, with the setting to change when Docker's own address pools
/// are full.
pub(crate) fn docker_pools(error: anyhow::Error) -> anyhow::Error {
    if format!("{error:#}").contains("fully subnetted") {
        return error.context(format!(
            "Docker's default address pools are full; set DESKTOP_ADDRESS_POOL to a range of Thoric's own, such as {DEFAULT_POOL}, where each network takes a small subnet"
        ));
    }
    error
}

/// Whether Docker refused a subnet because a network it knows already uses part of it.
fn overlapping(error: &anyhow::Error) -> bool {
    format!("{error:#}").contains("overlaps")
}

impl App {
    /// Make a network with [`create_args`]: in `subnet` when it has to keep one, otherwise in the
    /// first free subnet with `prefix` of `DESKTOP_ADDRESS_POOL`, or where Docker's own pools put
    /// it.
    pub(crate) async fn make_network(
        &self,
        name: &str,
        labels: &[String],
        reach: Reach,
        subnet: Option<&str>,
        prefix: u8,
    ) -> Result<Value> {
        if let Some(subnet) = subnet {
            return self
                .network_create(&create_args(name, labels, reach, Some(subnet)))
                .await;
        }
        let Some(pool) = self.config.address_pool else {
            return self
                .network_create(&create_args(name, labels, reach, None))
                .await
                .map_err(docker_pools);
        };
        let lock = self.resource_lock(LOCK);
        let _guard = lock.lock().await;
        let mut refused = Vec::new();
        loop {
            let mut taken = self.network_subnets().await?;
            taken.extend(&refused);
            let Some(subnet) = free_subnet(pool, prefix, &taken) else {
                bail!(pool_full(pool, prefix, name));
            };
            let args = create_args(name, labels, reach, Some(&subnet.to_string()));
            match self.network_create(&args).await {
                Err(error) if overlapping(&error) => {
                    refused.push(subnet);
                    if refused.len() == ATTEMPTS {
                        return Err(error).with_context(|| {
                            format!(
                                "Docker refused {ATTEMPTS} subnets of DESKTOP_ADDRESS_POOL ({pool}) for the network {name} as overlapping networks it has; set DESKTOP_ADDRESS_POOL to a range they do not use"
                            )
                        });
                    }
                }
                made => return made,
            }
        }
    }

    async fn network_create(&self, args: &[String]) -> Result<Value> {
        let id = self.docker_args(args).await?;
        self.inspect("network", String::from_utf8_lossy(&id).trim())
            .await
    }

    /// The IPv4 subnets of every Docker network.
    async fn network_subnets(&self) -> Result<Vec<Subnet>> {
        let mut tries = 3;
        loop {
            let listed = self
                .docker(&["network", "ls", "--quiet", "--no-trunc"])
                .await?;
            let mut args = vec![
                "network".to_owned(),
                "inspect".to_owned(),
                "--format".to_owned(),
                "{{range .IPAM.Config}}{{.Subnet}} {{end}}".to_owned(),
            ];
            args.extend(
                String::from_utf8_lossy(&listed)
                    .split_whitespace()
                    .map(str::to_owned),
            );
            if args.len() == 4 {
                return Ok(Vec::new());
            }
            match self.docker_args(&args).await {
                Ok(output) => return Ok(listed_subnets(&String::from_utf8_lossy(&output))),
                // Also a network removed after the list.
                Err(_) if tries > 1 => tries -= 1,
                Err(error) => return Err(error).context("Could not read the subnets Docker uses"),
            }
        }
    }
}

#[cfg(test)]
#[path = "address_pool_tests.rs"]
mod tests;
