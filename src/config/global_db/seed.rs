use rusqlite::{Connection, OptionalExtension, params};

use crate::config::schema::{
    AgentRole, PRIMARY_ONLY_TOOL_IDS, SUBAGENT_SERIES_TOOL_IDS, ToolPreset,
};
use crate::types::Result;

use super::builtin_prompts::{
    DEFAULT_DESCRIPTION, EXPLORE_DESCRIPTION, GENERAL_DESCRIPTION, ORCHESTRATOR_DESCRIPTION,
};
use super::store;
use super::tools::{
    core_configurable_tools, core_none_tools, default_bound_optional_builtins, network_core_tools,
};

pub const SEED_REVISION: &str = "16";

pub fn seed(conn: &Connection) -> Result<()> {
    let _ = conn.execute("DELETE FROM agent_tools WHERE tool_id = 'bash_output'", []);
    let _ = conn.execute(
        "DELETE FROM agent_tools WHERE tool_id = 'workspace_stats'",
        [],
    );
    // Retired shell names are gone. Drop leftover rows for every agent.
    strip_direct_shell_bindings(conn)?;
    seed_agents(conn)?;
    seed_default_agent_bindings(conn)?;
    seed_orchestrator_agent_bindings(conn)?;
    seed_general_agent_bindings(conn)?;
    seed_explore_agent_bindings(conn)?;
    // Seeded bindings must satisfy the role rules on their own (a subagent never
    // binds primary-only tools); this also repairs drift in older DBs.
    reconcile_role_rules(conn)?;
    mark_code_search_default_done(conn)?;

    conn.execute(
        "INSERT INTO meta (key, value) VALUES ('seed_revision', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [SEED_REVISION],
    )?;

    Ok(())
}

fn seed_agents(conn: &Connection) -> Result<()> {
    store::upsert_agent(
        conn,
        "default",
        AgentRole::Primary,
        "",
        "builtin:general",
        0.7,
        50,
        DEFAULT_DESCRIPTION,
        &["explore".into()],
    )?;
    store::upsert_agent(
        conn,
        "orchestrator",
        AgentRole::Primary,
        "",
        "builtin:orchestrator",
        0.7,
        50,
        ORCHESTRATOR_DESCRIPTION,
        &["explore".into(), "general".into()],
    )?;
    store::upsert_agent(
        conn,
        "compaction",
        AgentRole::Hidden,
        "",
        "builtin:compaction",
        0.7,
        50,
        "",
        &[],
    )?;
    store::upsert_agent(
        conn,
        "explore",
        AgentRole::Subagent,
        "",
        "builtin:explore",
        0.7,
        30,
        EXPLORE_DESCRIPTION,
        &[],
    )?;
    store::upsert_agent(
        conn,
        "general",
        AgentRole::Subagent,
        "",
        "builtin:general",
        0.7,
        50,
        GENERAL_DESCRIPTION,
        &[],
    )?;
    Ok(())
}

fn seed_default_agent_bindings(conn: &Connection) -> Result<()> {
    seed_primary_agent_bindings(conn, "default")
}

fn seed_orchestrator_agent_bindings(conn: &Connection) -> Result<()> {
    seed_primary_agent_bindings(conn, "orchestrator")
}

fn seed_primary_agent_bindings(conn: &Connection, agent_id: &str) -> Result<()> {
    bind_configurable(conn, agent_id, ToolPreset::All)?;
    for tool in core_none_tools() {
        bind_none(conn, agent_id, tool)?;
    }
    for tool in default_bound_optional_builtins() {
        if !agent_has_tool(conn, agent_id, tool)? {
            store::upsert_agent_tool(conn, agent_id, tool, &default_optional_binding(tool))?;
        }
    }
    Ok(())
}

/// Binding written when an optional builtin is bound by default (enabled, ALL preset).
pub(crate) fn default_optional_binding(tool: &str) -> crate::config::schema::AgentToolBinding {
    binding_seeded(tool, ToolPreset::All)
}

/// Meta marker for the one-time "primary agents bind code_search" backfill (WP-149).
pub const CODE_SEARCH_DEFAULT_MARKER: &str = "default_bind_code_search_v1";

fn mark_code_search_default_done(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, strftime('%s','now'))
         ON CONFLICT(key) DO NOTHING",
        [CODE_SEARCH_DEFAULT_MARKER],
    )?;
    Ok(())
}

/// One-time backfill for existing DBs: every primary agent without a
/// `code_search` row gets one (enabled). Existing rows, true or false, are never
/// touched. Runs in a transaction; the marker is written only on success.
fn ensure_code_search_default_v1(conn: &Connection) -> Result<()> {
    let done = conn
        .query_row(
            "SELECT 1 FROM meta WHERE key = ?1",
            [CODE_SEARCH_DEFAULT_MARKER],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if done {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    for tool in default_bound_optional_builtins() {
        let binding = default_optional_binding(tool);
        let policy_json = serde_json::to_string(&binding.policy)?;
        let path_mode = store::path_mode_to_str(binding.path_mode);
        let preset = binding.last_applied_preset.map(store::preset_to_str);
        tx.execute(
            "INSERT INTO agent_tools (agent_id, tool_id, enabled, policy_json, path_mode, last_applied_preset, allowed_tools_json)
             SELECT a.id, ?1, 1, ?2, ?3, ?4, NULL FROM agents a
             WHERE a.role = 'primary' AND a.id <> 'compaction'
               AND NOT EXISTS (SELECT 1 FROM agent_tools t WHERE t.agent_id = a.id AND t.tool_id = ?1)
             ON CONFLICT(agent_id, tool_id) DO NOTHING",
            params![tool, policy_json, path_mode, preset],
        )?;
    }
    mark_code_search_default_done(&tx)?;
    tx.commit()?;
    Ok(())
}

/// Same configurable set as primary, minus the primary-only tools.
/// Fixed-behavior tools a subagent may use (`knowledge`) are bound too.
fn seed_general_agent_bindings(conn: &Connection) -> Result<()> {
    bind_subagent_configurable(conn, "general", ToolPreset::All)?;
    for tool in subagent_none_tools() {
        bind_none(conn, "general", tool)?;
    }
    Ok(())
}

/// Configurable tools a subagent role may bind (no [`PRIMARY_ONLY_TOOL_IDS`]).
fn subagent_configurable_tools() -> impl Iterator<Item = &'static str> {
    core_configurable_tools()
        .iter()
        .copied()
        .filter(|tool| !PRIMARY_ONLY_TOOL_IDS.contains(tool))
}

/// Fixed-behavior tools a subagent may bind (`knowledge`).
/// Plan, todo, and the subagent series stay off this set.
fn subagent_none_tools() -> impl Iterator<Item = &'static str> {
    core_none_tools().iter().copied().filter(|tool| {
        !PRIMARY_ONLY_TOOL_IDS.contains(tool) && !SUBAGENT_SERIES_TOOL_IDS.contains(tool)
    })
}

fn bind_configurable(conn: &Connection, agent_id: &str, preset: ToolPreset) -> Result<()> {
    bind_tools(
        conn,
        agent_id,
        preset,
        core_configurable_tools().iter().copied(),
    )
}

fn bind_subagent_configurable(conn: &Connection, agent_id: &str, preset: ToolPreset) -> Result<()> {
    bind_tools(conn, agent_id, preset, subagent_configurable_tools())
}

fn bind_tools<'a>(
    conn: &Connection,
    agent_id: &str,
    preset: ToolPreset,
    tools: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    for tool in tools {
        store::upsert_agent_tool(conn, agent_id, tool, &binding_seeded(tool, preset))?;
    }
    Ok(())
}

/// Preset tools keep the requested dial. Fixed tools are enabled with allow-all and no dial.
fn binding_seeded(tool: &str, preset: ToolPreset) -> crate::config::schema::AgentToolBinding {
    use crate::config::schema::AgentToolBinding;
    use crate::permission::presets::{binding_for_tool, has_permission_preset};

    if !has_permission_preset(tool) {
        return AgentToolBinding {
            enabled: true,
            policy: crate::permission::ToolPolicy::allow_all(),
            path_mode: crate::permission::BindingPathMode::default(),
            last_applied_preset: None,
            allowed_tools: None,
        };
    }
    let (policy, path_mode) = binding_for_tool(tool, preset);
    AgentToolBinding {
        enabled: true,
        policy,
        path_mode,
        last_applied_preset: Some(preset),
        allowed_tools: None,
    }
}

fn bind_none(conn: &Connection, agent_id: &str, tool: &str) -> Result<()> {
    use crate::config::schema::AgentToolBinding;

    store::upsert_agent_tool(
        conn,
        agent_id,
        tool,
        &AgentToolBinding {
            enabled: true,
            policy: crate::permission::ToolPolicy::allow_all(),
            path_mode: crate::permission::BindingPathMode::default(),
            last_applied_preset: None,
            allowed_tools: None,
        },
    )
}

fn seed_explore_agent_bindings(conn: &Connection) -> Result<()> {
    use crate::config::schema::AgentToolBinding;
    use crate::permission::presets::binding_for_tool;

    for tool in ["read", "grep", "glob", "session_search"] {
        store::upsert_agent_tool(
            conn,
            "explore",
            tool,
            &binding_seeded(tool, ToolPreset::Safe),
        )?;
    }

    // WP-197: Code shell 三动作；仅绑 `run`（wait/kill 由 registry 注入）。
    let (run_policy, run_path) = binding_for_tool("run", ToolPreset::Safe);
    store::upsert_agent_tool(
        conn,
        "explore",
        "run",
        &AgentToolBinding {
            enabled: true,
            policy: run_policy,
            path_mode: run_path,
            last_applied_preset: Some(ToolPreset::Safe),
            allowed_tools: None,
        },
    )?;

    for tool in network_core_tools() {
        store::upsert_agent_tool(
            conn,
            "explore",
            tool,
            &binding_seeded(tool, ToolPreset::All),
        )?;
    }
    let (workspace_policy, workspace_path) =
        binding_for_tool("litecode_workspace", ToolPreset::All);
    store::upsert_agent_tool(
        conn,
        "explore",
        "litecode_workspace",
        &AgentToolBinding {
            enabled: true,
            policy: workspace_policy,
            path_mode: workspace_path,
            last_applied_preset: Some(ToolPreset::All),
            allowed_tools: None,
        },
    )?;
    for tool in subagent_none_tools() {
        bind_none(conn, "explore", tool)?;
    }
    Ok(())
}

/// Prune persisted rows that violate the current agent role rules.
///
/// The write path already normalizes profiles before saving
/// (`tool::agent_bindings::normalize_agent_profile`), but rows written by older
/// builds stay in the DB until the next save of that agent. Structural validation
/// rejects the whole settings document, so a legacy row would block serve boot.
/// Repair such rows in place instead of failing:
/// - `hidden` agents (and the reserved `compaction` id) keep no bindings;
/// - `subagent` agents keep no `plan` / `todo` / `subagent_*` bindings
///   (`knowledge` and `litecode_workspace` stay);
/// - non-primary agents keep no `allowed_subagents`.
pub fn reconcile_role_rules(conn: &Connection) -> Result<()> {
    let rows: Vec<(String, String, String)> = {
        let mut stmt = conn.prepare("SELECT id, role, allowed_subagents_json FROM agents")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut notes: Vec<String> = Vec::new();
    for (agent_id, role, allowed_json) in rows {
        let hidden = role == "hidden" || agent_id == "compaction";
        let subagent = role == "subagent";

        let mut removed = 0usize;
        if hidden {
            removed += conn.execute(
                "DELETE FROM agent_tools WHERE agent_id = ?1",
                params![agent_id],
            )?;
        } else if subagent {
            for tool_id in PRIMARY_ONLY_TOOL_IDS.iter().chain(SUBAGENT_SERIES_TOOL_IDS) {
                removed += conn.execute(
                    "DELETE FROM agent_tools WHERE agent_id = ?1 AND tool_id = ?2",
                    params![agent_id, *tool_id],
                )?;
            }
            // Imported/legacy ids outside the seeded series still count as series.
            removed += conn.execute(
                "DELETE FROM agent_tools WHERE agent_id = ?1 AND tool_id GLOB 'subagent_*'",
                params![agent_id],
            )?;
        }
        if removed > 0 {
            notes.push(format!("{agent_id}: pruned {removed} binding(s)"));
        }

        if (hidden || subagent) && allowed_json.trim() != "[]" {
            conn.execute(
                "UPDATE agents SET allowed_subagents_json = '[]' WHERE id = ?1",
                params![agent_id],
            )?;
            notes.push(format!("{agent_id}: cleared allowed_subagents"));
        }
    }

    if !notes.is_empty() {
        tracing::warn!(
            changes = %notes.join("; "),
            "global DB agent rows violated role rules; repaired in place"
        );
    }
    Ok(())
}

/// Drop leftover retired shell bindings for every agent (Direct triad + WP-196 run_command suite).
fn strip_direct_shell_bindings(conn: &Connection) -> Result<()> {
    for tool in [
        "bash",
        "wait_shell",
        "kill_shell",
        "run_command",
        "command_output",
        "command_kill",
    ] {
        conn.execute("DELETE FROM agent_tools WHERE tool_id = ?1", [tool])?;
    }
    Ok(())
}

pub fn needs_seed(conn: &Connection) -> Result<bool> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM agents", [], |row| row.get(0))?;
    Ok(count == 0)
}

/// Repair partial DB: keep existing disables; plant missing seed agents/bindings.
/// Runs last so the repaired document always satisfies the role rules
/// (see [`reconcile_role_rules`]).
pub fn ensure_core_bindings(conn: &Connection) -> Result<()> {
    let _ = conn.execute("DELETE FROM agent_tools WHERE tool_id = 'bash_output'", []);
    let _ = conn.execute(
        "DELETE FROM agent_tools WHERE tool_id = 'workspace_stats'",
        [],
    );
    // Retired shell names are gone. Drop leftover rows for every agent.
    strip_direct_shell_bindings(conn)?;
    ensure_default_core_bindings(conn)?;
    ensure_orchestrator_agent(conn)?;
    ensure_general_agent(conn)?;
    ensure_explore_shared_tools(conn)?;
    restore_default_if_mistakenly_replaced(conn)?;
    reconcile_role_rules(conn)?;
    normalize_fixed_builtin_bindings(conn)?;
    ensure_code_search_default_v1(conn)?;
    Ok(())
}

/// Tools that used to carry ALL/SAFE but are now bind-only. Rewrite stored rows
/// so an old SAFE=Ask does not linger until the next settings save.
/// Coding presets and `litecode_workspace` are left as stored.
fn normalize_fixed_builtin_bindings(conn: &Connection) -> Result<()> {
    use crate::permission::has_permission_preset;

    let policy_json = serde_json::to_string(&crate::permission::ToolPolicy::allow_all())?;
    let mut ids: Vec<String> = super::tools::core_tool_ids();
    ids.extend(
        super::tools::optional_builtin_ids()
            .iter()
            .map(|id| (*id).to_string()),
    );
    for id in ids {
        if has_permission_preset(&id) {
            continue;
        }
        conn.execute(
            "UPDATE agent_tools
             SET policy_json = ?1, path_mode = 'unrestricted', last_applied_preset = NULL
             WHERE tool_id = ?2",
            rusqlite::params![policy_json, id],
        )?;
    }
    Ok(())
}

fn ensure_default_core_bindings(conn: &Connection) -> Result<()> {
    let default_exists = conn
        .query_row("SELECT 1 FROM agents WHERE id = 'default'", [], |_| Ok(()))
        .optional()?
        .is_some();
    if !default_exists {
        return Ok(());
    }
    for tool in core_configurable_tools() {
        if !agent_has_tool(conn, "default", tool)? {
            store::upsert_agent_tool(
                conn,
                "default",
                tool,
                &binding_seeded(tool, ToolPreset::All),
            )?;
        }
    }
    for tool in core_none_tools() {
        if !agent_has_tool(conn, "default", tool)? {
            bind_none(conn, "default", tool)?;
        }
    }
    Ok(())
}

fn ensure_general_agent(conn: &Connection) -> Result<()> {
    let exists = conn
        .query_row("SELECT 1 FROM agents WHERE id = 'general'", [], |_| Ok(()))
        .optional()?
        .is_some();
    if !exists {
        store::upsert_agent(
            conn,
            "general",
            AgentRole::Subagent,
            "",
            "builtin:general",
            0.7,
            50,
            GENERAL_DESCRIPTION,
            &[],
        )?;
        seed_general_agent_bindings(conn)?;
        return Ok(());
    }
    for tool in subagent_configurable_tools() {
        if !agent_has_tool(conn, "general", tool)? {
            store::upsert_agent_tool(
                conn,
                "general",
                tool,
                &binding_seeded(tool, ToolPreset::All),
            )?;
        }
    }
    for tool in subagent_none_tools() {
        if !agent_has_tool(conn, "general", tool)? {
            bind_none(conn, "general", tool)?;
        }
    }
    Ok(())
}

/// Plant knowledge and the workspace tool on an existing explore agent.
/// A disabled row is left alone; only a missing binding is filled in.
fn ensure_explore_shared_tools(conn: &Connection) -> Result<()> {
    let exists = conn
        .query_row("SELECT 1 FROM agents WHERE id = 'explore'", [], |_| Ok(()))
        .optional()?
        .is_some();
    if !exists {
        return Ok(());
    }
    if !agent_has_tool(conn, "explore", "litecode_workspace")? {
        store::upsert_agent_tool(
            conn,
            "explore",
            "litecode_workspace",
            &binding_seeded("litecode_workspace", ToolPreset::All),
        )?;
    }
    if !agent_has_tool(conn, "explore", "run")? {
        store::upsert_agent_tool(
            conn,
            "explore",
            "run",
            &binding_seeded("run", ToolPreset::Safe),
        )?;
    }
    for tool in subagent_none_tools() {
        if !agent_has_tool(conn, "explore", tool)? {
            bind_none(conn, "explore", tool)?;
        }
    }
    Ok(())
}

fn ensure_orchestrator_agent(conn: &Connection) -> Result<()> {
    let exists = conn
        .query_row("SELECT 1 FROM agents WHERE id = 'orchestrator'", [], |_| {
            Ok(())
        })
        .optional()?
        .is_some();
    let model_ref = conn
        .query_row(
            "SELECT model_ref FROM agents WHERE id = 'default'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_default();
    if !exists {
        store::upsert_agent(
            conn,
            "orchestrator",
            AgentRole::Primary,
            &model_ref,
            "builtin:orchestrator",
            0.7,
            50,
            ORCHESTRATOR_DESCRIPTION,
            &["explore".into(), "general".into()],
        )?;
        seed_orchestrator_agent_bindings(conn)?;
        return Ok(());
    }
    for tool in core_configurable_tools() {
        if !agent_has_tool(conn, "orchestrator", tool)? {
            store::upsert_agent_tool(
                conn,
                "orchestrator",
                tool,
                &binding_seeded(tool, ToolPreset::All),
            )?;
        }
    }
    for tool in core_none_tools() {
        if !agent_has_tool(conn, "orchestrator", tool)? {
            bind_none(conn, "orchestrator", tool)?;
        }
    }
    Ok(())
}

/// Undo the mistaken rewrite of `default` into Orchestrator (seed revision 13).
fn restore_default_if_mistakenly_replaced(conn: &Connection) -> Result<()> {
    let Some((role, model_ref, system_prompt, temperature, max_steps, description, allowed_json)) =
        conn.query_row(
            "SELECT role, model_ref, system_prompt, temperature, max_steps, description, allowed_subagents_json
             FROM agents WHERE id = 'default'",
            [],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, f64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(());
    };
    if role != "primary" || system_prompt.trim() != "builtin:orchestrator" {
        return Ok(());
    }
    let mut allowed: Vec<String> = serde_json::from_str(&allowed_json).unwrap_or_default();
    allowed.retain(|id| id != "general");
    if !allowed.iter().any(|id| id == "explore") {
        allowed.push("explore".into());
    }
    let desc = if description == ORCHESTRATOR_DESCRIPTION {
        DEFAULT_DESCRIPTION
    } else {
        description.as_str()
    };
    store::upsert_agent(
        conn,
        "default",
        AgentRole::Primary,
        &model_ref,
        "builtin:general",
        temperature,
        max_steps as u32,
        desc,
        &allowed,
    )
}

fn agent_has_tool(conn: &Connection, agent_id: &str, tool: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM agent_tools WHERE agent_id = ?1 AND tool_id = ?2",
            rusqlite::params![agent_id, tool],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

#[cfg(test)]
mod tests {
    use super::super::migrate::{self};
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn seed_plants_default_orchestrator_explore_and_general() {
        let conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&conn).unwrap();
        seed(&conn).unwrap();

        let (role, prompt, allowed, desc): (String, String, String, String) = conn
            .query_row(
                "SELECT role, system_prompt, allowed_subagents_json, description FROM agents WHERE id = 'default'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(role, "primary");
        assert_eq!(prompt, "builtin:general");
        assert_eq!(desc, DEFAULT_DESCRIPTION);
        assert!(allowed.contains("explore"));
        assert!(!allowed.contains("general"));

        let (orch_role, orch_prompt, orch_allowed): (String, String, String) = conn
            .query_row(
                "SELECT role, system_prompt, allowed_subagents_json FROM agents WHERE id = 'orchestrator'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(orch_role, "primary");
        assert_eq!(orch_prompt, "builtin:orchestrator");
        assert!(orch_allowed.contains("explore"));
        assert!(orch_allowed.contains("general"));

        let (explore_role, explore_prompt): (String, String) = conn
            .query_row(
                "SELECT role, system_prompt FROM agents WHERE id = 'explore'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(explore_role, "subagent");
        assert_eq!(explore_prompt, "builtin:explore");

        let write: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_tools WHERE agent_id = 'explore' AND tool_id = 'write'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(write, 0);

        // WP-197: explore 默认无旧 shell；有 run SAFE。
        for tool in [
            "bash",
            "wait_shell",
            "kill_shell",
            "run_command",
            "command_output",
            "command_kill",
        ] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM agent_tools WHERE agent_id = 'explore' AND tool_id = ?1",
                    [tool],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 0, "explore must not bind {tool}");
        }

        let run_preset: Option<String> = conn
            .query_row(
                "SELECT last_applied_preset FROM agent_tools WHERE agent_id = 'explore' AND tool_id = 'run'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(run_preset.as_deref(), Some("SAFE"));

        let (general_role, general_prompt): (String, String) = conn
            .query_row(
                "SELECT role, system_prompt FROM agents WHERE id = 'general'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(general_role, "subagent");
        assert_eq!(general_prompt, "builtin:general");

        let general_write: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_tools WHERE agent_id = 'general' AND tool_id = 'write'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(general_write, 1);

        let general_plan: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_tools WHERE agent_id = 'general' AND tool_id = 'plan'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(general_plan, 0);

        let general_todo: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_tools WHERE agent_id = 'general' AND tool_id = 'todo'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(general_todo, 0);

        for agent in ["default", "orchestrator"] {
            let knowledge: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM agent_tools WHERE agent_id = ?1 AND tool_id = 'knowledge'",
                    [agent],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(knowledge, 1, "{agent} should bind knowledge");
        }
        for agent in ["explore", "general"] {
            let knowledge: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM agent_tools WHERE agent_id = ?1 AND tool_id = 'knowledge'",
                    [agent],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(knowledge, 1, "{agent} should bind knowledge");
            let workspace: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM agent_tools WHERE agent_id = ?1 AND tool_id = 'litecode_workspace'",
                    [agent],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(workspace, 1, "{agent} should bind litecode_workspace");
        }

        let general_launch: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_tools WHERE agent_id = 'general' AND tool_id = 'subagent_launch'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(general_launch, 0);

        // WP-197: general/default/orchestrator 默认无旧 shell；有 run。
        for agent in ["general", "default", "orchestrator"] {
            for tool in [
                "bash",
                "wait_shell",
                "kill_shell",
                "run_command",
                "command_output",
                "command_kill",
            ] {
                let n: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM agent_tools WHERE agent_id = ?1 AND tool_id = ?2",
                        rusqlite::params![agent, tool],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(n, 0, "{agent} must not bind {tool}");
            }
            let run_n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM agent_tools WHERE agent_id = ?1 AND tool_id = 'run'",
                    [agent],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(run_n, 1, "{agent} should bind run");
        }
    }

    #[test]
    fn ensure_prunes_rows_that_violate_role_rules() {
        let conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&conn).unwrap();
        seed(&conn).unwrap();

        // Rows written by older builds: subagent with plan/todo/subagent-series
        // bindings, hidden agent with bindings, subagent carrying an allowlist.
        bind_none(&conn, "explore", "plan").unwrap();
        bind_none(&conn, "explore", "todo").unwrap();
        bind_none(&conn, "explore", "subagent_wait").unwrap();
        bind_none(&conn, "compaction", "read").unwrap();
        conn.execute(
            "UPDATE agents SET allowed_subagents_json = '[\"explore\"]' WHERE id = 'explore'",
            [],
        )
        .unwrap();

        ensure_core_bindings(&conn).unwrap();

        for (agent, tool) in [
            ("explore", "plan"),
            ("explore", "todo"),
            ("explore", "subagent_wait"),
            ("compaction", "read"),
        ] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM agent_tools WHERE agent_id = ?1 AND tool_id = ?2",
                    rusqlite::params![agent, tool],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 0, "{agent}/{tool} must be pruned");
        }

        let kept: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_tools WHERE agent_id = 'explore' AND tool_id = 'read'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, 1, "legit subagent bindings survive");

        let primary_plan: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_tools WHERE agent_id = 'default' AND tool_id = 'plan'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(primary_plan, 1, "primary keeps plan/todo");

        let allowed: String = conn
            .query_row(
                "SELECT allowed_subagents_json FROM agents WHERE id = 'explore'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(allowed, "[]");

        let default_allowed: String = conn
            .query_row(
                "SELECT allowed_subagents_json FROM agents WHERE id = 'default'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(default_allowed.contains("explore"));

        let loaded = store::load(&conn).unwrap();
        crate::config::ConfigManager::validate(&loaded).unwrap();
    }

    // ---- WP-149: primary agents bind code_search by default ----

    fn cs_enabled(conn: &Connection, agent: &str) -> Option<i64> {
        conn.query_row(
            "SELECT enabled FROM agent_tools WHERE agent_id = ?1 AND tool_id = 'code_search'",
            [agent],
            |r| r.get(0),
        )
        .optional()
        .unwrap()
    }

    fn cs_marker(conn: &Connection) -> Option<String> {
        conn.query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [CODE_SEARCH_DEFAULT_MARKER],
            |r| r.get(0),
        )
        .optional()
        .unwrap()
    }

    fn cs_rows(conn: &Connection) -> Vec<(String, i64, String)> {
        let mut stmt = conn
            .prepare(
                "SELECT agent_id, enabled, policy_json FROM agent_tools
                 WHERE tool_id = 'code_search' ORDER BY agent_id",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    fn fresh_seeded() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&conn).unwrap();
        seed(&conn).unwrap();
        conn
    }

    /// Simulate a DB created before WP-149: no marker, no code_search rows.
    fn pre_wp149_db() -> Connection {
        let conn = fresh_seeded();
        conn.execute("DELETE FROM agent_tools WHERE tool_id = 'code_search'", [])
            .unwrap();
        conn.execute(
            "DELETE FROM meta WHERE key = ?1",
            [CODE_SEARCH_DEFAULT_MARKER],
        )
        .unwrap();
        conn
    }

    #[test]
    fn migration_fills_missing_primary_code_search_only() {
        let conn = pre_wp149_db();
        store::upsert_agent(
            &conn,
            "my_primary",
            AgentRole::Primary,
            "",
            "",
            0.7,
            50,
            "",
            &[],
        )
        .unwrap();
        store::upsert_agent(
            &conn,
            "my_sub",
            AgentRole::Subagent,
            "",
            "",
            0.7,
            50,
            "",
            &[],
        )
        .unwrap();
        ensure_core_bindings(&conn).unwrap();
        for agent in ["default", "orchestrator", "my_primary"] {
            assert_eq!(cs_enabled(&conn, agent), Some(1), "{agent}");
        }
        for agent in ["explore", "general", "compaction", "my_sub"] {
            assert_eq!(cs_enabled(&conn, agent), None, "{agent}");
        }
        assert!(cs_marker(&conn).is_some());
    }

    #[test]
    fn migration_keeps_existing_false_row() {
        let conn = pre_wp149_db();
        let mut off = default_optional_binding("code_search");
        off.enabled = false;
        store::upsert_agent_tool(&conn, "default", "code_search", &off).unwrap();
        let before = cs_rows(&conn);
        ensure_core_bindings(&conn).unwrap();
        assert_eq!(cs_enabled(&conn, "default"), Some(0));
        let after = cs_rows(&conn);
        let def = |rows: &[(String, i64, String)]| {
            rows.iter().find(|r| r.0 == "default").cloned().unwrap()
        };
        assert_eq!(def(&before), def(&after));
        assert_eq!(cs_enabled(&conn, "orchestrator"), Some(1));
    }

    #[test]
    fn migration_runs_once_on_repeated_startup() {
        let conn = pre_wp149_db();
        ensure_core_bindings(&conn).unwrap();
        let marker = cs_marker(&conn).expect("marker set");
        let rows = cs_rows(&conn);
        // Deleting a row after the one-time run must not bring it back.
        conn.execute(
            "DELETE FROM agent_tools WHERE agent_id = 'orchestrator' AND tool_id = 'code_search'",
            [],
        )
        .unwrap();
        let rows_after_delete = cs_rows(&conn);
        assert_eq!(rows_after_delete.len(), rows.len() - 1);
        for _ in 0..3 {
            ensure_core_bindings(&conn).unwrap();
            assert_eq!(cs_marker(&conn).as_deref(), Some(marker.as_str()));
            assert_eq!(cs_rows(&conn), rows_after_delete);
        }
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM meta WHERE key = ?1",
                [CODE_SEARCH_DEFAULT_MARKER],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }
}
