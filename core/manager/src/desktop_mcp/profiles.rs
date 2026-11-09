//! Tool profiles: a named choice of Thoric's container tools that a grant takes on top of its
//! scopes, so an agent that only runs scripts is not offered the screen's tools. A profile is
//! resolved when an endpoint lists and calls its tools, so a tool a later release adds follows
//! the profile of every grant already saved. Desktop servers' tools are no part of one: a grant
//! chooses those with `servers`.
//!
//! The rule asks two things of every tool an endpoint can offer:
//!
//! 1. Does it work only through the desktop's screen, sound or input devices: the picture, the
//!    windows, the pointer and keyboard, the clipboard, the accessibility tree, recordings,
//!    audio or the gamepad? Those are the screen's tools, and `headless` leaves them out. A tool
//!    that also works without them, such as `wait_for_condition`, whose conditions name
//!    applications too, is not one.
//! 2. Is it part of the everyday loop, look, act, start a program, read and write a file, run a
//!    script, or is it for looking deeper or for a special device: diagnostics, metrics,
//!    processes, terminals, changes, recordings, audio, the gamepad? `desktop` keeps the
//!    everyday ones.
//!
//! That gives four kinds, and every tool is of exactly one, which the tests hold to the scopes.
use serde::{Deserialize, Serialize};

/// Which of the tools its scopes give a grant offers.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Profile {
    /// Every tool of the scopes: what a grant saved before profiles has.
    #[default]
    Full,
    /// Scripts, terminals, files, applications, processes and jobs, without the screen's tools.
    Headless,
    /// Screen and input work with the everyday tools, without the rarely used ones.
    Desktop,
}

/// What kind of tool one is, by the two questions above.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Everyday and without the screen: in every profile.
    Core,
    /// Everyday on the screen: `full` and `desktop`.
    Screen,
    /// Deeper, without the screen: `full` and `headless`.
    Deep,
    /// Deeper or a special device, on the screen: `full` alone.
    ScreenDeep,
}

/// Everyday tools that need no screen, and the ones an endpoint cannot do without: its jobs, its
/// set's instances and asking a person.
const CORE: &[&str] = &[
    "get_container",
    "wait_for_condition",
    "request_human",
    "launch_application",
    "list_applications",
    "get_application",
    "application_logs",
    "stop_application",
    "list_files",
    "read_file",
    "download_file",
    "search_files",
    "write_file",
    "upload_file",
    "move_file",
    "delete_file",
    "exec_container",
    "container_action",
    "create_instance",
    "delete_instance",
    "list_instances",
    "get_job",
];
/// Everyday tools of the screen: looking at it and acting on it.
const SCREEN: &[&str] = &[
    "screenshot",
    "list_windows",
    "find_text",
    "get_clipboard",
    "desktop_input",
    "act_and_observe",
    "focus_window",
    "set_clipboard",
];
/// Tools for looking deeper that need no screen.
const DEEP: &[&str] = &[
    "container_logs",
    "container_diagnostics",
    "get_usage",
    "list_processes",
    "application_diagnostics",
    "sample_metrics",
    "kill_process",
    "get_changes",
    "open_terminal",
    "list_terminals",
    "terminal_input",
    "terminal_output",
    "close_terminal",
];
/// Rarely used tools of the screen, sound and gamepad.
const SCREEN_DEEP: &[&str] = &[
    "inspect_ui",
    "set_window_geometry",
    "gamepad_input",
    "save_screenshot",
    "start_recording",
    "stop_recording",
    "list_recordings",
    "export_artifacts",
    "capture_audio",
    "inject_audio",
];

const KINDS: [(Kind, &[&str]); 4] = [
    (Kind::Core, CORE),
    (Kind::Screen, SCREEN),
    (Kind::Deep, DEEP),
    (Kind::ScreenDeep, SCREEN_DEEP),
];

/// The kind of the container tool `name`, or nothing for a name no rule knows.
pub(crate) fn kind(name: &str) -> Option<Kind> {
    KINDS
        .iter()
        .find(|(_, names)| names.contains(&name))
        .map(|(kind, _)| *kind)
}

impl Profile {
    pub const ALL: [Profile; 3] = [Profile::Full, Profile::Headless, Profile::Desktop];
    pub fn id(self) -> &'static str {
        match self {
            Profile::Full => "full",
            Profile::Headless => "headless",
            Profile::Desktop => "desktop",
        }
    }
    pub fn is_full(&self) -> bool {
        *self == Profile::Full
    }
    /// Whether the profile keeps tools of this kind.
    fn keeps(self, kind: Kind) -> bool {
        match self {
            Profile::Full => true,
            Profile::Headless => matches!(kind, Kind::Core | Kind::Deep),
            Profile::Desktop => matches!(kind, Kind::Core | Kind::Screen),
        }
    }
    /// Whether the profile offers the container tool `name`. A name no rule knows is offered by
    /// `full` alone, so a narrower profile never widens by an oversight.
    pub fn includes(self, name: &str) -> bool {
        match kind(name) {
            Some(kind) => self.keeps(kind),
            None => self.is_full(),
        }
    }
    /// The container tools the profile offers, in the rules' order.
    pub fn tools(self) -> Vec<&'static str> {
        KINDS
            .iter()
            .filter(|(kind, _)| self.keeps(*kind))
            .flat_map(|(_, names)| names.iter().copied())
            .collect()
    }
}

#[cfg(test)]
#[path = "profiles_tests.rs"]
mod tests;
