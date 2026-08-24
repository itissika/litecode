use rusqlite::{Connection, OptionalExtension};

use crate::config::schema::{AgentRole, ToolPreset};
use crate::types::Result;

use super::store;
use super::tools::{core_configurable_tools, core_none_tools};

pub const SEED_REVISION: &str = "10";

pub fn seed(conn: &Connection) -> Result<()> {
    let _ = conn.execute("DELETE FROM agent_tools WHERE tool_id = 'bash_output'", []);
    seed_agents(conn)?;
    seed_default_agent_bindings(conn)?;

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
        "General-purpose coding assistant",
        &[],
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
    Ok(())
}

fn seed_default_agent_bindings(conn: &Connection) -> Result<()> {
    use crate::config::schema::AgentToolBinding;
    use crate::permission::presets::binding_for_tool;

    for tool in core_configurable_tools() {
        let (policy, path_mode) = binding_for_tool(tool, ToolPreset::All);
        let binding = AgentToolBinding {
            enabled: true,
            policy,
            path_mode,
            last_applied_preset: Some(ToolPreset::All),
            allowed_tools: None,
        };
        store::upsert_agent_tool(conn, "default", tool, &binding)?;
    }
    for tool in core_none_tools() {
        let binding = AgentToolBinding {
            enabled: true,
            policy: crate::permission::ToolPolicy::allow_all(),
            path_mode: crate::permission::BindingPathMode::default(),
            last_applied_preset: None,
            allowed_tools: None,
        };
        store::upsert_agent_tool(conn, "default", tool, &binding)?;
    }
    Ok(())
}

pub fn needs_seed(conn: &Connection) -> Result<bool> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM agents", [], |row| row.get(0))?;
    Ok(count == 0)
}

/// Repair partial DB: keep default-agent core bindings; do not overwrite user disables.
pub fn ensure_core_bindings(conn: &Connection) -> Result<()> {
    let _ = conn.execute("DELETE FROM agent_tools WHERE tool_id = 'bash_output'", []);
    ensure_default_core_bindings(conn)
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
        let exists = conn
            .query_row(
                "SELECT 1 FROM agent_tools WHERE agent_id = 'default' AND tool_id = ?1",
                [*tool],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
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
        let exists = conn
            .query_row(
                "SELECT 1 FROM agent_tools WHERE agent_id = 'default' AND tool_id = ?1",
                [*tool],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            let binding = crate::config::schema::AgentToolBinding {
                enabled: true,
                policy: crate::permission::ToolPolicy::allow_all(),
                path_mode: crate::permission::BindingPathMode::default(),
                last_applied_preset: None,
                allowed_tools: None,
            };
            store::upsert_agent_tool(conn, "default", tool, &binding)?;
        }
    }
    Ok(())
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

        let providers: i64 = conn
            .query_row("SELECT COUNT(*) FROM providers", [], |r| r.get(0))
            .unwrap();
        let models: i64 = conn
            .query_row("SELECT COUNT(*) FROM models", [], |r| r.get(0))
            .unwrap();
        assert_eq!(providers, 0);
        assert_eq!(models, 0);

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
}
