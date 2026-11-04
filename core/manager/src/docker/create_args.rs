//! The `docker create` arguments of a desktop container, every setting Docker fixes when it creates
//! one, which a settings change can alter only by creating the container again.
use crate::{App, model::*};

/// The device a VPN client opens its tunnel through, which a desktop with `vpn` gets.
pub(super) const TUN_DEVICE: &str = "/dev/net/tun";

impl App {
    /// The `docker create` arguments for a container: every setting that Docker
    /// fixes when it creates a container is here. `private_subnet` is its private network's, the
    /// one the desktop's VNC server listens in.
    pub(crate) fn container_args(
        &self,
        env: &Container,
        private_subnet: Option<&str>,
    ) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "create",
            "--name",
            &env.container,
            "--network",
            &env.private_network,
            "--restart",
            "unless-stopped",
            "--label",
            &format!("{LABEL}=true"),
            "--label",
            &format!("thoric.environment={}", env.id),
            "--shm-size",
            &format!("{}m", env.spec.shm_mb),
            "--tmpfs",
            "/run:rw,nosuid,size=128m",
            "--tmpfs",
            &format!("/tmp:rw,nosuid,size={}m", env.spec.tmp_mb),
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        if let Some(hostname) = &env.spec.hostname {
            args.extend(["--hostname".into(), hostname.clone()]);
        }
        if let Some(mac) = &env.spec.mac_address {
            // For the private network; the internet network gets it when it is connected.
            args.extend(["--mac-address".into(), mac.clone()]);
        }
        let envs = [
            ("DESKTOP_RESOLUTION", env.spec.resolution.clone()),
            ("DESKTOP_SUDO", env.spec.sudo.to_string()),
            ("DESKTOP_AUDIO", env.spec.audio.to_string()),
            ("DESKTOP_MICROPHONE", env.spec.microphone.to_string()),
            ("DESKTOP_GPU", env.spec.gpu.to_string()),
            ("GPU_ADAPTER", self.config.adapter.clone()),
        ];
        for (key, val) in envs
            .into_iter()
            .chain(crate::outbound::environment(&env.spec))
        {
            args.extend(["--env".into(), format!("{key}={val}")]);
        }
        if let Some(subnet) = private_subnet {
            args.extend(["--env".into(), format!("DESKTOP_PRIVATE_SUBNET={subnet}")]);
        } else {
            // Its VNC server listens everywhere, whatever its image says.
            args.extend([
                "--label".into(),
                format!("{}=all", crate::groups::VNC_LABEL),
            ]);
        }
        if let Some(profile) = self.config.seccomp_profile() {
            // The CLI reads the file and sends its contents with the request.
            args.extend([
                "--security-opt".into(),
                format!("seccomp={}", profile.display()),
            ]);
        }
        if let Some(c) = env.spec.cpus {
            args.extend(["--cpus".into(), c.to_string()]);
        }
        if let Some(m) = env.spec.memory_mb {
            args.extend(["--memory".into(), format!("{m}m")]);
        }
        // What keeps it from exhausting the host: its pids limit and a log that rotates.
        args.extend(crate::desktop_limits::docker_args(&self.config));
        for m in env.mounts() {
            args.extend([
                "--mount".into(),
                format!(
                    "type=volume,src={},dst={}{}",
                    m.volume,
                    m.target,
                    if m.readonly { ",readonly" } else { "" }
                ),
            ]);
        }
        if env.spec.gpu {
            args.extend(crate::gpu::docker_args(
                &crate::gpu::routes(&self.config).used,
            ));
        }
        if env.spec.gamepad {
            // The virtual controller is created by a root helper inside the desktop; input
            // device nodes (major 13) are made there, so the cgroup rule must allow them.
            args.extend([
                "--device".into(),
                "/dev/uinput".into(),
                "--device-cgroup-rule".into(),
                "c 13:* rmw".into(),
            ]);
        }
        if env.spec.vpn {
            // A VPN client opens its tunnel and routes in the desktop's own network namespace;
            // wg-quick marks its packets, which needs `src_valid_mark` for the default route.
            args.extend([
                "--cap-add".into(),
                "NET_ADMIN".into(),
                "--device".into(),
                TUN_DEVICE.into(),
                "--sysctl".into(),
                "net.ipv4.conf.all.src_valid_mark=1".into(),
            ]);
        }
        // Use an immutable ID, not a tag that could change between inspect and create.
        args.push(env.container_image().to_owned());
        args
    }
}
#[cfg(test)]
#[path = "create_args_tests.rs"]
mod tests;
