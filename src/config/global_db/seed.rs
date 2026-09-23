use rusqlite::{Connection, OptionalExtension, params};

use crate::config::schema::{AgentRole, PLAN_TODO_TOOL_IDS, SUBAGENT_SERIES_TOOL_IDS, ToolPreset};
use crate::types::Result;

use super::builtin_prompts::{
    DEFAULT_DESCRIPTION, EXPLORE_DESCRIPTION, GENERAL_DESCRIPTION, ORCHESTRATOR_DESCRIPTION,
};
use super::store;
use super::tools::{core_configurable_tools, core_none_tools, network_core_tools};

pub const SEED_REVISION: &str = "13";

pub fn seed(conn: &Connection) -> Result<()> {
    let _ = conn.execute("DELETE FROM agent_tools WHERE tool_id = 'bash_output'", []);
    seed_agents(conn)?;
    seed_default_agent_bindings(conn)?;
    seed_orchestrator_agent_bindings(conn)?;
    seed_general_agent_bindings(conn)?;
    seed_explore_agent_bindings(conn)?;

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
    Ok(())
}

/// Same configurable set as primary. No plan/todo (subagent tool-set gate).
fn seed_general_agent_bindings(conn: &Connection) -> Result<()> {
    bind_configurable(conn, "general", ToolPreset::All)
}

fn bind_configurable(conn: &Connection, agent_id: &str, preset: ToolPreset) -> Result<()> {
    use crate::config::schema::AgentToolBinding;
    use crate::permission::presets::binding_for_tool;

    for tool in core_configurable_tools() {
        let (policy, path_mode) = binding_for_tool(tool, preset);
        let binding = AgentToolBinding {
            enabled: true,
            policy,
            path_mode,
            last_applied_preset: Some(preset),
            allowed_tools: None,
        };
        store::upsert_agent_tool(conn, agent_id, tool, &binding)?;
    }
    Ok(())
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
        let (policy, path_mode) = binding_for_tool(tool, ToolPreset::Safe);
        let binding = AgentToolBinding {
            enabled: true,
            policy,
            path_mode,
            last_applied_preset: Some(ToolPreset::Safe),
            allowed_tools: None,
        };
        store::upsert_agent_tool(conn, "explore", tool, &binding)?;
    }

    let (bash_policy, bash_path) = binding_for_tool("bash", ToolPreset::Safe);
    store::upsert_agent_tool(
        conn,
        "explore",
        "bash",
        &AgentToolBinding {
            enabled: true,
            policy: bash_policy,
            path_mode: bash_path,
            last_applied_preset: Some(ToolPreset::Safe),
            allowed_tools: None,
        },
    )?;
    for tool in ["wait_shell", "kill_shell"] {
        store::upsert_agent_tool(
            conn,
            "explore",
            tool,
            &AgentToolBinding {
                enabled: true,
                policy: crate::permission::ToolPolicy::allow_all(),
                path_mode: crate::permission::BindingPathMode::default(),
                last_applied_preset: None,
                allowed_tools: None,
            },
        )?;
    }

    for tool in network_core_tools() {
        let (policy, path_mode) = binding_for_tool(tool, ToolPreset::All);
        store::upsert_agent_tool(
            conn,
            "explore",
            tool,
            &AgentToolBinding {
                enabled: true,
                policy,
                path_mode,
                last_applied_preset: Some(ToolPreset::All),
                allowed_tools: None,
            },
        )?;
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
/// - `subagent` agents keep no `plan` / `todo` / `subagent_*` bindings;
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
            for tool_id in PLAN_TODO_TOOL_IDS.iter().chain(SUBAGENT_SERIES_TOOL_IDS) {
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

pub fn needs_seed(conn: &Connection) -> Result<bool> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM agents", [], |row| row.get(0))?;
    Ok(count == 0)
}

/// Repair partial DB: keep existing disables; plant missing seed agents/bindings.
/// Runs last so the repaired document always satisfies the role rules
/// (see [`reconcile_role_rules`]).
pub fn ensure_core_bindings(conn: &Connection) -> Result<()> {
    let _ = conn.execute("DELETE FROM agent_tools WHERE tool_id = 'bash_output'", []);
    ensure_default_core_bindings(conn)?;
    ensure_orchestrator_agent(conn)?;
    ensure_general_agent(conn)?;
    restore_default_if_mistakenly_replaced(conn)?;
    reconcile_role_rules(conn)?;
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
            let (policy, path_mode) =
                crate::permission::presets::binding_for_tool(tool, ToolPreset::All);
            let binding = crate::config::schema::AgentToolBinding {
                enabled: true,
                policy,
                path_mode,
                last_applied_preset: Some(ToolPreset::All),
                allowed_tools: None,
            };
            store::upsert_agent_tool(conn, "default", tool, &binding)?;
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
    for tool in core_configurable_tools() {
        if !agent_has_tool(conn, "general", tool)? {
            let (policy, path_mode) =
                crate::permission::presets::binding_for_tool(tool, ToolPreset::All);
            let binding = crate::config::schema::AgentToolBinding {
                enabled: true,
                policy,
                path_mode,
                last_applied_preset: Some(ToolPreset::All),
                allowed_tools: None,
            };
            store::upsert_agent_tool(conn, "general", tool, &binding)?;
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
            let (policy, path_mode) =
                crate::permission::presets::binding_for_tool(tool, ToolPreset::All);
            let binding = crate::config::schema::AgentToolBinding {
                enabled: true,
                policy,
                path_mode,
                last_applied_preset: Some(ToolPreset::All),
                allowed_tools: None,
            };
            store::upsert_agent_tool(conn, "orchestrator", tool, &binding)?;
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
    use super::super::migrate::{self, CURRENT_USER_VERSION};
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn seed_has_agents_without_fake_llm_rows() {
        let conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&conn).unwrap();
        seed(&conn).unwrap();

        // A fresh install has no legacy LLM tables at all: provider/model facts
        // live in provider-catalog.toml, credentials in provider_credentials.
        for legacy in ["providers", "models"] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [legacy],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "fresh schema must not create the '{legacy}' table");
        }
        let credentials: i64 = conn
            .query_row("SELECT COUNT(*) FROM provider_credentials", [], |r| r.get(0))
            .unwrap();
        assert_eq!(credentials, 0);

        let default_ref: String = conn
            .query_row(
                "SELECT model_ref FROM agents WHERE id = 'default'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(default_ref.is_empty());

        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_USER_VERSION);
    }

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

        let bash_preset: Option<String> = conn
            .query_row(
                "SELECT last_applied_preset FROM agent_tools WHERE agent_id = 'explore' AND tool_id = 'bash'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(bash_preset.as_deref(), Some("SAFE"));

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

        let general_launch: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_tools WHERE agent_id = 'general' AND tool_id = 'subagent_launch'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(general_launch, 0);

        let general_bash: Option<String> = conn
            .query_row(
                "SELECT last_applied_preset FROM agent_tools WHERE agent_id = 'general' AND tool_id = 'bash'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(general_bash.as_deref(), Some("ALL"));
    }

    #[test]
    fn ensure_plants_missing_agents_without_replacing_default() {
        let conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&conn).unwrap();
        store::upsert_agent(
            &conn,
            "default",
            AgentRole::Primary,
            "",
            "builtin:general",
            0.7,
            50,
            DEFAULT_DESCRIPTION,
            &["explore".into()],
        )
        .unwrap();
        bind_none(&conn, "default", "read").unwrap();

        ensure_core_bindings(&conn).unwrap();

        let (prompt, allowed, desc): (String, String, String) = conn
            .query_row(
                "SELECT system_prompt, allowed_subagents_json, description FROM agents WHERE id = 'default'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(prompt, "builtin:general");
        assert!(allowed.contains("explore"));
        assert!(!allowed.contains("general"));
        assert_eq!(desc, DEFAULT_DESCRIPTION);

        let general: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agents WHERE id = 'general'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(general, 1);
        let orch: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agents WHERE id = 'orchestrator'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(orch, 1);
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

    #[test]
    fn ensure_restores_default_if_rewritten_to_orchestrator() {
        let conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&conn).unwrap();
        store::upsert_agent(
            &conn,
            "default",
            AgentRole::Primary,
            "",
            "builtin:orchestrator",
            0.7,
            50,
            ORCHESTRATOR_DESCRIPTION,
            &["explore".into(), "general".into()],
        )
        .unwrap();

        ensure_core_bindings(&conn).unwrap();

        let (prompt, allowed, desc): (String, String, String) = conn
            .query_row(
                "SELECT system_prompt, allowed_subagents_json, description FROM agents WHERE id = 'default'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(prompt, "builtin:general");
        assert_eq!(desc, DEFAULT_DESCRIPTION);
        assert!(allowed.contains("explore"));
        assert!(!allowed.contains("general"));

        let orch_prompt: String = conn
            .query_row(
                "SELECT system_prompt FROM agents WHERE id = 'orchestrator'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(orch_prompt, "builtin:orchestrator");
    }
}
