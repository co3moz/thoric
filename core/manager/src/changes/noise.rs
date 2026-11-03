//! What a desktop changes on its own, which a change list leaves out unless asked for all of it.
//! The rules come from what a fresh desktop of each catalog image changes once its session has
//! started, also with the GPU through the WSL GPU and the NVIDIA Container Toolkit: with them such
//! a fresh desktop lists nothing (`noise_tests.rs` holds them to it), and what is left is what
//! happened in it afterwards. Each entry matches one rule at most, the first that holds, and an
//! answer names every rule with how many entries it matched. A desktop with render devices under
//! `/dev/dri` still lists `/etc/group` and `/etc/gshadow`, which its start changes to join their
//! groups: who belongs to which group is never left out.
use super::source::{Change, Kind};

/// A path of a rule: the path alone, such as a folder the session makes, whose files still show,
/// the path and everything under it, or every path that starts with it, such as the libraries of
/// one name in a folder.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Reach {
    Exact,
    Tree,
    Prefix,
}
use Reach::*;

pub(crate) struct Rule {
    pub id: &'static str,
    pub description: &'static str,
    pub paths: &'static [(&'static str, Reach)],
}
impl Rule {
    fn matches(&self, path: &str) -> bool {
        self.paths.iter().any(|(rule_path, reach)| match reach {
            Exact => path == *rule_path,
            Tree => within(path, rule_path),
            Prefix => path.starts_with(rule_path),
        })
    }
}

/// The rules that match paths, in the order they are tried.
pub(crate) const RULES: [Rule; 5] = [
    Rule {
        id: "system",
        description: "What the kernel and Docker provide in place of the container's own files.",
        paths: &[("/proc", Tree), ("/sys", Tree), ("/dev", Tree)],
    },
    Rule {
        id: "temporary",
        description: "Temporary and runtime files, such as the X11 and ICE sockets, which a desktop keeps in memory.",
        paths: &[
            ("/tmp", Tree),
            ("/run", Tree),
            ("/var/tmp", Tree),
            ("/var/run", Tree),
            ("/var/lock", Tree),
        ],
    },
    Rule {
        id: "caches",
        description: "Caches and logs that programs rebuild or append to, such as the library cache ldconfig writes, and the package lists apt downloads.",
        paths: &[
            ("/etc/ld.so.cache", Exact),
            ("/var/cache", Tree),
            ("/var/log", Tree),
            ("/var/lib/apt/lists", Tree),
            ("/home/desktop/.cache", Tree),
            ("/root/.cache", Tree),
        ],
    },
    Rule {
        id: "start",
        description: "What every start of the desktop writes: its sudo rules, its login profile and the machine ID Thoric gives it.",
        paths: &[
            ("/etc/sudoers.d/desktop", Exact),
            ("/etc/sudoers.d/thoric-proxy", Exact),
            ("/etc/profile.d/desktop.sh", Exact),
            ("/etc/machine-id", Exact),
        ],
    },
    Rule {
        id: "session",
        description: "What the desktop session keeps on its own: its bus, sound and input method, the desktop environment's settings and state, the services GNOME starts and the standard folders it makes, whose files still show.",
        paths: &[
            ("/home/desktop/.ICEauthority", Exact),
            ("/home/desktop/.Xauthority", Exact),
            ("/home/desktop/.dbus", Tree),
            ("/home/desktop/.config", Exact),
            ("/home/desktop/.config/.gsd-keyboard.settings-ported", Exact),
            ("/home/desktop/.config/autostart", Exact),
            (
                "/home/desktop/.config/autostart/org.gnome.SettingsDaemon.Power.desktop",
                Exact,
            ),
            (
                "/home/desktop/.config/autostart/org.gnome.SettingsDaemon.Rfkill.desktop",
                Exact,
            ),
            (
                "/home/desktop/.config/autostart/org.gnome.SettingsDaemon.Sharing.desktop",
                Exact,
            ),
            (
                "/home/desktop/.config/autostart/org.gnome.SettingsDaemon.Smartcard.desktop",
                Exact,
            ),
            (
                "/home/desktop/.config/autostart/org.gnome.SettingsDaemon.UsbProtection.desktop",
                Exact,
            ),
            ("/home/desktop/.config/autostart/pulseaudio.desktop", Exact),
            ("/home/desktop/.config/dconf", Tree),
            ("/home/desktop/.config/gnome-session", Tree),
            ("/home/desktop/.config/ibus", Tree),
            ("/home/desktop/.config/nautilus", Exact),
            ("/home/desktop/.config/pulse", Tree),
            ("/home/desktop/.config/thoric-gnome-initialized", Exact),
            // Thunar's own actions are the default it writes; other files of it show.
            ("/home/desktop/.config/Thunar", Exact),
            ("/home/desktop/.config/Thunar/uca.xml", Exact),
            ("/home/desktop/.config/user-dirs.dirs", Exact),
            ("/home/desktop/.config/user-dirs.locale", Exact),
            ("/home/desktop/.config/xfce4", Tree),
            ("/home/desktop/.local", Exact),
            ("/home/desktop/.local/share", Exact),
            ("/home/desktop/.local/share/applications", Exact),
            ("/home/desktop/.local/share/flatpak", Exact),
            ("/home/desktop/.local/share/flatpak/db", Exact),
            ("/home/desktop/.local/share/gnome-settings-daemon", Tree),
            // GNOME Shell's own marks, never its extensions, which it runs.
            ("/home/desktop/.local/share/gnome-shell", Exact),
            (
                "/home/desktop/.local/share/gnome-shell/lock-warning-shown",
                Exact,
            ),
            (
                "/home/desktop/.local/share/gnome-shell/update-check-",
                Prefix,
            ),
            ("/home/desktop/.local/share/gvfs-metadata", Tree),
            ("/home/desktop/.local/share/icc", Exact),
            // The file manager's index, never its scripts.
            ("/home/desktop/.local/share/nautilus", Exact),
            ("/home/desktop/.local/share/nautilus/tags", Tree),
            (
                "/home/desktop/.local/share/nautilus/tracker2-migration-complete",
                Exact,
            ),
            ("/home/desktop/.local/share/recently-used.xbel", Exact),
            ("/home/desktop/.local/share/session_migration-gnome", Exact),
            ("/home/desktop/.local/share/sounds", Exact),
            ("/home/desktop/Desktop", Exact),
            ("/home/desktop/Documents", Exact),
            ("/home/desktop/Downloads", Exact),
            ("/home/desktop/Music", Exact),
            ("/home/desktop/Pictures", Exact),
            ("/home/desktop/Public", Exact),
            ("/home/desktop/Templates", Exact),
            ("/home/desktop/Videos", Exact),
            ("/var/lib/AccountsService", Tree),
            ("/var/lib/colord", Tree),
            ("/var/lib/geoclue", Tree),
            ("/var/lib/udisks2", Tree),
        ],
    },
];

/// The driver files the NVIDIA Container Toolkit mounts into a desktop that has the GPU through
/// it, over empty files it makes in the container's own files: tried only for such a desktop, so
/// a file of these names in any other is listed. In WSL these are `nvidia-smi`, `libcuda`,
/// `libnvidia-*`, `libdxcore` and `libcudadebugger`; the graphics libraries, Vulkan and EGL files
/// and the firmware are what the toolkit adds on a Linux host for the graphics capability.
pub(crate) const TOOLKIT: Rule = Rule {
    id: "gpu",
    description: "The driver files and tools the NVIDIA Container Toolkit mounts into a desktop that has the GPU through it.",
    paths: &[
        ("/usr/bin/nvidia-", Prefix),
        ("/usr/lib/x86_64-linux-gnu/libcuda", Prefix),
        ("/usr/lib/x86_64-linux-gnu/libdxcore", Prefix),
        ("/usr/lib/x86_64-linux-gnu/libnvidia-", Prefix),
        ("/usr/lib/x86_64-linux-gnu/libnvcuvid", Prefix),
        ("/usr/lib/x86_64-linux-gnu/libnvoptix", Prefix),
        ("/usr/lib/x86_64-linux-gnu/libEGL_nvidia", Prefix),
        ("/usr/lib/x86_64-linux-gnu/libGLX_nvidia", Prefix),
        ("/usr/lib/x86_64-linux-gnu/libGLESv1_CM_nvidia", Prefix),
        ("/usr/lib/x86_64-linux-gnu/libGLESv2_nvidia", Prefix),
        ("/usr/lib/x86_64-linux-gnu/gbm/nvidia", Prefix),
        ("/usr/lib/x86_64-linux-gnu/vdpau/libvdpau_nvidia", Prefix),
        ("/usr/lib/aarch64-linux-gnu/libcuda", Prefix),
        ("/usr/lib/aarch64-linux-gnu/libdxcore", Prefix),
        ("/usr/lib/aarch64-linux-gnu/libnvidia-", Prefix),
        ("/usr/lib/aarch64-linux-gnu/libnvcuvid", Prefix),
        ("/usr/lib/aarch64-linux-gnu/libnvoptix", Prefix),
        ("/usr/lib/aarch64-linux-gnu/libEGL_nvidia", Prefix),
        ("/usr/lib/aarch64-linux-gnu/libGLX_nvidia", Prefix),
        ("/usr/lib/aarch64-linux-gnu/libGLESv1_CM_nvidia", Prefix),
        ("/usr/lib/aarch64-linux-gnu/libGLESv2_nvidia", Prefix),
        ("/usr/lib/aarch64-linux-gnu/gbm/nvidia", Prefix),
        ("/usr/lib/aarch64-linux-gnu/vdpau/libvdpau_nvidia", Prefix),
        ("/usr/share/glvnd/egl_vendor.d/10_nvidia", Prefix),
        ("/usr/share/egl/egl_external_platform.d/10_nvidia", Prefix),
        ("/usr/share/egl/egl_external_platform.d/15_nvidia", Prefix),
        ("/usr/share/vulkan/icd.d/nvidia", Prefix),
        ("/usr/share/vulkan/implicit_layer.d/nvidia", Prefix),
        ("/etc/vulkan/icd.d/nvidia", Prefix),
        ("/etc/vulkan/implicit_layer.d/nvidia", Prefix),
        ("/usr/lib/firmware/nvidia", Tree),
        ("/lib/firmware/nvidia", Tree),
    ],
};

/// Where a volume or a host folder is mounted: the folder Docker made for it, whose files are not
/// in the list at all.
pub(crate) const MOUNTS: (&str, &str) = (
    "mounts",
    "The folders volumes and host folders are mounted at; their files are not in the list.",
);
/// A folder that is in the list only for what is below it, such as one made to hold a mount.
pub(crate) const FOLDERS: (&str, &str) = (
    "folders",
    "A changed folder whose entries below say what changed in it, and a new folder whose entries below are all left out.",
);

/// What a desktop's Docker container says about it that the rules depend on.
#[derive(Debug, Default)]
pub(crate) struct Desktop {
    /// The folders mounted in it.
    pub mounts: Vec<String>,
    /// Whether it has the GPU through the NVIDIA Container Toolkit, Docker's `--gpus`.
    pub toolkit: bool,
}

/// Whether `path` is `folder` or inside it.
pub(crate) fn within(path: &str, folder: &str) -> bool {
    folder == "/"
        || path
            .strip_prefix(folder)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// The rule of the path rules that `path` matches in `desktop`.
fn by_path(path: &str, desktop: &Desktop) -> Option<&'static str> {
    RULES
        .iter()
        .chain(desktop.toolkit.then_some(&TOOLKIT))
        .find(|rule| rule.matches(path))
        .map(|rule| rule.id)
}

/// Sort `entries` so that everything below a path follows it, and nothing else does before the
/// next path beside it: by the path's names in order, where `/` comes before any other character.
pub(crate) fn sort(entries: &mut [Change]) {
    entries.sort_by(|a, b| a.path.split('/').cmp(b.path.split('/')));
}

/// The rule each entry matches, or `None` for what happened in the desktop. `entries` are in the
/// order [`sort`] gives.
pub(crate) fn classify(entries: &[Change], desktop: &Desktop) -> Vec<Option<&'static str>> {
    let count = entries.len();
    // Where the entries below each one end, with what is below a path right after it.
    let mut end = vec![count; count];
    let mut open: Vec<usize> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        while let Some(&top) = open.last() {
            if within(&entry.path, &entries[top].path) {
                break;
            }
            end[top] = index;
            open.pop();
        }
        open.push(index);
    }
    // From the last entry back, so a folder's entries below are decided before it; `shown[i]`
    // counts the entries from `i` on that no rule matched.
    let mut rules = vec![None; count];
    let mut shown = vec![0usize; count + 1];
    for index in (0..count).rev() {
        let entry = &entries[index];
        let below = end[index] - index - 1;
        let shown_below = shown[index + 1] - shown[end[index]];
        rules[index] = by_path(&entry.path, desktop)
            .or_else(|| desktop.mounts.contains(&entry.path).then_some(MOUNTS.0))
            .or_else(|| {
                (below > 0
                    && match entry.kind {
                        Kind::Changed => true,
                        Kind::Added => shown_below == 0,
                        Kind::Deleted => false,
                    })
                .then_some(FOLDERS.0)
            });
        shown[index] = shown[index + 1] + usize::from(rules[index].is_none());
    }
    rules
}

#[cfg(test)]
#[path = "noise_tests.rs"]
mod tests;
