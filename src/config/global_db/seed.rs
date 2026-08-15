use rusqlite::{Connection, OptionalExtension};

use crate::config::schema::{AgentRole, InitScope, ToolPreset, ToolTier};
use crate::types::Result;

use super::store;
use super::tools::{core_configurable_tools, core_none_tools, optional_builtin_ids};

pub const SEED_REVISION: &str = "9";

/// Core built-in tools: always ready + catalog_enabled (CONFIG §2.4).
const CORE_CATALOG: &[(&str, ToolTier)] = &[
    ("read", ToolTier::Core),
    ("write", ToolTier::Core),
    ("edit", ToolTier::Core),
    ("grep", ToolTier::Core),
    ("glob", ToolTier::Core),
    ("bash", ToolTier::Core),
    ("kill_shell", ToolTier::Core),
    ("wait_shell", ToolTier::Core),
    ("plan", ToolTier::Core),
    ("todo", ToolTier::Core),
    ("subagent_launch", ToolTier::Core),
    ("session_search", ToolTier::Core),
];

pub fn seed(conn: &Connection) -> Result<()> {
    // No fake providers/models — LLM rows are user-configured via Settings.
    seed_tool_catalog(conn)?;
    // Removed in seed_revision 8: background bash output is file-backed via `read`.
    let _ = conn.execute("DELETE FROM agent_tools WHERE tool_id = 'bash_output'", []);
    let _ = conn.execute("DELETE FROM tool_catalog WHERE id = 'bash_output'", []);
    seed_agents(conn)?;
    seed_default_agent_bindings(conn)?;

    conn.execute(
        "INSERT INTO meta (key, value) VALUES ('seed_revision', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [SEED_REVISION],
    )?;

    Ok(())
}

const OPTIONAL_CATALOG: &[(&str, InitScope)] = &[
    ("webfetch", InitScope::Global),
    ("websearch", InitScope::Global),
    ("code_search", InitScope::Workspace),
    ("lsp", InitScope::Workspace),
];

fn seed_tool_catalog(conn: &Connection) -> Result<()> {
    for (id, tier) in CORE_CATALOG {
        store::upsert_catalog_entry(conn, id, *tier, InitScope::None, true)?;
    }
    for (id, init_scope) in OPTIONAL_CATALOG {
        store::upsert_catalog_entry(conn, id, ToolTier::Optional, *init_scope, false)?;
    }
    Ok(())
}

fn seed_agents(conn: &Connection) -> Result<()> {
    // model_ref empty until a structurally-ready model exists.
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
        let (policy, path_mode) = binding_for_tool(*tool, ToolPreset::All);
        let binding = AgentToolBinding {
            enabled: true,
            policy,
            path_mode,
            last_applied_preset: Some(ToolPreset::All),
        };
        store::upsert_agent_tool(conn, "default", tool, &binding)?;
    }
    for tool in core_none_tools() {
        let binding = AgentToolBinding {
            enabled: true,
            policy: crate::permission::ToolPolicy::allow_all(),
            path_mode: crate::permission::BindingPathMode::default(),
            last_applied_preset: None,
        };
        store::upsert_agent_tool(conn, "default", tool, &binding)?;
    }
    Ok(())
}

fn ensure_core_catalog_entries(conn: &Connection) -> Result<()> {
    for (id, tier) in CORE_CATALOG {
        let exists = conn
            .query_row(
                "SELECT 1 FROM tool_catalog WHERE id = ?1",
                [*id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            store::upsert_catalog_entry(conn, id, *tier, InitScope::None, true)?;
        }
    }
    Ok(())
}

pub fn needs_seed(conn: &Connection) -> Result<bool> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM agents", [], |row| row.get(0))?;
    Ok(count == 0)
}

/// Repair partial DB state: agents exist but catalog was wiped (e.g. empty PUT).
pub fn ensure_core_catalog(conn: &Connection) -> Result<()> {
    // seed_revision 8: drop obsolete bash_output (file-backed bg output via read).
    let _ = conn.execute("DELETE FROM agent_tools WHERE tool_id = 'bash_output'", []);
    let _ = conn.execute("DELETE FROM tool_catalog WHERE id = 'bash_output'", []);

    let count: i64 = conn.query_row("SELECT COUNT(*) FROM tool_catalog", [], |row| row.get(0))?;
    if count == 0 {
        seed_tool_catalog(conn)?;
    } else {
        ensure_core_catalog_entries(conn)?;
    }
    ensure_default_core_bindings(conn)?;
    Ok(())
}

/// Insert missing default-agent bindings for core tools (does not overwrite user disables).
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
                crate::permission::presets::binding_for_tool(*tool, ToolPreset::All);
            let binding = crate::config::schema::AgentToolBinding {
                enabled: true,
                policy,
                path_mode,
                last_applied_preset: Some(ToolPreset::All),
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
            };
            store::upsert_agent_tool(conn, "default", tool, &binding)?;
        }
    }
    Ok(())
}

/// Add missing optional builtin catalog rows on a current-schema DB (not schema migration).
pub fn ensure_optional_catalog(conn: &Connection) -> Result<()> {
    for id in optional_builtin_ids() {
        let exists = conn
            .query_row(
                "SELECT 1 FROM tool_catalog WHERE id = ?1",
                [*id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if exists {
            continue;
        }
        let init_scope = match *id {
            "webfetch" | "websearch" => InitScope::Global,
            "code_search" | "lsp" => InitScope::Workspace,
            _ => InitScope::None,
        };
        store::upsert_catalog_entry(conn, id, ToolTier::Optional, init_scope, false)?;
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
