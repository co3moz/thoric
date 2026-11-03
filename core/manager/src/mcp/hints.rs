//! What the catalog says of each tool beyond its schema: the title a client shows a person
//! instead of its name, and MCP's idempotentHint, whether calling a tool that changes something
//! again with the same arguments changes nothing more. A read-only tool is idempotent anyway.
use rmcp::model::{Tool, ToolAnnotations};

/// Tools that change something, but nothing more when called again with the same arguments: they
/// set a state, replace a whole value, or are refused once their work is done, as a delete or a
/// create of a name that exists is. `container_action` is not one, since a second restart
/// restarts again, nor is `request_human`, which asks anew once its request was answered.
pub(super) const IDEMPOTENT: [&str; 23] = [
    "set_internet",
    "update_container",
    "delete_container",
    "pull_image",
    "set_image_compose",
    "delete_image",
    "create_volume",
    "copy_volume",
    "delete_volume",
    "write_file",
    "upload_file",
    "move_file",
    "delete_file",
    "focus_window",
    "set_window_geometry",
    "stop_application",
    "close_terminal",
    "register_mcp_server",
    "remove_mcp_server",
    "create_mcp_endpoint",
    "update_mcp_endpoint",
    "delete_mcp_endpoint",
    "cancel_job",
];

/// Every tool's title.
pub(super) const TITLES: [(&str, &str); 81] = [
    ("get_settings", "Read installation settings"),
    ("get_system", "Check the Docker host"),
    ("pull_image", "Download a catalog image"),
    ("build_catalog_image", "Build a catalog image"),
    ("list_containers", "List containers"),
    ("get_container", "Read a container"),
    ("create_container", "Create a container"),
    ("container_action", "Start, stop or restart a container"),
    ("set_internet", "Turn internet access on or off"),
    ("update_container", "Change a container's settings"),
    ("retry_container", "Repair a container's setup"),
    ("delete_container", "Delete a container"),
    ("container_logs", "Read container logs"),
    ("container_diagnostics", "Inspect a container's desktop"),
    ("exec_container", "Run a script in a container"),
    ("reset_container", "Reset a container"),
    ("list_images", "List images"),
    ("get_image_layers", "Explain an image's size"),
    ("import_image", "Register a host image"),
    ("build_image", "Build an image"),
    ("get_image_compose", "Read an image's compose"),
    ("set_image_compose", "Set an image's compose"),
    ("snapshot", "Snapshot a container"),
    ("delete_image", "Delete an image"),
    ("list_volumes", "List volumes"),
    ("create_volume", "Create a volume"),
    ("copy_volume", "Copy a volume"),
    ("delete_volume", "Delete a volume"),
    ("get_usage", "Read resource usage"),
    ("list_jobs", "List jobs"),
    ("get_job", "Read a job"),
    ("cancel_job", "Cancel a job"),
    ("upload_file", "Upload a file"),
    ("read_file", "Read a file"),
    ("write_file", "Write a file"),
    ("list_files", "List a folder"),
    ("move_file", "Move a file"),
    ("delete_file", "Delete a file"),
    ("download_file", "Download a file"),
    ("screenshot", "Take a screenshot"),
    ("save_screenshot", "Save a screenshot as a file"),
    ("act_and_observe", "Act and take a screenshot"),
    ("desktop_input", "Use the mouse and keyboard"),
    ("find_text", "Find text on the screen"),
    ("wait_for_condition", "Wait for a condition"),
    ("inspect_ui", "Read the accessibility tree"),
    ("launch_application", "Launch an application"),
    ("list_applications", "List applications"),
    ("get_application", "Read an application"),
    ("application_logs", "Read an application's log"),
    ("application_diagnostics", "Inspect an application"),
    ("sample_metrics", "Sample an application's CPU and memory"),
    ("stop_application", "Stop an application"),
    ("list_windows", "List windows"),
    ("focus_window", "Focus a window"),
    ("set_window_geometry", "Move or resize a window"),
    ("start_recording", "Start a screen recording"),
    ("stop_recording", "Stop a screen recording"),
    ("list_recordings", "List recordings"),
    ("export_artifacts", "Export recordings and logs"),
    ("capture_audio", "Record the desktop's sound"),
    ("inject_audio", "Play sound into the desktop"),
    ("get_clipboard", "Read the desktop's clipboard"),
    ("set_clipboard", "Set or paste the desktop's clipboard"),
    ("open_terminal", "Open a terminal"),
    ("list_terminals", "List terminals"),
    ("terminal_input", "Type into a terminal"),
    ("terminal_output", "Read a terminal's output"),
    ("close_terminal", "Close a terminal"),
    ("gamepad_input", "Use the virtual gamepad"),
    ("create_transfer", "Create a transfer link"),
    ("request_human", "Ask a person for help"),
    ("list_mcp_servers", "List desktop MCP servers"),
    ("call_mcp_server_tool", "Call a desktop MCP server's tool"),
    ("register_mcp_server", "Register a desktop MCP server"),
    ("remove_mcp_server", "Remove a desktop MCP server"),
    ("list_mcp_endpoints", "List MCP endpoints"),
    ("create_mcp_endpoint", "Create an MCP endpoint"),
    ("update_mcp_endpoint", "Change an MCP endpoint"),
    ("delete_mcp_endpoint", "Delete an MCP endpoint"),
    ("create_mcp_endpoint_token", "Create an endpoint token"),
];

/// Give every tool its title, on the tool and in its annotations for clients of protocol
/// revisions before the tool had one, and its idempotentHint.
pub(super) fn annotate(tools: &mut [Tool]) {
    for tool in tools {
        let title = TITLES
            .iter()
            .find(|(name, _)| *name == tool.name)
            .map(|(_, title)| (*title).to_owned());
        let annotations = tool.annotations.get_or_insert_with(ToolAnnotations::new);
        let read_only = annotations.read_only_hint == Some(true);
        annotations.idempotent_hint = Some(read_only || IDEMPOTENT.contains(&tool.name.as_ref()));
        annotations.title.clone_from(&title);
        tool.title = title;
    }
}
