//! What the Docker host shows of its GPUs, read once when the manager starts: the WSL GPU device,
//! an NVIDIA GPU that NVIDIA's own driver runs, the NVIDIA Container Toolkit in Docker, and the
//! render devices Mesa's own drivers can use.
//!
//! A container shares its host's `/sys`, so the manager reads it without a helper container. It is
//! the Docker host's only while the manager talks to a local daemon; with another one nothing is
//! known, and the settings apply as they are written.
use crate::config::Config;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// PCI vendor IDs as `/sys` writes them.
const NVIDIA: &str = "0x10de";
const AMD: &str = "0x1002";
const INTEL: &str = "0x8086";

/// The Docker host's GPU devices, or nothing when the manager cannot see them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Host {
    pub seen: Option<Seen>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(crate) struct Seen {
    /// The WSL GPU device, `/dev/dxg`.
    pub wsl: bool,
    /// A display device that NVIDIA's own driver runs; the toolkit needs that driver.
    pub nvidia: bool,
    /// Whether Docker has the NVIDIA Container Toolkit, `None` when it was not looked for or
    /// Docker did not answer.
    pub toolkit: Option<bool>,
    /// The vendors of the render devices under `/dev/dri` that Mesa's own drivers run, such as
    /// `AMD`, `Intel` or the Raspberry Pi's `Broadcom`, each once.
    pub render: Vec<String>,
}

impl Host {
    /// Look at the host the manager's Docker daemon runs on, with `docker_info` as the daemon
    /// answered it when the manager started ([`crate::desktop_limits::docker_info`]).
    pub(crate) fn detect(config: &Config, docker_info: Option<&Value>) -> Host {
        // Only a local daemon's socket can be the host whose /sys the manager sees.
        if config.docker_socket.is_none() {
            return Host::default();
        }
        let Some(mut seen) = Seen::read(Path::new("/sys")) else {
            return Host::default();
        };
        if seen.nvidia && !seen.wsl {
            seen.toolkit = docker_info.map(toolkit);
        }
        Host { seen: Some(seen) }
    }
}

impl Seen {
    /// Read the devices under `sys`, the host's `/sys`.
    pub(crate) fn read(sys: &Path) -> Option<Seen> {
        if !sys.join("class").is_dir() {
            return None;
        }
        let nvidia = entries(&sys.join("bus/pci/devices")).iter().any(|device| {
            file(device, "vendor").as_deref() == Some(NVIDIA)
                && file(device, "class").is_some_and(|class| class.starts_with("0x03"))
                && driver(device).as_deref() == Some("nvidia")
        });
        let mut render: Vec<String> = entries(&sys.join("class/drm"))
            .iter()
            .filter(|node| {
                node.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("renderD"))
            })
            .map(|node| node.join("device"))
            .filter_map(|device| {
                let driver = driver(&device);
                // NVIDIA's own driver renders only through the toolkit.
                if driver.as_deref() == Some("nvidia") {
                    return None;
                }
                match file(&device, "vendor") {
                    Some(vendor) => Some(vendor_name(&vendor)),
                    // A board's GPU is no PCI device, so its driver names it. A virtual device
                    // such as WSL's vgem has neither a vendor nor one of those drivers.
                    None => driver.as_deref().and_then(board_gpu).map(str::to_owned),
                }
            })
            .collect();
        render.sort();
        render.dedup();
        Some(Seen {
            wsl: sys.join("class/misc/dxg").exists(),
            nvidia,
            toolkit: None,
            render,
        })
    }
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    paths.sort();
    paths
}

fn file(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name))
        .ok()
        .map(|text| text.trim().to_ascii_lowercase())
}

/// The driver a device is bound to, as its `uevent` names it.
fn driver(device: &Path) -> Option<String> {
    std::fs::read_to_string(device.join("uevent"))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("DRIVER="))
        .map(str::to_owned)
}

fn vendor_name(vendor: &str) -> String {
    match vendor {
        AMD => "AMD".into(),
        INTEL => "Intel".into(),
        NVIDIA => "NVIDIA".into(),
        other => other.into(),
    }
}

/// The vendor of a GPU that is part of its board rather than a PCI device, by the kernel driver
/// that runs it, for those Mesa has a driver of its own for: the Raspberry Pi's V3D (and the VC4
/// of the boards before the Pi 4), Arm's Mali, Qualcomm's Adreno, Vivante's and Apple's.
fn board_gpu(driver: &str) -> Option<&'static str> {
    Some(match driver {
        "v3d" | "vc4-drm" => "Broadcom",
        "panfrost" | "panthor" | "lima" => "Arm",
        "msm" => "Qualcomm",
        "etnaviv" => "Vivante",
        "asahi" => "Apple",
        _ => return None,
    })
}

/// Whether `docker info` names the NVIDIA Container Toolkit: the `nvidia` runtime its Docker
/// setup registers, or an `nvidia.com/gpu` device among Docker's CDI devices.
pub(crate) fn toolkit(info: &Value) -> bool {
    info["Runtimes"].get("nvidia").is_some()
        || info["DiscoveredDevices"].as_array().is_some_and(|devices| {
            devices.iter().any(|device| {
                device["ID"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("nvidia.com/gpu"))
            })
        })
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
