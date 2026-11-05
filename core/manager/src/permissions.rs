//! What a desktop may do beyond Docker's defaults when its settings ask for it. Each permission
//! is allowed by a switch of the installation (`DEBUGGER_ENABLED` and the others, see
//! [`crate::config`]) and given as `docker create` arguments, so a change recreates the container.
//! Each reaches the desktop's own programs and devices; what one also shows of the host is said
//! where it does.
//!
//! - `debugger`: `SYS_PTRACE`. The WSL kernel, like Ubuntu's, keeps Yama's `ptrace_scope` at 1,
//!   so without it not even root attaches to a program it did not start (`gdb -p`, `strace -p`,
//!   `py-spy`). With it root does, and the desktop user through `sudo`; starting a program under
//!   a debugger needs nothing. Docker's seccomp profile, which `images/desktop/seccomp.json`
//!   copies, allows `ptrace`, `process_vm_readv` and the rest to a container with the capability.
//!   Root can then read every program of the desktop, the proxy that holds its upstream's
//!   credentials (`crate::outbound`) included.
//! - `realtime`: limits rather than capabilities, so the desktop user has them without `sudo`:
//!   real-time priorities up to [`REALTIME_PRIORITY`], nice values down to -20 and memory locked
//!   without a limit, as audio servers and digital audio workstations ask for. The kernel does
//!   not hold a real-time program to `cpus`, which limits ordinary scheduling only.
//! - `profiler`: `PERFMON`, to which the seccomp profile ties `perf_event_open`. The desktop user
//!   measures its own programs; root measures every program on a CPU, the host's and other
//!   desktops' included.
//! - `kvm`: the host's `/dev/kvm`, for an Android emulator or QEMU, where the host has one: a Linux
//!   host with virtualization, or WSL with nested virtualization. The manager sees the host's
//!   devices in its `/sys` as [`crate::gpu`] does, and refuses the permission on a host without it.
use crate::{config::Config, model::ContainerSpec};
use std::path::Path;

/// The device a desktop with `kvm` gets.
pub(crate) const KVM_DEVICE: &str = "/dev/kvm";

/// The highest real-time priority the programs of a desktop with `realtime` may take, the one
/// audio setups give their group.
pub(crate) const REALTIME_PRIORITY: u32 = 95;

/// `RLIMIT_NICE` is 20 minus the lowest nice value a program may take, so 40 allows -20.
const NICE_LIMIT: u32 = 40;

/// What a Docker host without `/dev/kvm` needs, and what to do meanwhile.
const KVM_MISSING: &str = "This Docker host has no /dev/kvm; turn on virtualization for it (nested virtualization in WSL) and restart Thoric, or use the container without kvm";

/// The `docker create` arguments of the permissions `spec` asks for.
pub(crate) fn docker_args(spec: &ContainerSpec) -> Vec<String> {
    let mut args = Vec::new();
    if spec.debugger {
        args.extend(["--cap-add".into(), "SYS_PTRACE".into()]);
    }
    if spec.profiler {
        args.extend(["--cap-add".into(), "PERFMON".into()]);
    }
    if spec.realtime {
        for limit in [
            format!("rtprio={REALTIME_PRIORITY}"),
            format!("nice={NICE_LIMIT}"),
            // No limit, as the soft and the hard one.
            "memlock=-1".into(),
        ] {
            args.extend(["--ulimit".into(), limit]);
        }
    }
    if spec.kvm {
        args.extend(["--device".into(), KVM_DEVICE.into()]);
    }
    args
}

/// Whether the Docker host has `/dev/kvm`, read in `sys`, the host's `/sys`, or `None` when the
/// manager cannot see that host: a daemon other than a local one, or no `/sys` at all.
pub(crate) fn kvm_host(config: &Config, sys: &Path) -> Option<bool> {
    config.docker_socket.as_ref()?;
    sys.join("class")
        .is_dir()
        .then(|| sys.join("class/misc/kvm").exists())
}

/// What refuses `kvm` on this installation: its switch, or a host without `/dev/kvm`. A host the
/// manager cannot see may have one, and Docker's error says so when it does not.
pub(crate) fn kvm_refusal(config: &Config) -> Option<&'static str> {
    if !config.kvm {
        return Some("KVM is disabled for this installation (KVM_ENABLED=false)");
    }
    (config.kvm_host == Some(false)).then_some(KVM_MISSING)
}

/// Whether desktops can have `kvm` on this installation.
pub(crate) fn kvm_enabled(config: &Config) -> bool {
    kvm_refusal(config).is_none()
}

/// What refuses the permissions `spec` asks for on this installation, if anything.
pub(crate) fn refusal(config: &Config, spec: &ContainerSpec) -> Option<&'static str> {
    if spec.debugger && !config.debugger {
        return Some(
            "The debugger permission is disabled for this installation (DEBUGGER_ENABLED=false)",
        );
    }
    if spec.realtime && !config.realtime {
        return Some(
            "Real-time scheduling is disabled for this installation (REALTIME_ENABLED=false)",
        );
    }
    if spec.profiler && !config.profiler {
        return Some("Profiling is disabled for this installation (PROFILER_ENABLED=false)");
    }
    if spec.kvm {
        return kvm_refusal(config);
    }
    None
}

/// A Docker error from creating or starting a desktop with `kvm` on a host without the device,
/// with the setting behind it; any other error as it is.
pub(crate) fn explain(spec: &ContainerSpec, error: anyhow::Error) -> anyhow::Error {
    if spec.kvm && format!("{error:#}").contains(KVM_DEVICE) {
        return error.context(KVM_MISSING);
    }
    error
}

#[cfg(test)]
#[path = "permissions_tests.rs"]
mod tests;
