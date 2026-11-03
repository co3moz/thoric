//! What counts as noise: a fresh desktop of either catalog image, as Docker listed it once its
//! session had started, also one with the GPU in WSL, lists nothing, while what an agent does in it
//! afterwards stays, code the desktop runs and group membership among it, and the folders listed
//! only for what is below them go with it.
use super::*;

/// `docker diff` of a fresh XFCE desktop, `ghcr.io/co3moz/thoric/ubuntu-xfce:24.04`, half a
/// minute after `/run/desktop/ready`, with the machine ID Thoric writes before it starts.
const FRESH_XFCE: &str = "C /etc
C /etc/profile.d
A /etc/profile.d/desktop.sh
C /etc/sudoers.d
A /etc/sudoers.d/desktop
A /etc/sudoers.d/thoric-proxy
C /etc/machine-id
C /home
C /home/desktop
A /home/desktop/Desktop
A /home/desktop/Videos
A /home/desktop/.ICEauthority
A /home/desktop/.config
A /home/desktop/.config/Thunar
A /home/desktop/.config/Thunar/uca.xml
A /home/desktop/.config/pulse
A /home/desktop/.config/pulse/439fa3aca958-device-volumes.tdb
A /home/desktop/.config/pulse/439fa3aca958-stream-volumes.tdb
A /home/desktop/.config/pulse/cookie
A /home/desktop/.config/pulse/439fa3aca958-card-database.tdb
A /home/desktop/.config/pulse/439fa3aca958-default-sink
A /home/desktop/.config/pulse/439fa3aca958-default-source
A /home/desktop/.config/xfce4
A /home/desktop/.config/xfce4/desktop
A /home/desktop/.config/xfce4/desktop/icons.screen0-1264x677.rc
A /home/desktop/.config/xfce4/desktop/icons.screen.latest.rc
A /home/desktop/.config/xfce4/panel
A /home/desktop/.config/xfce4/panel/launcher-17
A /home/desktop/.config/xfce4/panel/launcher-17/17937337231.desktop
A /home/desktop/.config/xfce4/xfconf
A /home/desktop/.config/xfce4/xfconf/xfce-perchannel-xml
A /home/desktop/.config/xfce4/xfconf/xfce-perchannel-xml/displays.xml
A /home/desktop/.config/xfce4/xfconf/xfce-perchannel-xml/xfce4-panel.xml
A /home/desktop/.config/xfce4/xfwm4
A /home/desktop/.local
A /home/desktop/.local/share
A /home/desktop/Music
A /home/desktop/Pictures
A /home/desktop/.cache
A /home/desktop/.cache/sessions
A /home/desktop/Downloads
A /home/desktop/.dbus
A /home/desktop/.dbus/session-bus
A /home/desktop/.dbus/session-bus/5657e85467ae204ec13c5cf46ae07fe4-0
A /home/desktop/Documents
";

/// The same of a fresh GNOME desktop, `ghcr.io/co3moz/thoric/ubuntu-gnome:24.04`, with its
/// shader and tracker caches cut to one file each.
const FRESH_GNOME: &str = "C /home
C /home/desktop
A /home/desktop/Documents
A /home/desktop/Downloads
A /home/desktop/Videos
A /home/desktop/.config
A /home/desktop/.config/autostart
A /home/desktop/.config/autostart/org.gnome.SettingsDaemon.Power.desktop
A /home/desktop/.config/autostart/org.gnome.SettingsDaemon.Rfkill.desktop
A /home/desktop/.config/autostart/org.gnome.SettingsDaemon.Sharing.desktop
A /home/desktop/.config/autostart/org.gnome.SettingsDaemon.Smartcard.desktop
A /home/desktop/.config/autostart/org.gnome.SettingsDaemon.UsbProtection.desktop
A /home/desktop/.config/autostart/pulseaudio.desktop
A /home/desktop/.config/dconf
A /home/desktop/.config/dconf/user
A /home/desktop/.config/gnome-session
A /home/desktop/.config/gnome-session/saved-session
A /home/desktop/.config/ibus
A /home/desktop/.config/ibus/bus
A /home/desktop/.config/ibus/bus/5657e85467ae204ec13c5cf46ae07fe4-unix-0
A /home/desktop/.config/nautilus
A /home/desktop/.config/pulse
A /home/desktop/.config/pulse/cookie
A /home/desktop/.config/thoric-gnome-initialized
A /home/desktop/.config/.gsd-keyboard.settings-ported
A /home/desktop/.dbus
A /home/desktop/.dbus/session-bus
A /home/desktop/.dbus/session-bus/5657e85467ae204ec13c5cf46ae07fe4-0
A /home/desktop/.local
A /home/desktop/.local/share
A /home/desktop/.local/share/gnome-settings-daemon
A /home/desktop/.local/share/gnome-settings-daemon/input-sources-converted
A /home/desktop/.local/share/gnome-shell
A /home/desktop/.local/share/gnome-shell/lock-warning-shown
A /home/desktop/.local/share/gnome-shell/update-check-46
A /home/desktop/.local/share/icc
A /home/desktop/.local/share/nautilus
A /home/desktop/.local/share/nautilus/tags
A /home/desktop/.local/share/nautilus/tags/meta.db
A /home/desktop/.local/share/nautilus/tracker2-migration-complete
A /home/desktop/.local/share/session_migration-gnome
A /home/desktop/.local/share/sounds
A /home/desktop/.local/share/applications
A /home/desktop/.local/share/flatpak
A /home/desktop/.local/share/flatpak/db
A /home/desktop/Desktop
A /home/desktop/Music
A /home/desktop/Pictures
A /home/desktop/.cache
A /home/desktop/.cache/gstreamer-1.0
A /home/desktop/.cache/gstreamer-1.0/registry.x86_64.bin
A /home/desktop/.cache/mesa_shader_cache
A /home/desktop/.cache/mesa_shader_cache/index
A /home/desktop/.cache/tracker3
A /home/desktop/.cache/tracker3/files/meta.db
C /etc
C /etc/sudoers.d
A /etc/sudoers.d/desktop
A /etc/sudoers.d/thoric-proxy
C /etc/profile.d
A /etc/profile.d/desktop.sh
C /var
C /var/lib
C /var/lib/geoclue
A /var/lib/geoclue/.cache
A /var/lib/geoclue/.cache/dconf
A /var/lib/geoclue/.cache/dconf/user
A /var/lib/udisks2
C /var/lib/AccountsService
C /var/lib/AccountsService/icons
C /var/lib/AccountsService/users
A /var/lib/AccountsService/users/desktop
C /var/lib/colord
A /var/lib/colord/storage.db
A /var/lib/colord/.cache
A /var/lib/colord/mapping.db
";

/// What an agent did in a desktop: a project, a note in a standard folder, a package, a sudo
/// rule of its own, an MCP server and a deleted system file, beside apt's lists and logs.
const AGENT: &str = "A /home/desktop/project
A /home/desktop/project/main.py
A /home/desktop/project/data
A /home/desktop/project/data/input.csv
A /home/desktop/Documents/report.md
A /home/desktop/.config/thoric
A /home/desktop/.config/thoric/mcp
A /home/desktop/.config/thoric/mcp/browser.json
A /home/desktop/.bash_history
C /usr
C /usr/local
C /usr/local/bin
A /usr/local/bin/tool
C /var/lib/dpkg
C /var/lib/dpkg/status
C /var/lib/apt
C /var/lib/apt/lists
A /var/lib/apt/lists/archive.ubuntu.com_ubuntu_dists_noble_InRelease
C /var/log
C /var/log/apt
A /var/log/apt/history.log
A /etc/sudoers.d/agent
D /etc/legal
";

/// What the WSL GPU and the NVIDIA Container Toolkit add to a fresh XFCE desktop, as Docker listed
/// it for one created as Thoric creates a GPU desktop in WSL: `--gpus all`, `/dev/dxg` and
/// `/usr/lib/wsl` mounted.
const WSL_GPU: &str = "C /usr
C /usr/bin
A /usr/bin/nvidia-smi
C /usr/lib
A /usr/lib/wsl
C /usr/lib/x86_64-linux-gnu
A /usr/lib/x86_64-linux-gnu/libcuda.so
A /usr/lib/x86_64-linux-gnu/libnvidia-gpucomp.so
A /usr/lib/x86_64-linux-gnu/libcuda.so.1
A /usr/lib/x86_64-linux-gnu/libnvidia-ml.so.1
A /usr/lib/x86_64-linux-gnu/libdxcore.so
A /usr/lib/x86_64-linux-gnu/libcudadebugger.so.1
A /usr/lib/x86_64-linux-gnu/libnvidia-gpucomp.so.615.71.08
C /etc/ld.so.cache
C /var
C /var/cache
C /var/cache/ldconfig
C /var/cache/ldconfig/aux-cache
";

fn sorted(text: &str) -> Vec<Change> {
    let mut entries = super::super::source::cli_entries(text);
    sort(&mut entries);
    entries
}

/// A desktop without mounts or the toolkit.
fn plain() -> Desktop {
    Desktop::default()
}

/// The entries no rule matches.
fn shown(entries: &[Change], desktop: &Desktop) -> Vec<String> {
    entries
        .iter()
        .zip(classify(entries, desktop))
        .filter(|(_, rule)| rule.is_none())
        .map(|(entry, _)| format!("{} {}", entry.kind.id(), entry.path))
        .collect()
}

#[test]
fn a_fresh_desktop_of_either_image_lists_nothing() {
    for fresh in [FRESH_XFCE, FRESH_GNOME] {
        let entries = sorted(fresh);
        assert!(!entries.is_empty());
        assert_eq!(shown(&entries, &plain()), Vec::<String>::new());
    }
}

#[test]
fn a_fresh_gpu_desktop_in_wsl_lists_nothing_while_the_same_files_elsewhere_stay() {
    let entries = sorted(&format!("{FRESH_XFCE}{WSL_GPU}"));
    let gpu = Desktop {
        mounts: vec!["/run".into(), "/tmp".into(), "/usr/lib/wsl".into()],
        toolkit: true,
    };
    assert_eq!(shown(&entries, &gpu), Vec::<String>::new());
    // Without the toolkit, files of those names are the container's own.
    let listed = shown(
        &entries,
        &Desktop {
            toolkit: false,
            ..gpu
        },
    );
    assert!(
        listed.contains(&"added /usr/bin/nvidia-smi".to_owned()),
        "{listed:?}"
    );
    assert!(listed.contains(&"added /usr/lib/x86_64-linux-gnu/libcuda.so.1".to_owned()));
}

#[test]
fn what_an_agent_does_afterwards_stays_and_its_folders_go() {
    let entries = sorted(&format!("{FRESH_XFCE}{AGENT}"));
    assert_eq!(
        shown(&entries, &plain()),
        [
            "deleted /etc/legal",
            "added /etc/sudoers.d/agent",
            "added /home/desktop/.bash_history",
            "added /home/desktop/.config/thoric",
            "added /home/desktop/.config/thoric/mcp",
            "added /home/desktop/.config/thoric/mcp/browser.json",
            "added /home/desktop/Documents/report.md",
            "added /home/desktop/project",
            "added /home/desktop/project/data",
            "added /home/desktop/project/data/input.csv",
            "added /home/desktop/project/main.py",
            "added /usr/local/bin/tool",
            "changed /var/lib/dpkg/status",
        ]
    );
    let rule_of = |path: &str| {
        let index = entries.iter().position(|entry| entry.path == path).unwrap();
        classify(&entries, &plain())[index]
    };
    assert_eq!(rule_of("/home/desktop/Documents"), Some("session"));
    assert_eq!(rule_of("/usr/local/bin"), Some(FOLDERS.0));
    assert_eq!(rule_of("/var/log/apt/history.log"), Some("caches"));
    assert_eq!(rule_of("/etc/sudoers.d/desktop"), Some("start"));
}

#[test]
fn code_the_desktop_runs_and_group_membership_are_never_left_out() {
    // A GNOME Shell extension, a file manager script, a Thunar setting beside its default
    // actions, and the groups a start with render devices joins.
    let entries = sorted(&format!(
        "{FRESH_GNOME}A /home/desktop/.local/share/gnome-shell/extensions
A /home/desktop/.local/share/gnome-shell/extensions/agent@example/extension.js
A /home/desktop/.local/share/nautilus/scripts
A /home/desktop/.local/share/nautilus/scripts/run.sh
A /home/desktop/.config/Thunar
A /home/desktop/.config/Thunar/uca.xml
A /home/desktop/.config/Thunar/accels.scm
C /etc/group
C /etc/gshadow
"
    ));
    assert_eq!(
        shown(&entries, &plain()),
        [
            "changed /etc/group",
            "changed /etc/gshadow",
            "added /home/desktop/.config/Thunar/accels.scm",
            "added /home/desktop/.local/share/gnome-shell/extensions",
            "added /home/desktop/.local/share/gnome-shell/extensions/agent@example/extension.js",
            "added /home/desktop/.local/share/nautilus/scripts",
            "added /home/desktop/.local/share/nautilus/scripts/run.sh",
        ]
    );
}

#[test]
fn a_mount_point_and_the_folders_made_to_hold_it_go_while_a_deleted_folder_stays() {
    let entries = sorted(
        "C /home/desktop
A /home/desktop/work
A /home/desktop/work/unity
A /usr/lib/wsl
D /opt/old
",
    );
    let desktop = Desktop {
        mounts: vec!["/home/desktop/work/unity".to_owned(), "/usr/lib/wsl".into()],
        toolkit: false,
    };
    let rules = classify(&entries, &desktop);
    let by_path: Vec<(&str, Option<&str>)> = entries
        .iter()
        .map(|entry| entry.path.as_str())
        .zip(rules)
        .collect();
    assert_eq!(
        by_path,
        [
            ("/home/desktop", Some(FOLDERS.0)),
            ("/home/desktop/work", Some(FOLDERS.0)),
            ("/home/desktop/work/unity", Some(MOUNTS.0)),
            ("/opt/old", None),
            ("/usr/lib/wsl", Some(MOUNTS.0)),
        ]
    );
}

#[test]
fn a_changed_folder_with_nothing_below_it_is_a_change_of_its_own() {
    // A folder whose own mode changed, beside one whose file did.
    let entries = sorted("C /srv\nC /etc\nC /etc/hosts.allow\n");
    assert_eq!(
        shown(&entries, &plain()),
        ["changed /etc/hosts.allow", "changed /srv"]
    );
}

#[test]
fn what_is_below_a_path_follows_it_before_a_name_that_shares_its_start() {
    let entries =
        sorted("A /home/desktop-old\nA /home/desktop/a\nC /home/desktop\nA /home/desktop.d\n");
    let paths: Vec<&str> = entries.iter().map(|entry| entry.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "/home/desktop",
            "/home/desktop/a",
            "/home/desktop-old",
            "/home/desktop.d"
        ]
    );
    // A neighbour that shares the start is not below the folder, so the folder holds one entry.
    assert_eq!(
        shown(&entries, &plain()),
        [
            "added /home/desktop/a",
            "added /home/desktop-old",
            "added /home/desktop.d"
        ]
    );
}

#[test]
fn every_rule_path_is_canonical_and_rules_are_named_once() {
    let rules: Vec<&Rule> = RULES.iter().chain([&TOOLKIT]).collect();
    let mut ids: Vec<&str> = rules.iter().map(|rule| rule.id).collect();
    ids.extend([MOUNTS.0, FOLDERS.0]);
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len());
    for rule in rules {
        for (path, _) in rule.paths {
            crate::apps::directory(path).unwrap();
            assert_ne!(*path, "/", "{}", rule.id);
        }
    }
    assert!(within("/tmp/.X11-unix/X0", "/tmp"));
    assert!(!within("/tmpfile", "/tmp"));
    assert!(within("/anything", "/"));
}

#[test]
fn the_dashboard_names_every_rule() {
    // The Changes view labels an entry by its rule; a rule without a label shows its bare id.
    let catalog = include_str!("../../../web/src/locales/en.ts");
    for id in RULES
        .iter()
        .chain([&TOOLKIT])
        .map(|rule| rule.id)
        .chain([MOUNTS.0, FOLDERS.0])
    {
        assert!(
            catalog.contains(&format!("\"changes.rule.{id}\":")),
            "core/web/src/locales/en.ts needs changes.rule.{id}"
        );
    }
}
