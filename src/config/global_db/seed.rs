//! Global DB seeding.
//!
//! `builtin_agents.rs` is the single source for built-in identity, prompts,
//! descriptions and bindings. The DB only owns each agent's `model_ref` and the
//! user's non-core `agent_tools` overrides.

use rusqlite::{Connection, OptionalExtension, params_from_iter};

use crate::types::Result;

use super::builtin_agents::builtin_agents;
use super::store;

/// First boot: plant the six built-in agent rows.
///
/// Core and default-optional bindings come from the spec at read time, so
/// `agent_tools` only ever holds user overrides.
pub fn seed(conn: &Connection) -> Result<()> {
    write_builtin_agent_rows(conn, false)
}

/// Every later boot: drop agent data this build does not ship and keep the six
/// built-in rows (and their `model_ref`) in place.
pub fn ensure_core_bindings(conn: &Connection) -> Result<()> {
    prune_agents_outside_the_spec(conn)?;
    write_builtin_agent_rows(conn, true)
}

/// A custom agent row and its bindings are deleted outright: spec ids are the
/// only agents the runtime can address.
fn prune_agents_outside_the_spec(conn: &Connection) -> Result<()> {
    let ids: Vec<&str> = builtin_agents().iter().map(|spec| spec.id).collect();
    let placeholders = vec!["?"; ids.len()].join(", ");
    let removed_bindings = conn.execute(
        &format!("DELETE FROM agent_tools WHERE agent_id NOT IN ({placeholders})"),
        params_from_iter(ids.iter().copied()),
    )?;
    let removed_agents = conn.execute(
        &format!("DELETE FROM agents WHERE id NOT IN ({placeholders})"),
        params_from_iter(ids.iter().copied()),
    )?;
    if removed_agents > 0 || removed_bindings > 0 {
        tracing::info!(
            agents = removed_agents,
            bindings = removed_bindings,
            "deleted agent data outside the built-in spec set"
        );
    }
    Ok(())
}

/// Write the six agent rows. Every column except `model_ref` mirrors the spec
/// so the DB stays readable; `load_agents` never reads them back.
fn write_builtin_agent_rows(conn: &Connection, preserve_models: bool) -> Result<()> {
    for spec in builtin_agents() {
        let model_ref = if preserve_models {
            conn.query_row(
                "SELECT model_ref FROM agents WHERE id = ?1",
                [spec.id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_default()
        } else {
            String::new()
        };
        let allowed: Vec<String> = spec
            .allowed_subagents
            .iter()
            .map(|id| (*id).to_string())
            .collect();
        store::upsert_agent(
            conn,
            spec.id,
            spec.role,
            &model_ref,
            &spec.prompt_marker(),
            spec.description,
            &allowed,
        )?;
    }
    Ok(())
}

/// Whether this DB has never been seeded (empty `agents` table).
pub fn needs_seed(conn: &Connection) -> Result<bool> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM agents", [], |row| row.get(0))?;
    Ok(count == 0)
}

#[cfg(test)]
mod tests {
    use super::super::migrate::{self};
    use super::*;
    use crate::config::schema::{AgentRole, ToolPreset};
    use rusqlite::Connection;

    fn fresh_seeded() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&conn).unwrap();
        seed(&conn).unwrap();
        conn
    }

    fn agent_ids(conn: &Connection) -> Vec<String> {
        let mut stmt = conn.prepare("SELECT id FROM agents ORDER BY id").unwrap();
        stmt.query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    fn count_bindings_for(conn: &Connection, agent_id: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM agent_tools WHERE agent_id = ?1",
            [agent_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    /// The DB stores only what the user owns: six agent rows with an empty
    /// `model_ref`, and no binding rows at all.
    #[test]
    fn seed_plants_the_six_builtin_agents_without_binding_rows() {
        let conn = fresh_seeded();

        assert!(!needs_seed(&conn).unwrap());
        assert_eq!(
            agent_ids(&conn),
            vec![
                "default",
                "explore",
                "general",
                "orchestrator",
                "plan",
                "verification"
            ]
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM agent_tools", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0,
            "core and default-optional bindings must not be planted as rows"
        );

        let (role, prompt, description, model_ref): (String, String, String, String) = conn
            .query_row(
                "SELECT role, system_prompt, description, model_ref FROM agents WHERE id = 'default'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(role, "primary");
        assert_eq!(prompt, "builtin:default");
        assert_eq!(description, crate::config::global_db::DEFAULT_DESCRIPTION);
        assert_eq!(model_ref, "", "first boot has no model yet");
    }

    /// A second boot deletes every agent outside the spec (and its bindings),
    /// rewrites the identity columns to spec values, keeps `model_ref`, and
    /// leaves the user's non-core overrides alone.
    #[test]
    fn ensure_core_bindings_deletes_agents_outside_the_spec() {
        let conn = fresh_seeded();
        conn.execute(
            "UPDATE agents SET model_ref = 'prov/kept' WHERE id = 'default'",
            [],
        )
        .unwrap();
        // A custom agent from an older build, even with a retired role.
        conn.execute(
            "INSERT INTO agents (id, role, model_ref, system_prompt, description, allowed_subagents_json)
             VALUES ('my_custom', 'hidden', 'prov/x', 'HACKED', 'x', '[]')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO agent_tools (agent_id, tool_id, enabled, preset, allowed_tools_json)
             VALUES ('my_custom', 'read', 1, NULL, NULL)",
            [],
        )
        .unwrap();
        // Tampered identity columns on a spec agent.
        conn.execute(
            "UPDATE agents SET role = 'subagent', system_prompt = 'HACKED', allowed_subagents_json = '[]'
             WHERE id = 'explore'",
            [],
        )
        .unwrap();
        // A user's explicit non-core override must survive the repair.
        conn.execute(
            "INSERT INTO agent_tools (agent_id, tool_id, enabled, preset, allowed_tools_json)
             VALUES ('default', 'code_search', 0, NULL, NULL)",
            [],
        )
        .unwrap();

        ensure_core_bindings(&conn).unwrap();

        assert_eq!(
            agent_ids(&conn),
            vec![
                "default",
                "explore",
                "general",
                "orchestrator",
                "plan",
                "verification"
            ],
            "agents outside the spec must be deleted, not just ignored"
        );
        assert_eq!(
            count_bindings_for(&conn, "my_custom"),
            0,
            "the deleted agent's bindings must go with it"
        );

        let (model_ref, prompt, role): (String, String, String) = conn
            .query_row(
                "SELECT model_ref, system_prompt, role FROM agents WHERE id = 'default'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(model_ref, "prov/kept", "the user's model choice survives");
        assert_eq!(prompt, "builtin:default");
        assert_eq!(role, "primary");

        let explore_role: String = conn
            .query_row("SELECT role FROM agents WHERE id = 'explore'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(explore_role, "subagent", "identity columns are rewritten");

        let code_search: i64 = conn
            .query_row(
                "SELECT enabled FROM agent_tools WHERE agent_id = 'default' AND tool_id = 'code_search'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(code_search, 0, "non-core user intent is untouched");

        // Repeated boots are idempotent and never resurrect deleted rows.
        for _ in 0..3 {
            ensure_core_bindings(&conn).unwrap();
        }
        assert_eq!(agent_ids(&conn).len(), 6);
        let code_search: i64 = conn
            .query_row(
                "SELECT enabled FROM agent_tools WHERE agent_id = 'default' AND tool_id = 'code_search'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(code_search, 0);
    }

    /// A DB whose `agents` table is empty is the only "needs seed" state, and
    /// `ensure_core_bindings` alone can rebuild the closed set.
    #[test]
    fn ensure_core_bindings_rebuilds_all_six_rows() {
        let conn = fresh_seeded();
        conn.execute("DELETE FROM agents", []).unwrap();

        ensure_core_bindings(&conn).unwrap();

        assert_eq!(agent_ids(&conn).len(), 6);
        for spec in builtin_agents() {
            let (prompt, role): (String, String) = conn
                .query_row(
                    "SELECT system_prompt, role FROM agents WHERE id = ?1",
                    [spec.id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(prompt, spec.prompt_marker(), "{}", spec.id);
            assert_eq!(
                role,
                match spec.role {
                    AgentRole::Primary => "primary",
                    AgentRole::Subagent => "subagent",
                },
                "{}",
                spec.id
            );
        }
    }

    /// Guard: with an empty `agent_tools` table the loaded profiles are exactly
    /// the spec's core plus default-optional bindings.
    #[test]
    fn spec_core_and_optional_cover_every_locked_binding() {
        let conn = fresh_seeded();
        let settings = super::super::store::load(&conn).unwrap();
        for spec in builtin_agents() {
            let profile = settings
                .agents
                .get(spec.id)
                .unwrap_or_else(|| panic!("{} missing", spec.id));
            let expected: usize = spec.core.len() + spec.default_optional.len();
            assert_eq!(
                profile.tools.len(),
                expected,
                "{}: empty DB must yield exactly core + default optional",
                spec.id
            );
            for (tool_id, binding) in spec.core_bindings() {
                assert_eq!(profile.tools[tool_id], binding, "{} {tool_id}", spec.id);
            }
            for (tool_id, binding) in spec.default_optional_bindings() {
                assert_eq!(profile.tools[tool_id], binding, "{} {tool_id}", spec.id);
            }
        }
    }

    // ---- the spec table must reproduce the frozen seed baseline ----
    //
    // Hand-transcribed from the locked seed output: this is the guard that the
    // table-driven seed did not move a binding.

    struct BaselineAgent {
        id: &'static str,
        role: AgentRole,
        marker: &'static str,
        description: &'static str,
        allowed: &'static [&'static str],
        tools: &'static [(&'static str, Option<ToolPreset>)],
    }

    const BASELINE: &[BaselineAgent] = &[
        BaselineAgent {
            id: "default",
            role: AgentRole::Primary,
            marker: "builtin:default",
            description: "General-purpose software engineering agent. Investigates, implements, verifies, and delivers end-to-end changes.",
            allowed: &["explore"],
            tools: &[
                ("ask_user", None),
                ("code_search", None),
                ("edit", Some(ToolPreset::All)),
                ("exec", Some(ToolPreset::All)),
                ("glob", Some(ToolPreset::All)),
                ("grep", Some(ToolPreset::All)),
                ("knowledge", None),
                ("litecode_workspace", Some(ToolPreset::All)),
                ("memory", None),
                ("plan", None),
                ("read", Some(ToolPreset::All)),
                ("session_search", None),
                ("subagent_tools", None),
                ("todo", None),
                ("webfetch", None),
                ("websearch", None),
                ("write", Some(ToolPreset::All)),
            ],
        },
        BaselineAgent {
            id: "plan",
            role: AgentRole::Primary,
            marker: "builtin:plan",
            description: "Read-only software architect and planner. Explores the codebase, clarifies ambiguities, and produces a concrete implementation plan for user approval.",
            allowed: &["explore"],
            tools: &[
                ("ask_user", None),
                ("code_search", None),
                ("edit", Some(ToolPreset::All)),
                ("exec", Some(ToolPreset::Safe)),
                ("glob", Some(ToolPreset::All)),
                ("grep", Some(ToolPreset::All)),
                ("knowledge", None),
                ("litecode_workspace", Some(ToolPreset::Safe)),
                ("memory", None),
                ("plan", None),
                ("read", Some(ToolPreset::All)),
                ("session_search", None),
                ("subagent_tools", None),
                ("webfetch", None),
                ("websearch", None),
                ("write", Some(ToolPreset::All)),
            ],
        },
        BaselineAgent {
            id: "orchestrator",
            role: AgentRole::Primary,
            marker: "builtin:orchestrator",
            description: "Team coordinator and goal owner. Plans work, delegates bounded tasks to explore/general/verification subagents, enforces independent verification, and owns the final outcome.",
            allowed: &["explore", "general", "verification"],
            tools: &[
                ("ask_user", None),
                ("code_search", None),
                ("edit", Some(ToolPreset::All)),
                ("exec", Some(ToolPreset::All)),
                ("glob", Some(ToolPreset::All)),
                ("grep", Some(ToolPreset::All)),
                ("knowledge", None),
                ("litecode_workspace", Some(ToolPreset::All)),
                ("memory", None),
                ("plan", None),
                ("read", Some(ToolPreset::All)),
                ("session_search", None),
                ("subagent_tools", None),
                ("todo", None),
                ("webfetch", None),
                ("websearch", None),
                ("write", Some(ToolPreset::All)),
            ],
        },
        BaselineAgent {
            id: "explore",
            role: AgentRole::Subagent,
            marker: "builtin:explore",
            description: "Fast read-only codebase and source investigator. Searches files, traces implementations, and returns concise, evidence-backed conclusions.",
            allowed: &[],
            tools: &[
                ("code_search", None),
                ("exec", Some(ToolPreset::Safe)),
                ("glob", Some(ToolPreset::Safe)),
                ("grep", Some(ToolPreset::Safe)),
                ("knowledge", None),
                ("litecode_workspace", Some(ToolPreset::Safe)),
                ("read", Some(ToolPreset::Safe)),
                ("session_search", None),
                ("webfetch", None),
                ("websearch", None),
            ],
        },
        BaselineAgent {
            id: "general",
            role: AgentRole::Subagent,
            marker: "builtin:general",
            description: "Bounded general-purpose implementer. Executes assigned code changes, fixes, and tests within its owned scope and returns a concise evidence-backed report.",
            allowed: &[],
            tools: &[
                ("code_search", None),
                ("edit", Some(ToolPreset::All)),
                ("exec", Some(ToolPreset::All)),
                ("glob", Some(ToolPreset::All)),
                ("grep", Some(ToolPreset::All)),
                ("knowledge", None),
                ("litecode_workspace", Some(ToolPreset::All)),
                ("read", Some(ToolPreset::All)),
                ("session_search", None),
                ("webfetch", None),
                ("websearch", None),
                ("write", Some(ToolPreset::All)),
            ],
        },
        BaselineAgent {
            id: "verification",
            role: AgentRole::Subagent,
            marker: "builtin:verification",
            description: "Independent adversarial verifier. Does not modify project files; runs builds, tests, linters, and adversarial probes to issue an evidence-backed PASS, FAIL, or PARTIAL verdict.",
            allowed: &[],
            tools: &[
                ("exec", Some(ToolPreset::All)),
                ("glob", Some(ToolPreset::All)),
                ("grep", Some(ToolPreset::All)),
                ("knowledge", None),
                ("read", Some(ToolPreset::All)),
                ("webfetch", None),
                ("websearch", None),
            ],
        },
    ];

    #[test]
    fn seed_reproduces_the_baseline_builtin_bindings() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("litecode.db");

        // First load migrates and seeds a fresh DB; the second load runs the
        // repair path (`ensure_core_bindings`). A real install sees both.
        crate::config::ConfigManager::load_global_from(&db).expect("fresh DB seeds");
        let settings = crate::config::ConfigManager::load_global_from(&db).expect("repair loads");

        assert_eq!(settings.agents.len(), BASELINE.len());
        for baseline in BASELINE {
            let profile = settings
                .agents
                .get(baseline.id)
                .unwrap_or_else(|| panic!("{} missing from seeded agents", baseline.id));
            assert_eq!(profile.role, baseline.role, "{} role", baseline.id);
            assert_eq!(
                profile.system_prompt, baseline.marker,
                "{} prompt marker",
                baseline.id
            );
            assert_eq!(
                profile.description, baseline.description,
                "{} description",
                baseline.id
            );
            let allowed: Vec<&str> = profile
                .allowed_subagents
                .iter()
                .map(String::as_str)
                .collect();
            assert_eq!(
                allowed, baseline.allowed,
                "{} allowed_subagents",
                baseline.id
            );

            let mut actual: Vec<(&str, bool, Option<ToolPreset>)> = profile
                .tools
                .iter()
                .map(|(tool, binding)| (tool.as_str(), binding.enabled, binding.preset))
                .collect();
            actual.sort_by(|a, b| a.0.cmp(b.0));
            let mut expected: Vec<(&str, bool, Option<ToolPreset>)> = baseline
                .tools
                .iter()
                .map(|(tool, preset)| (*tool, true, *preset))
                .collect();
            expected.sort_by(|a, b| a.0.cmp(b.0));
            assert_eq!(actual, expected, "{} tool bindings drifted", baseline.id);
        }
    }
}
