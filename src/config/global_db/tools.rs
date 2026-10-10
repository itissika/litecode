//! Core tool id sets shared by seed (CONFIG §2.5).

pub fn core_configurable_tools() -> &'static [&'static str] {
    &[
        "read",
        "grep",
        "glob",
        "write",
        "edit",
        "bash",
        "kill_shell",
        "wait_shell",
        "session_search",
        "litecode_workspace",
    ]
}

pub fn network_core_tools() -> &'static [&'static str] {
    &["webfetch", "websearch"]
}

pub fn core_none_tools() -> &'static [&'static str] {
    &[
        "plan",
        "todo",
        "ask_user",
        "knowledge",
        "subagent_launch",
        "subagent_wait",
        "subagent_stop",
        "subagent_list",
        "subagent_send",
    ]
}

pub fn core_tool_ids() -> Vec<String> {
    core_configurable_tools()
        .iter()
        .chain(core_none_tools().iter())
        .chain(network_core_tools().iter())
        .map(|s| (*s).to_string())
        .collect()
}

pub fn is_core_tool(id: &str) -> bool {
    core_configurable_tools().contains(&id)
        || core_none_tools().contains(&id)
        || network_core_tools().contains(&id)
}

pub fn optional_builtin_ids() -> &'static [&'static str] {
    &["code_search", "lsp"]
}

pub fn is_optional_builtin(id: &str) -> bool {
    optional_builtin_ids().contains(&id)
}

pub fn is_reserved_builtin(id: &str) -> bool {
    is_core_tool(id) || is_optional_builtin(id)
}

/// A custom tool may own permission only when the catalog key is the body name
/// and that name is not a builtin. Reserved names always keep the binding.
pub fn is_custom_tool_identity(key: &str, body_name: &str) -> bool {
    !key.is_empty() && key == body_name && !is_reserved_builtin(key)
}

pub fn is_workspace_optional(id: &str) -> bool {
    matches!(id, "code_search" | "lsp")
}

pub fn mcp_catalog_id(server_id: &str) -> String {
    format!("mcp_{server_id}")
}

pub fn is_mcp_catalog_id(id: &str) -> bool {
    id.starts_with("mcp_")
}

pub fn normalize_mcp_tool_name(name: &str) -> Option<String> {
    if name.starts_with("mcp_") {
        Some(name.to_string())
    } else {
        Some(mcp_catalog_id(name))
    }
}
