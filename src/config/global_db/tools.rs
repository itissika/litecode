//! Core tool id sets shared by seed (CONFIG §2.5).

/// Model-facing code members behind the `memory` capability card.
pub const MEMORY_TOOL_IDS: &[&str] = &["memory.list", "memory.upsert", "memory.delete"];
pub const TODO_TOOL_IDS: &[&str] = &["todo.write"];
pub const PLAN_TOOL_IDS: &[&str] = &["plan.create", "plan.finish"];
pub const KNOWLEDGE_TOOL_IDS: &[&str] = &[
    "knowledge.list",
    "knowledge.check",
    "knowledge.refs",
    "knowledge.create",
];
pub const WORKSPACE_TOOL_IDS: &[&str] = &["workspace.help", "workspace.refresh", "workspace.status"];
pub const BOT_TOOL_IDS: &[&str] = &["bot.create", "bot.list", "bot.send"];

pub fn core_configurable_tools() -> &'static [&'static str] {
    &[
        "read",
        "grep",
        "glob",
        "write",
        "edit",
        "exec",
        "session_search",
        "litecode_workspace",
    ]
}

pub fn core_none_tools() -> &'static [&'static str] {
    &[
        "plan",
        "todo",
        "memory",
        "ask_user",
        "knowledge",
        crate::config::schema::SUBAGENT_CAPABILITY_ID,
    ]
}

pub fn core_tool_ids() -> Vec<String> {
    core_configurable_tools()
        .iter()
        .chain(core_none_tools().iter())
        .map(|s| (*s).to_string())
        .collect()
}

pub fn is_core_tool(id: &str) -> bool {
    core_configurable_tools().contains(&id)
        || core_none_tools().contains(&id)
        // Members are no longer cards, but their model names stay reserved.
        || crate::config::schema::SUBAGENT_MEMBER_TOOL_IDS.contains(&id)
        || MEMORY_TOOL_IDS.contains(&id)
        || TODO_TOOL_IDS.contains(&id)
        || PLAN_TOOL_IDS.contains(&id)
        || KNOWLEDGE_TOOL_IDS.contains(&id)
        || WORKSPACE_TOOL_IDS.contains(&id)
        || BOT_TOOL_IDS.contains(&id)
}

pub fn optional_builtin_ids() -> &'static [&'static str] {
    &["code_search", "lsp", "browser", "webfetch", "websearch"]
}

/// Optional builtins that primary-role agents bind by default (WP-146 §B).
pub fn default_bound_optional_builtins() -> &'static [&'static str] {
    &["code_search"]
}

pub fn is_optional_builtin(id: &str) -> bool {
    optional_builtin_ids().contains(&id)
}

pub fn is_reserved_builtin(id: &str) -> bool {
    is_core_tool(id) || is_optional_builtin(id) || reserved_command_name_rejection(id).is_some()
}

/// `run_code` belongs to the code transport. Every create, write, and load path
/// for a custom tool — and any other non-transport source that would publish a
/// tool name — must reject with this function. `None` means the name is free.
///
/// Command-job companion names are not listed here. [`crate::tool::registry`]
/// strips this function's hits before projection, and the real `exec_command`
/// / `write_stdin` / `apply_edits` tools must survive that strip. Those names
/// use [`reserved_command_name_rejection`].
pub fn non_transport_name_rejection(name: &str) -> Option<&'static str> {
    if name == "run_code" {
        Some("run_code is reserved for the code transport")
    } else {
        None
    }
}

/// Names a custom tool or an MCP tool must not publish. The built-in
/// `exec_command` / `write_stdin` / `apply_edits` tools are registered
/// separately and are not rejected by this check at projection time. Retired
/// shell names stay reserved so custom tools cannot reclaim them.
pub fn reserved_command_name_rejection(name: &str) -> Option<&'static str> {
    match name {
        "run" | "wait" | "kill" | "exec_command" | "write_stdin" => {
            Some("name is reserved for a code command tool")
        }
        "apply_edits" => Some("name is reserved for a code edit tool"),
        "run_command" | "command_output" | "command_kill" | "bash" | "wait_shell"
        | "kill_shell" => Some("name is reserved and is not a custom or MCP tool"),
        _ => None,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{CustomToolDefinition, McpServerDefinition, ToolSchema};
    use std::collections::HashMap;

    const WEB_TOOLS: [&str; 2] = ["webfetch", "websearch"];

    /// D3: the web cards are optional builtins, not core tools. Their names are
    /// still reserved so a custom tool or an MCP server cannot take them.
    #[test]
    fn web_engines_are_optional_builtins_with_reserved_names() {
        for id in WEB_TOOLS {
            assert!(
                optional_builtin_ids().contains(&id),
                "{id} must be an optional builtin"
            );
            assert!(is_optional_builtin(id), "{id} is_optional_builtin");
            assert!(!is_core_tool(id), "{id} must not be a core tool");
            assert!(is_reserved_builtin(id), "{id} name must stay reserved");
            assert!(
                !core_tool_ids().iter().any(|core| core == id),
                "{id} must not be in core_tool_ids"
            );
            assert!(
                !is_workspace_optional(id),
                "{id} is an install-level engine, not workspace-level"
            );
        }
    }

    fn web_custom_tool(name: &str) -> CustomToolDefinition {
        CustomToolDefinition {
            name: name.to_string(),
            description: String::new(),
            schema: ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: Vec::new(),
            },
            command: "echo".into(),
            args: Vec::new(),
            timeout: 30,
            rules: Vec::new(),
            suite: None,
        }
    }

    #[test]
    fn custom_tool_and_mcp_cannot_claim_a_web_name() {
        for id in WEB_TOOLS {
            let mut def = web_custom_tool(id);
            let err = crate::config::settings_writer::validate_custom_definition_with_suites(
                id,
                &mut def,
                &HashMap::new(),
            )
            .expect_err("a custom tool must not take a builtin name");
            assert!(
                err.to_string().contains(id),
                "{id}: rejection must name the tool, got {err}"
            );

            let mut mcp = McpServerDefinition {
                command: "echo".into(),
                ..Default::default()
            };
            let err = crate::config::settings_writer::validate_mcp_definition(id, &mut mcp)
                .expect_err("an MCP server must not take a builtin name");
            assert!(
                err.to_string().contains("conflicts with a builtin tool"),
                "{id}: unexpected rejection {err}"
            );
        }
    }
}
