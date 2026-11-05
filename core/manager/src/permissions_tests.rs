//! Pins down what each permission gives a desktop in `docker create`, how the manager reads
//! whether the Docker host has `/dev/kvm`, and that a refusal names the setting behind it: an
//! installation switch, or a host without the device.
use super::*;
use crate::test_support::spec;
use std::path::PathBuf;

/// The arguments `docker_args` gives a spec with `edit` applied, as one line.
fn args(edit: impl FnOnce(&mut ContainerSpec)) -> String {
    let mut spec = spec();
    edit(&mut spec);
    docker_args(&spec).join(" ")
}

#[test]
fn each_permission_gives_its_own_arguments_and_none_gives_nothing() {
    assert_eq!(args(|_| {}), "");
    assert_eq!(args(|spec| spec.debugger = true), "--cap-add SYS_PTRACE");
    assert_eq!(args(|spec| spec.profiler = true), "--cap-add PERFMON");
    // Limits rather than capabilities, so the desktop user has them without sudo.
    assert_eq!(
        args(|spec| spec.realtime = true),
        "--ulimit rtprio=95 --ulimit nice=40 --ulimit memlock=-1"
    );
    assert_eq!(args(|spec| spec.kvm = true), "--device /dev/kvm");
}

/// A `/sys` of its own, removed when the test ends.
struct Sys(PathBuf);

impl Sys {
    fn new(kvm: bool) -> Sys {
        let root = std::env::temp_dir().join(format!("thoric-sys-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("class/misc/tun")).unwrap();
        if kvm {
            std::fs::create_dir_all(root.join("class/misc/kvm")).unwrap();
        }
        Sys(root)
    }
}

impl Drop for Sys {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_host_has_kvm_when_its_sys_lists_the_device_and_is_unknown_when_unseen() {
    let mut config = Config::for_tests();
    // Only a local daemon's host is the one whose /sys the manager sees.
    config.docker_socket = None;
    assert_eq!(kvm_host(&config, &Sys::new(true).0), None);
    config.docker_socket = Some("/var/run/docker.sock".into());
    assert_eq!(kvm_host(&config, &Sys::new(true).0), Some(true));
    // WSL without nested virtualization lists other devices but not this one.
    assert_eq!(kvm_host(&config, &Sys::new(false).0), Some(false));
    let missing = std::env::temp_dir().join(format!("thoric-nosys-{}", uuid::Uuid::new_v4()));
    assert_eq!(kvm_host(&config, &missing), None);
}

#[test]
fn a_refused_permission_names_the_setting_or_the_missing_device() {
    let mut config = Config::for_tests();
    let asking = |edit: fn(&mut ContainerSpec)| {
        let mut spec = spec();
        edit(&mut spec);
        spec
    };
    let all = asking(|spec| {
        spec.debugger = true;
        spec.realtime = true;
        spec.profiler = true;
        spec.kvm = true;
    });
    // Every switch is on in the tests, and the manager does not see the host.
    assert_eq!(refusal(&config, &all), None);
    assert!(kvm_enabled(&config));
    for (edit, switch) in [
        (
            (|config: &mut Config| config.debugger = false) as fn(&mut Config),
            "DEBUGGER_ENABLED",
        ),
        (|config| config.realtime = false, "REALTIME_ENABLED"),
        (|config| config.profiler = false, "PROFILER_ENABLED"),
        (|config| config.kvm = false, "KVM_ENABLED"),
    ] {
        let mut off = Config::for_tests();
        edit(&mut off);
        let refused = refusal(&off, &all).unwrap();
        assert!(refused.contains(switch), "{refused}");
    }
    // A switch that is off refuses nothing a spec does not ask for.
    config.debugger = false;
    assert_eq!(refusal(&config, &asking(|spec| spec.kvm = true)), None);
    // A host seen without the device refuses kvm alone, and says what the host needs.
    config.kvm_host = Some(false);
    assert!(!kvm_enabled(&config));
    let refused = refusal(&config, &asking(|spec| spec.kvm = true)).unwrap();
    assert!(refused.contains("no /dev/kvm"), "{refused}");
    assert!(refused.contains("without kvm"), "{refused}");
    config.kvm_host = Some(true);
    assert!(kvm_enabled(&config));
}

#[test]
fn dockers_error_about_the_device_names_the_kvm_setting() {
    let docker = || {
        anyhow::anyhow!(
            "error gathering device information while adding custom device \"/dev/kvm\": no such file or directory"
        )
    };
    let mut spec = spec();
    // Without kvm the device is not the container's, and Docker's words stay as they are.
    assert_eq!(
        format!("{:#}", explain(&spec, docker())),
        format!("{:#}", docker())
    );
    spec.kvm = true;
    let explained = format!("{:#}", explain(&spec, docker()));
    assert!(explained.starts_with(KVM_MISSING), "{explained}");
    assert!(explained.contains("custom device"), "{explained}");
}
