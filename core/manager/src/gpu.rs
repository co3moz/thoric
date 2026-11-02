//! How a GPU desktop reaches the host's GPU. `WSL_GPU`, `NVIDIA_COMPUTE` and `DRI_GPU` each allow
//! a route, and a route is used where the host has it ([`host`]): a WSL distribution gets the WSL
//! GPU, a Linux host with NVIDIA's driver and the NVIDIA Container Toolkit gets the NVIDIA GPU, and
//! one with an AMD or Intel GPU, or a board's own such as a Raspberry Pi's, gets its render
//! devices. Docker Desktop on a Mac shows none of these. Where the manager cannot see the host,
//! the WSL and NVIDIA settings apply as they are written, as they did before routes were chosen,
//! and `/dev/dri` is never passed, since inside WSL it would push the WSL GPU aside.
use crate::config::Config;
use serde::Serialize;
use serde_json::{Value, json};

mod host;
pub(crate) use host::Host;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Route {
    /// `/dev/dxg` and the WSL driver's libraries; Mesa's D3D12 driver renders on it.
    Wsl,
    /// The NVIDIA Container Toolkit: CUDA, and OpenGL through NVIDIA's EGL library.
    Nvidia,
    /// `/dev/dri`, for Mesa's own drivers such as AMD's, Intel's and the Raspberry Pi's V3D.
    Dri,
}

/// In the order their `docker create` arguments go.
const ROUTES: [Route; 3] = [Route::Nvidia, Route::Wsl, Route::Dri];

/// The WSL GPU device and the driver libraries Windows puts beside it.
pub(crate) const WSL_DEVICE: [&str; 4] = [
    "--device",
    "/dev/dxg",
    "--mount",
    "type=bind,src=/usr/lib/wsl,dst=/usr/lib/wsl,readonly",
];

impl Route {
    pub(crate) fn setting(self) -> &'static str {
        match self {
            Route::Wsl => "WSL_GPU",
            Route::Nvidia => "NVIDIA_COMPUTE",
            Route::Dri => "DRI_GPU",
        }
    }

    fn allowed(self, config: &Config) -> bool {
        match self {
            Route::Wsl => config.wsl_gpu,
            Route::Nvidia => config.cuda,
            Route::Dri => config.dri_gpu,
        }
    }

    /// What to do about a Docker error that names this route's devices.
    fn hint(self) -> &'static str {
        match self {
            Route::Wsl => {
                "WSL_GPU gives desktops the WSL GPU, which Docker could not pass; on a host outside WSL set WSL_GPU=false"
            }
            Route::Nvidia => {
                "NVIDIA_COMPUTE gives desktops the GPU through the NVIDIA Container Toolkit, which Docker could not use; configure it with nvidia-ctk runtime configure --runtime=docker, or set NVIDIA_COMPUTE=false"
            }
            Route::Dri => {
                "DRI_GPU gives desktops the render devices under /dev/dri, which Docker could not pass; set DRI_GPU=false"
            }
        }
    }
}

/// Why a route its setting allows is not used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Missing {
    NoWslGpu,
    NoNvidiaGpu,
    NoToolkit,
    NoRenderDevice,
    /// Mesa would prefer these devices to the WSL GPU.
    BesideWsl,
    /// `/dev/dri` is passed only where the manager sees the host's devices.
    Unseen,
}

impl Missing {
    fn text(self) -> &'static str {
        match self {
            Missing::NoWslGpu => "this host has no WSL GPU (/dev/dxg)",
            Missing::NoNvidiaGpu => "this host has no NVIDIA GPU with NVIDIA's driver",
            Missing::NoToolkit => {
                "Docker has no NVIDIA Container Toolkit (nvidia-ctk runtime configure --runtime=docker)"
            }
            Missing::NoRenderDevice => "this host has no render device under /dev/dri",
            Missing::BesideWsl => "the WSL GPU is used instead",
            Missing::Unseen => "the manager cannot see the Docker host's devices",
        }
    }
}

/// The routes a GPU desktop gets, and why each other one its setting allows is not used.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Routes {
    pub used: Vec<Route>,
    pub unused: Vec<(Route, Missing)>,
}

pub(crate) fn routes(config: &Config) -> Routes {
    let mut routes = Routes::default();
    for route in ROUTES.into_iter().filter(|route| route.allowed(config)) {
        match check(route, config) {
            Ok(()) => routes.used.push(route),
            Err(missing) => routes.unused.push((route, missing)),
        }
    }
    routes
}

fn check(route: Route, config: &Config) -> Result<(), Missing> {
    let Some(seen) = &config.gpu_host.seen else {
        return match route {
            Route::Dri => Err(Missing::Unseen),
            _ => Ok(()),
        };
    };
    match route {
        Route::Wsl if !seen.wsl => Err(Missing::NoWslGpu),
        // In WSL the GPU is paravirtual and has no PCI vendor; the WSL package's first start
        // sets NVIDIA_COMPUTE from the driver libraries Windows provides.
        Route::Wsl | Route::Nvidia if seen.wsl => Ok(()),
        Route::Nvidia if !seen.nvidia => Err(Missing::NoNvidiaGpu),
        Route::Nvidia if seen.toolkit == Some(false) => Err(Missing::NoToolkit),
        Route::Dri if seen.render.is_empty() => Err(Missing::NoRenderDevice),
        Route::Dri if seen.wsl && config.wsl_gpu => Err(Missing::BesideWsl),
        _ => Ok(()),
    }
}

/// Say at startup which routes GPU desktops get, and warn about a GPU the settings allow that
/// they cannot have.
pub(crate) fn log_routes(config: &Config) {
    let routes = routes(config);
    if !routes.used.is_empty() {
        let settings: Vec<&str> = routes.used.iter().map(|route| route.setting()).collect();
        tracing::info!("GPU desktops get the GPU through {}", settings.join(", "));
    }
    for (route, missing) in &routes.unused {
        let line = format!("{} is true, but {}", route.setting(), missing.text());
        if routes.used.is_empty() || *missing == Missing::NoToolkit {
            tracing::warn!("{line}");
        } else {
            tracing::info!("{line}");
        }
    }
}

/// The `docker create` arguments that give a desktop the GPU through `routes`.
pub(crate) fn docker_args(routes: &[Route]) -> Vec<String> {
    let mut args = Vec::new();
    for route in routes {
        match route {
            // In WSL, Mesa renders OpenGL on the WSL GPU, and the toolkit adds CUDA.
            Route::Nvidia if routes.contains(&Route::Wsl) => args.extend(["--gpus", "all"]),
            // Elsewhere OpenGL goes through NVIDIA's EGL library, which the toolkit's hook adds
            // only for the graphics capability; a CDI device adds every library anyway. The
            // capabilities also hold Docker to NVIDIA's device driver.
            Route::Nvidia => {
                args.extend(["--gpus", r#"all,"capabilities=compute,utility,graphics""#])
            }
            Route::Wsl => args.extend(WSL_DEVICE),
            Route::Dri => args.extend(["--device", "/dev/dri"]),
        }
    }
    args.into_iter().map(str::to_owned).collect()
}

/// Why a desktop cannot have the GPU on this installation, or `None` when it can.
pub(crate) fn refusal(config: &Config) -> Option<String> {
    let routes = routes(config);
    if !routes.used.is_empty() {
        return None;
    }
    if routes.unused.is_empty() {
        return Some(
            "GPU access is disabled for this installation (WSL_GPU, NVIDIA_COMPUTE and DRI_GPU are false); use the container without GPU".into(),
        );
    }
    let reasons: Vec<String> = routes
        .unused
        .iter()
        .map(|(route, missing)| format!("{} is true, but {}", route.setting(), missing.text()))
        .collect();
    Some(format!(
        "No GPU on this host can be given to desktops: {}; use the container without GPU",
        reasons.join("; ")
    ))
}

/// Name the setting behind a Docker error about the GPU devices a desktop was given.
pub(crate) fn explain(error: anyhow::Error, routes: &[Route]) -> anyhow::Error {
    let text = format!("{error:#}");
    let lower = text.to_ascii_lowercase();
    let named = if text.contains("/usr/lib/wsl") || text.contains("/dev/dxg") {
        Route::Wsl
    } else if lower.contains("could not select device driver") || lower.contains("nvidia") {
        Route::Nvidia
    } else if text.contains("/dev/dri") {
        Route::Dri
    } else {
        return error;
    };
    if routes.contains(&named) {
        error.context(named.hint())
    } else {
        error
    }
}

/// What the settings route says about the GPU: the routes desktops get, why the others their
/// settings allow are not used, and what the host showed, `null` where the manager cannot see it.
pub(crate) fn report(config: &Config) -> Value {
    let routes = routes(config);
    let unused: Vec<Value> = routes
        .unused
        .iter()
        .map(|(route, missing)| json!({"route": route, "setting": route.setting(), "reason": missing}))
        .collect();
    json!({"routes": routes.used, "unused": unused, "found": config.gpu_host.seen})
}

#[cfg(test)]
#[path = "gpu_tests.rs"]
mod tests;
