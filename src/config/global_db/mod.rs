use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};

use crate::config::schema::{
    AgentProfile, AgentRole, AgentToolBinding, AuthSettings, CustomSuiteDefinition,
    CustomToolDefinition, GlobalSettings, LogSettings, McpServerDefinition, McpTransport,
    ToolPreset, WebSearchSettings,
};
use crate::types::{LitecodeError, Result};

pub mod migrate;

/// Current global DB schema version (exposed for tests/consumers to assert
/// against without reaching into the private migration internals).
pub const fn current_user_version() -> i32 {
    migrate::CURRENT_USER_VERSION
}
mod builtin_agents;
mod builtin_prompts;
mod seed;
pub mod tools;

pub use builtin_agents::{BuiltinAgentSpec, builtin_agent, builtin_agents, is_builtin_agent};
pub use builtin_prompts::{
    CHECKPOINT_PREAMBLE, CITATION_PROMPT, COMPACTION_INSTRUCTION, DEFAULT_DESCRIPTION,
    DEFAULT_PROMPT, EXPLORE_DESCRIPTION, EXPLORE_PROMPT, GENERAL_DESCRIPTION, GENERAL_PROMPT,
    HARNESS_CODE_RUNTIME, HARNESS_SYSTEM, ORCHESTRATOR_DESCRIPTION, ORCHESTRATOR_PROMPT,
    PLAN_DESCRIPTION, PLAN_PROMPT, VERIFICATION_DESCRIPTION, VERIFICATION_PROMPT,
    builtin_prompt_for, is_builtin_prompt_marker,
};

pub fn default_db_path() -> PathBuf {
    #[cfg(windows)]
    {
        return windows_default_db_path();
    }
    #[cfg(not(windows))]
    {
        xdg_default_db_path()
    }
}

fn xdg_default_db_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".local")
        .join("share")
        .join("litecode")
        .join("litecode.db")
}

/// Windows: `%LOCALAPPDATA%\litecode\litecode.db`.
#[cfg(windows)]
fn windows_default_db_path() -> PathBuf {
    match std::env::var_os("LOCALAPPDATA") {
        Some(local) => PathBuf::from(local).join("litecode").join("litecode.db"),
        None => xdg_default_db_path(),
    }
}

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if path.is_file() {
        let old = peek_user_version(path)?;
        if old != 0 && old != migrate::CURRENT_USER_VERSION {
            rebuild_incompatible_db(path, old)?;
        }
    }

    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode=WAL;
         PRAGMA busy_timeout=5000;",
    )?;
    migrate::migrate(&conn)?;
    Ok(conn)
}

/// Per-path cached connections (G7): the settings hot paths no longer reopen
/// the DB on every request. Single-writer serialization via the mutex is fine —
/// global settings are low-frequency.
static CONN_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<PathBuf, Arc<std::sync::Mutex<Connection>>>>,
> = std::sync::OnceLock::new();

pub fn open_cached(path: &Path) -> Result<Arc<std::sync::Mutex<Connection>>> {
    let map = CONN_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let cached = map
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(path)
        .cloned();
    if let Some(conn) = cached {
        return Ok(conn);
    }
    let conn = Arc::new(std::sync::Mutex::new(open(path)?));
    map.lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(path.to_path_buf(), Arc::clone(&conn));
    Ok(conn)
}

/// Run `f` against the cached connection for `path`.
pub fn with_conn<F, R>(path: &Path, f: F) -> Result<R>
where
    F: FnOnce(&Connection) -> Result<R>,
{
    let conn = open_cached(path)?;
    let guard = conn.lock().unwrap_or_else(|e| e.into_inner());
    f(&guard)
}

/// Read `PRAGMA user_version` without running migrate.
fn peek_user_version(path: &Path) -> Result<i32> {
    let conn = Connection::open(path)?;
    Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?)
}

/// Carry credentials across a rebuild.
///
/// Best-effort: a missing table or a corrupt file yields no rows. Credentials
/// are the only user state that survives an epoch break.
fn salvage_provider_credentials(path: &Path) -> Vec<(String, String)> {
    let Ok(conn) = Connection::open(path) else {
        return Vec::new();
    };
    let sql = "SELECT provider_id, api_key FROM provider_credentials";
    let Ok(mut stmt) = conn.prepare(sql) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return Vec::new();
    };
    rows.flatten()
        .filter(|(_, api_key)| !api_key.trim().is_empty())
        .collect()
}

/// A DB from another schema epoch is never migrated in place: delete the file
/// (and its SQLite sidecars), rebuild from `schema.sql`, seed, and restore the
/// salvaged provider credentials. No archive files are kept.
fn rebuild_incompatible_db(path: &Path, old_version: i32) -> Result<()> {
    let salvaged = salvage_provider_credentials(path);
    tracing::warn!(
        old_version,
        current = migrate::CURRENT_USER_VERSION,
        path = %path.display(),
        salvaged = salvaged.len(),
        "incompatible global DB; deleting and rebuilding from the current schema"
    );
    std::fs::remove_file(path).map_err(|e| {
        LitecodeError::Config(format!(
            "failed to remove incompatible global DB {}: {e}",
            path.display()
        ))
    })?;
    remove_sqlite_sidecars(path);

    let conn = Connection::open(path)?;
    migrate::migrate(&conn)?;
    seed::seed(&conn)?;
    for (provider_id, api_key) in &salvaged {
        conn.execute(
            "INSERT OR REPLACE INTO provider_credentials (provider_id, api_key) VALUES (?1, ?2)",
            params![provider_id, api_key],
        )?;
    }
    Ok(())
}

fn remove_sqlite_sidecars(path: &Path) {
    let base = path.to_string_lossy();
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{base}{suffix}"));
        let _ = std::fs::remove_file(sidecar);
    }
}

pub fn load_global_from_path(path: &Path) -> Result<GlobalSettings> {
    with_conn(path, |conn| {
        if seed::needs_seed(conn)? {
            seed::seed(conn)?;
        } else {
            seed::ensure_core_bindings(conn)?;
        }
        store::load(conn)
    })
}

pub fn load_global() -> Result<GlobalSettings> {
    load_global_from_path(&default_db_path())
}

pub fn save_global(path: &Path, settings: &GlobalSettings) -> Result<()> {
    with_conn(path, |conn| store::replace_all(conn, settings))
}

pub fn import_into(path: &Path, settings: &GlobalSettings) -> Result<()> {
    with_conn(path, |conn| store::replace_all(conn, settings))
}

/// Read one `meta` marker (used by the provider catalog lifecycle).
pub fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .optional()?)
}

/// Write one `meta` marker.
pub fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    store::upsert_meta(conn, key, Some(value))
}

pub fn agent_tools_for(
    conn: &Connection,
    agent_id: &str,
) -> Result<HashMap<String, AgentToolBinding>> {
    store::load_agent_tools(conn, agent_id)
}

/// `meta` key holding the evaluator's scored tool surface.
///
/// This is a system channel, not a setting: `load_agents` lets it **replace**
/// the whole tool map of one built-in agent — including core cards, which no
/// settings write can switch off — and neither the settings UI nor
/// `SettingsWriter::write_agent` can read or write it. Only
/// `config profile apply` / `config profile clear` touch this key.
pub const EVAL_SURFACE_META_KEY: &str = "eval_surface";

/// The evaluator's surface for one built-in agent, as stored under
/// [`EVAL_SURFACE_META_KEY`].
///
/// `cards` is the whole surface: a card missing from the map is off, so the
/// override never merges with the spec or with the user's non-core toggles.
/// The presets are the profile's tiers, and a card without a permission dial
/// carries `None`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EvalSurfaceOverride {
    pub agent_id: String,
    pub cards: BTreeMap<String, Option<ToolPreset>>,
}

/// Boot-time sibling of the per-commit repair in `SettingsWriter`.
///
/// A provider key removed while the app was closed, or a model this build no
/// longer ships, leaves agents pointing at a model that cannot run. No settings
/// write happens at startup to heal that. Writes only the repaired agent rows:
/// the embedded catalog is the source of truth, a stored ref is derived state.
///
/// Lives in `global_db` so rusqlite stays behind the SQL death-list boundary
/// (`tests/session_data_sql_death_list.rs` `is_global_db_ok`).
pub fn repair_agent_models_on_boot(
    conn: &Connection,
    catalog: &crate::provider_catalog::ProviderCatalog,
) -> Result<()> {
    let mut settings = store::load(conn)?;
    let repaired = crate::config::bridge::repair_agent_models(&mut settings, catalog);
    if repaired.is_empty() {
        return Ok(());
    }
    tracing::info!(
        agents = ?repaired,
        "boot: auto-assigned a runnable model to agents whose model_ref could not run"
    );
    for id in repaired {
        let Some(profile) = settings.agents.get(&id) else {
            continue;
        };
        store::upsert_agent(
            conn,
            &id,
            profile.role,
            &profile.model_ref,
            &profile.system_prompt,
            &profile.description,
            &profile.allowed_subagents,
        )?;
    }
    Ok(())
}

pub mod store {
    use super::*;

    pub fn load(conn: &Connection) -> Result<GlobalSettings> {
        Ok(GlobalSettings {
            provider_credentials: provider_credentials(conn)?,
            disabled_models: disabled_models(conn)?,
            agents: load_agents(conn)?,
            custom_tools: load_custom_tools(conn)?,
            custom_suites: load_custom_suites(conn)?,
            mcp_servers: load_mcp_servers(conn)?,
            auth: load_auth(conn)?,
            log: load_log(conn)?,
            websearch: load_websearch(conn)?,
        })
    }

    pub fn replace_all(conn: &Connection, settings: &GlobalSettings) -> Result<()> {
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM agent_tools", [])?;
        tx.execute("DELETE FROM agents", [])?;
        tx.execute("DELETE FROM custom_tools", [])?;
        tx.execute("DELETE FROM custom_suites", [])?;
        tx.execute("DELETE FROM mcp_servers", [])?;
        tx.execute("DELETE FROM provider_credentials", [])?;
        tx.execute("DELETE FROM disabled_models", [])?;

        for (provider_id, api_key) in &settings.provider_credentials {
            set_provider_credential(&tx, provider_id, api_key)?;
        }
        for model_ref in &settings.disabled_models {
            tx.execute(
                "INSERT OR IGNORE INTO disabled_models (model_ref) VALUES (?1)",
                params![model_ref],
            )?;
        }
        for (id, profile) in &settings.agents {
            upsert_agent(
                &tx,
                id,
                profile.role,
                &profile.model_ref,
                &profile.system_prompt,
                &profile.description,
                &profile.allowed_subagents,
            )?;
            for (tool_id, binding) in &profile.tools {
                // Core bindings live in the spec, not in the DB: writing them
                // back would only leave dead rows the next read has to ignore.
                if super::tools::is_core_tool(tool_id) {
                    continue;
                }
                upsert_agent_tool(&tx, id, tool_id, binding)?;
            }
        }
        for (id, suite) in &settings.custom_suites {
            upsert_custom_suite(&tx, id, suite)?;
        }
        for custom in &settings.custom_tools {
            upsert_custom_tool(&tx, custom)?;
        }
        for (id, mcp) in &settings.mcp_servers {
            upsert_mcp_server(&tx, id, mcp)?;
        }
        save_auth(&tx, &settings.auth)?;
        save_log(&tx, &settings.log)?;
        save_websearch(&tx, &settings.websearch)?;
        tx.commit()?;
        Ok(())
    }

    /// Credentials only, without the full settings load.
    pub(crate) fn provider_credentials(conn: &Connection) -> Result<HashMap<String, String>> {
        let mut stmt = conn.prepare("SELECT provider_id, api_key FROM provider_credentials")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (provider_id, api_key) = row?;
            if !api_key.trim().is_empty() {
                map.insert(provider_id, api_key);
            }
        }
        Ok(map)
    }

    /// Catalog model refs the user switched off. Stored as the exception, so an
    /// empty table means "every catalog model is on".
    pub(crate) fn disabled_models(conn: &Connection) -> Result<HashSet<String>> {
        let mut stmt = conn.prepare("SELECT model_ref FROM disabled_models")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut set = HashSet::new();
        for row in rows {
            let model_ref = row?;
            if !model_ref.trim().is_empty() {
                set.insert(model_ref);
            }
        }
        Ok(set)
    }

    /// Insert or replace one provider credential.
    pub fn set_provider_credential(
        conn: &Connection,
        provider_id: &str,
        api_key: &str,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO provider_credentials (provider_id, api_key) VALUES (?1, ?2)
             ON CONFLICT(provider_id) DO UPDATE SET api_key = excluded.api_key",
            params![provider_id, api_key],
        )?;
        Ok(())
    }

    /// Remove one provider credential. The catalog provider itself never goes away.
    pub fn delete_provider_credential(conn: &Connection, provider_id: &str) -> Result<()> {
        conn.execute(
            "DELETE FROM provider_credentials WHERE provider_id = ?1",
            [provider_id],
        )?;
        Ok(())
    }

    /// The evaluator's override from `meta`, if one is stored.
    ///
    /// A corrupt value is an error, not a silent `None`: dropping it would score
    /// a run against the full surface, which reads exactly like a model that was
    /// simply better.
    pub fn load_eval_surface(conn: &Connection) -> Result<Option<EvalSurfaceOverride>> {
        let Some(raw) = super::meta_get(conn, super::EVAL_SURFACE_META_KEY)? else {
            return Ok(None);
        };
        let surface: EvalSurfaceOverride = serde_json::from_str(&raw).map_err(|error| {
            LitecodeError::Config(format!(
                "invalid `{}` value in the settings database: {error}",
                super::EVAL_SURFACE_META_KEY
            ))
        })?;
        Ok(Some(surface))
    }

    /// Replace the stored override wholesale. Never merged: the caller's map is
    /// the entire surface.
    pub fn write_eval_surface(conn: &Connection, surface: &EvalSurfaceOverride) -> Result<()> {
        let raw = serde_json::to_string(surface)?;
        super::meta_set(conn, super::EVAL_SURFACE_META_KEY, &raw)
    }

    /// Delete the override. `true` when a row was actually removed.
    pub fn clear_eval_surface(conn: &Connection) -> Result<bool> {
        let removed = conn.execute(
            "DELETE FROM meta WHERE key = ?1",
            [super::EVAL_SURFACE_META_KEY],
        )?;
        Ok(removed > 0)
    }

    /// Build the agent map from the built-in specs, never from the `agents`
    /// table's identity columns.
    ///
    /// The spec is the single source for `role`, `system_prompt`,
    /// `description`, `allowed_subagents`, every core binding and its preset.
    /// The DB contributes exactly two things: `model_ref`, and the user's
    /// overrides for **non-core** bindings (optional builtins, custom tools,
    /// MCP). Core rows, unknown agent rows and tampered identity columns in the
    /// DB are ignored here.
    ///
    /// Precedence for `tools`, in this order: spec core → spec optional
    /// defaults → DB rows for this agent (core rows skipped, role-invalid rows
    /// skipped, dial-less optional presets forced to `None`). A user's explicit
    /// `enabled=0` row therefore still wins over the spec default.
    ///
    /// One exception: an [`EvalSurfaceOverride`] stored under
    /// [`super::EVAL_SURFACE_META_KEY`] that names this agent **replaces** the
    /// whole tool map, core cards included. That is the evaluator's channel;
    /// a settings write cannot reach it, and it does not touch
    /// `agent_tools`, so `meta` clearing restores the user's surface at once.
    fn load_agents(conn: &Connection) -> Result<HashMap<String, AgentProfile>> {
        let eval_surface = load_eval_surface(conn)?;
        let mut map = HashMap::new();
        for spec in super::builtin_agents::builtin_agents() {
            let model_ref: String = conn
                .query_row(
                    "SELECT model_ref FROM agents WHERE id = ?1",
                    [spec.id],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or_default();

            let mut tools: HashMap<String, AgentToolBinding> = HashMap::new();
            let surface = eval_surface
                .as_ref()
                .filter(|surface| surface.agent_id == spec.id);
            match surface {
                Some(surface) => {
                    for (card_id, preset) in &surface.cards {
                        tools.insert(
                            card_id.clone(),
                            AgentToolBinding {
                                enabled: true,
                                preset: *preset,
                                allowed_tools: None,
                            },
                        );
                    }
                }
                None => {
                    for (tool_id, binding) in spec.core_bindings() {
                        tools.insert(tool_id.to_string(), binding);
                    }
                    for (tool_id, binding) in spec.default_optional_bindings() {
                        tools.insert(tool_id.to_string(), binding);
                    }
                    for (tool_id, mut binding) in load_non_core_agent_tools(conn, spec.id)? {
                        if !crate::config::binding_scope::scope_of(&tool_id).allows_role(spec.role) {
                            continue;
                        }
                        if super::tools::is_optional_builtin(&tool_id)
                            && !crate::permission::has_permission_preset(&tool_id)
                        {
                            binding.preset = None;
                        }
                        tools.insert(tool_id, binding);
                    }
                }
            }

            map.insert(
                spec.id.to_string(),
                AgentProfile {
                    role: spec.role,
                    model_ref,
                    system_prompt: spec.prompt_marker(),
                    description: spec.description.to_string(),
                    tools,
                    allowed_subagents: spec
                        .allowed_subagents
                        .iter()
                        .map(|id| (*id).to_string())
                        .collect(),
                },
            );
        }
        Ok(map)
    }

    /// Every `agent_tools` row for one agent, exactly as stored.
    ///
    /// Kept for callers that want the raw table (diagnostics, the settings
    /// paths that read what the user has). `load_agents` uses
    /// [`Self::load_non_core_agent_tools`] instead.
    pub fn load_agent_tools(
        conn: &Connection,
        agent_id: &str,
    ) -> Result<HashMap<String, AgentToolBinding>> {
        read_agent_tool_rows(conn, agent_id, false)
    }

    /// The user's overrides for one agent, with core rows dropped **before**
    /// `preset` and `allowed_tools_json` are parsed.
    ///
    /// Core bindings always come from the spec, so a leftover core row (older
    /// build, hand edit) is dead weight — and one carrying `preset = 'BOGUS'` or
    /// `allowed_tools_json = 'not-json'` must not be able to break the boot
    /// load just because it was parsed before being skipped.
    fn load_non_core_agent_tools(
        conn: &Connection,
        agent_id: &str,
    ) -> Result<HashMap<String, AgentToolBinding>> {
        read_agent_tool_rows(conn, agent_id, true)
    }

    fn read_agent_tool_rows(
        conn: &Connection,
        agent_id: &str,
        skip_core: bool,
    ) -> Result<HashMap<String, AgentToolBinding>> {
        let mut stmt = conn.prepare(
            "SELECT tool_id, enabled, preset, allowed_tools_json FROM agent_tools WHERE agent_id = ?1",
        )?;
        let mut rows = stmt.query([agent_id])?;
        let mut map = HashMap::new();
        while let Some(row) = rows.next()? {
            // The tool id is read first so a skipped row never has its `preset`
            // or `allowed_tools_json` parsed.
            let tool_id: String = row.get(0)?;
            if skip_core && super::tools::is_core_tool(&tool_id) {
                continue;
            }
            let preset: Option<String> = row.get(2)?;
            let allowed_tools_json: Option<String> = row.get(3)?;
            let allowed_tools = allowed_tools_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(|e| {
                    LitecodeError::Config(format!("invalid allowed_tools_json for {tool_id}: {e}"))
                })?;
            let binding = AgentToolBinding {
                enabled: row.get::<_, i64>(1)? != 0,
                preset: preset.as_deref().map(parse_preset).transpose()?,
                allowed_tools,
            };
            map.insert(tool_id, binding);
        }
        Ok(map)
    }

    pub fn upsert_agent(
        conn: &Connection,
        id: &str,
        role: AgentRole,
        model_ref: &str,
        system_prompt: &str,
        description: &str,
        allowed_subagents: &[String],
    ) -> Result<()> {
        let allowed_json = serde_json::to_string(allowed_subagents)?;
        conn.execute(
            "INSERT INTO agents (id, role, model_ref, system_prompt, description, allowed_subagents_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
               role = excluded.role,
               model_ref = excluded.model_ref,
               system_prompt = excluded.system_prompt,
               description = excluded.description,
               allowed_subagents_json = excluded.allowed_subagents_json",
            params![
                id,
                role_to_str(role),
                model_ref,
                system_prompt,
                description,
                allowed_json,
            ],
        )?;
        Ok(())
    }

    pub fn upsert_agent_tool(
        conn: &Connection,
        agent_id: &str,
        tool_id: &str,
        binding: &AgentToolBinding,
    ) -> Result<()> {
        let preset = binding.preset.map(preset_to_str);
        let allowed_tools_json = binding
            .allowed_tools
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        conn.execute(
            "INSERT INTO agent_tools (agent_id, tool_id, enabled, preset, allowed_tools_json)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(agent_id, tool_id) DO UPDATE SET
               enabled = excluded.enabled,
               preset = excluded.preset,
               allowed_tools_json = excluded.allowed_tools_json",
            params![
                agent_id,
                tool_id,
                binding.enabled as i64,
                preset,
                allowed_tools_json,
            ],
        )?;
        Ok(())
    }

    fn load_custom_tools(conn: &Connection) -> Result<Vec<CustomToolDefinition>> {
        let mut stmt = conn.prepare(
            "SELECT id, schema_json, command, args_json, timeout, description, rules_json, suite_id, definition_version FROM custom_tools",
        )?;
        let rows = stmt.query_map([], |row| {
            let schema_json: String = row.get(1)?;
            let args_json: String = row.get(3)?;
            let rules_json: Option<String> = row.get(6)?;
            let rules = match rules_json {
                Some(json) => serde_json::from_str(&json).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        6,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
                None => Vec::new(),
            };
            let version: i64 = row.get(8)?;
            Ok((
                CustomToolDefinition {
                    name: row.get(0)?,
                    schema: serde_json::from_str(&schema_json).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?,
                    command: row.get(2)?,
                    args: serde_json::from_str(&args_json).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?,
                    timeout: row.get::<_, u64>(4)?,
                    description: row.get(5)?,
                    rules,
                    suite: row.get::<_, Option<String>>(7)?,
                },
                version,
            ))
        })?;
        let mut tools = Vec::new();
        for row in rows {
            let (tool, version) = row?;
            if version != 2 {
                tracing::warn!(
                    tool = %tool.name,
                    version,
                    "skipping legacy global custom tool; rewrite it in the code-only custom_tools + suites format"
                );
                continue;
            }
            if let Some(reason) = super::tools::non_transport_name_rejection(&tool.name)
                .or_else(|| super::tools::reserved_command_name_rejection(&tool.name))
            {
                tracing::warn!(
                    tool = %tool.name,
                    reason,
                    "skipping global custom tool"
                );
                continue;
            }
            tools.push(tool);
        }
        Ok(tools)
    }

    pub fn upsert_custom_tool(conn: &Connection, custom: &CustomToolDefinition) -> Result<()> {
        let schema_json = serde_json::to_string(&custom.schema)?;
        let args_json = serde_json::to_string(&custom.args)?;
        let rules_json = if custom.rules.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&custom.rules)?)
        };
        conn.execute(
            "INSERT INTO custom_tools (id, schema_json, command, args_json, timeout, description, rules_json, suite_id, definition_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 2)
             ON CONFLICT(id) DO UPDATE SET
               schema_json = excluded.schema_json,
               command = excluded.command,
               args_json = excluded.args_json,
               timeout = excluded.timeout,
               description = excluded.description,
               rules_json = excluded.rules_json,
               suite_id = excluded.suite_id,
               definition_version = 2",
            params![
                custom.name,
                schema_json,
                custom.command,
                args_json,
                custom.timeout,
                custom.description,
                rules_json,
                custom.suite,
            ],
        )?;
        Ok(())
    }

    fn load_custom_suites(conn: &Connection) -> Result<HashMap<String, CustomSuiteDefinition>> {
        let mut stmt = conn.prepare("SELECT id, label, rules_json FROM custom_suites")?;
        let rows = stmt.query_map([], |row| {
            let rules = match row.get::<_, Option<String>>(2)? {
                Some(json) => serde_json::from_str(&json).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
                None => Vec::new(),
            };
            Ok((
                row.get::<_, String>(0)?,
                CustomSuiteDefinition {
                    label: row.get(1)?,
                    rules,
                },
            ))
        })?;
        let mut suites = HashMap::new();
        for row in rows {
            let (id, suite) = row?;
            suites.insert(id, suite);
        }
        Ok(suites)
    }

    pub fn upsert_custom_suite(
        conn: &Connection,
        id: &str,
        suite: &CustomSuiteDefinition,
    ) -> Result<()> {
        let rules_json = if suite.rules.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&suite.rules)?)
        };
        conn.execute(
            "INSERT INTO custom_suites (id, label, rules_json) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET
               label = excluded.label,
               rules_json = excluded.rules_json",
            params![id, suite.label, rules_json],
        )?;
        Ok(())
    }

    fn load_mcp_servers(conn: &Connection) -> Result<HashMap<String, McpServerDefinition>> {
        let mut stmt = conn.prepare(
            "SELECT id, command, args_json, env_json, transport_json, timeout FROM mcp_servers",
        )?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let args_json: String = row.get(2)?;
            let env_json: String = row.get(3)?;
            let transport_json: String = row.get(4)?;
            let transport: McpTransport = serde_json::from_str(&transport_json).unwrap_or_default();
            let timeout: i64 = row.get(5)?;
            Ok((
                id.clone(),
                McpServerDefinition {
                    command: row.get(1)?,
                    args: serde_json::from_str(&args_json).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?,
                    env: serde_json::from_str(&env_json).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?,
                    transport,
                    timeout: if timeout <= 0 {
                        crate::config::schema::DEFAULT_MCP_TOOL_TIMEOUT_SECS
                    } else {
                        timeout as u64
                    },
                },
            ))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (id, mcp) = row?;
            map.insert(id, mcp);
        }
        Ok(map)
    }

    pub fn upsert_mcp_server(conn: &Connection, id: &str, mcp: &McpServerDefinition) -> Result<()> {
        let args_json = serde_json::to_string(&mcp.args)?;
        let env_json = serde_json::to_string(&mcp.env)?;
        let transport_json = serde_json::to_string(&mcp.transport)?;
        conn.execute(
            "INSERT INTO mcp_servers (id, command, args_json, env_json, transport_json, timeout)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
               command = excluded.command,
               args_json = excluded.args_json,
               env_json = excluded.env_json,
               transport_json = excluded.transport_json,
               timeout = excluded.timeout",
            params![
                id,
                mcp.command,
                args_json,
                env_json,
                transport_json,
                mcp.call_timeout_secs() as i64
            ],
        )?;
        Ok(())
    }

    fn load_auth(_conn: &Connection) -> Result<AuthSettings> {
        // Inbound serve auth is env-only (`LITECODE_TOKEN`); never surface DB tokens.
        Ok(AuthSettings::default())
    }

    fn save_auth(conn: &Connection, _auth: &AuthSettings) -> Result<()> {
        // Drop any legacy persisted token on settings writes.
        conn.execute("DELETE FROM meta WHERE key = 'auth.token'", [])?;
        Ok(())
    }

    fn load_log(conn: &Connection) -> Result<LogSettings> {
        let level = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'log.level'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(LogSettings {
            level: level.filter(|l| !l.is_empty()),
        })
    }

    fn save_log(conn: &Connection, log: &LogSettings) -> Result<()> {
        if let Some(level) = &log.level {
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('log.level', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [level],
            )?;
        } else {
            conn.execute("DELETE FROM meta WHERE key = 'log.level'", [])?;
        }
        Ok(())
    }

    fn load_websearch(conn: &Connection) -> Result<WebSearchSettings> {
        // `websearch.search_endpoint` may still exist in older DBs; ignore it.
        let api_key = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'websearch.api_key'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(WebSearchSettings {
            api_key: api_key.filter(|s| !s.is_empty()),
        })
    }

    fn save_websearch(conn: &Connection, websearch: &WebSearchSettings) -> Result<()> {
        upsert_meta(conn, "websearch.api_key", websearch.api_key.as_deref())?;
        Ok(())
    }

    pub(super) fn upsert_meta(conn: &Connection, key: &str, value: Option<&str>) -> Result<()> {
        if let Some(value) = value.filter(|s| !s.is_empty()) {
            conn.execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
        } else {
            conn.execute("DELETE FROM meta WHERE key = ?1", [key])?;
        }
        Ok(())
    }

    fn role_to_str(role: AgentRole) -> &'static str {
        match role {
            AgentRole::Primary => "primary",
            AgentRole::Subagent => "subagent",
        }
    }

    pub(crate) fn preset_to_str(preset: ToolPreset) -> &'static str {
        match preset {
            ToolPreset::All => "ALL",
            ToolPreset::Safe => "SAFE",
        }
    }

    fn parse_preset(s: &str) -> Result<ToolPreset> {
        match s {
            "ALL" => Ok(ToolPreset::All),
            "SAFE" => Ok(ToolPreset::Safe),
            _ => Err(LitecodeError::Config(format!("unknown tool preset: {s}"))),
        }
    }
}

#[cfg(test)]
mod open_tests {
    use super::*;
    use crate::config::schema::ToolSchema;

    #[test]
    fn an_incompatible_db_is_deleted_and_rebuilt_keeping_only_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("litecode.db");
        {
            let conn = open(&path).unwrap();
            seed::seed(&conn).unwrap();
            store::set_provider_credential(&conn, "openai", "sk-keep-me").unwrap();
            conn.execute("UPDATE agents SET model_ref = 'legacy/model' WHERE id = 'default'", [])
                .unwrap();
            conn.execute_batch("PRAGMA user_version = 4;").unwrap();
        }

        let settings = load_global_from_path(&path).expect("an incompatible DB is rebuilt");

        let conn = open(&path).unwrap();
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, migrate::CURRENT_USER_VERSION);
        assert_eq!(settings.agents.len(), 6, "the rebuild seeds the six agents");
        assert_eq!(
            settings.agents["default"].model_ref, "",
            "everything except credentials is discarded, not carried over"
        );
        assert_eq!(
            settings
                .provider_credentials
                .get("openai")
                .map(String::as_str),
            Some("sk-keep-me"),
            "provider credentials survive the rebuild"
        );

        let archives: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".bak"))
            .collect();
        assert!(
            archives.is_empty(),
            "a rebuild leaves no archive files behind: {archives:?}"
        );
    }

    #[test]
    fn an_incompatible_db_without_a_credentials_table_still_rebuilds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("litecode.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE junk (id TEXT); PRAGMA user_version = 4;")
                .unwrap();
        }

        let settings = load_global_from_path(&path).expect("a failed salvage must not block");

        assert_eq!(settings.agents.len(), 6);
        assert!(settings.provider_credentials.is_empty());
    }

    #[test]
    fn custom_tool_roundtrips_through_global_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("litecode.db");
        let conn = open(&path).unwrap();
        seed::seed(&conn).unwrap();

        let def = CustomToolDefinition {
            name: "code_probe".into(),
            description: "code probe".into(),
            schema: ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: Vec::new(),
            },
            command: "echo".into(),
            args: Vec::new(),
            timeout: 120,
            rules: Vec::new(),
            suite: None,
        };
        store::upsert_custom_tool(&conn, &def).unwrap();

        let loaded = store::load(&conn).unwrap();
        let custom = loaded
            .custom_tools
            .iter()
            .find(|tool| tool.name == "code_probe")
            .expect("custom tool loaded");
        assert_eq!(custom.command, "echo");
    }
}

/// The DB cannot redefine a built-in agent.
///
/// A tampered database is a first-class scenario (older builds, hand edits):
/// identity columns, core rows, and extra agents are ignored on read — with the
/// one exception that non-core user intent is honored.
#[cfg(test)]
mod lockdown_tests {
    use super::*;
    use crate::config::schema::ToolPreset;
    use crate::config::{ConfigManager, WorkspaceState};

    const AGENTS: [&str; 6] = [
        "default",
        "plan",
        "orchestrator",
        "explore",
        "general",
        "verification",
    ];

    fn put_tool(conn: &Connection, agent: &str, tool: &str, enabled: bool, preset: Option<&str>) {
        let binding = AgentToolBinding {
            enabled,
            preset: preset.map(|value| match value {
                "ALL" => ToolPreset::All,
                "SAFE" => ToolPreset::Safe,
                other => panic!("unknown preset {other}"),
            }),
            allowed_tools: None,
        };
        store::upsert_agent_tool(conn, agent, tool, &binding).unwrap();
    }

    #[test]
    fn a_tampered_db_cannot_redefine_the_builtin_agents() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("litecode.db");
        // First boot: migrate + seed. Second boot: the repair path.
        ConfigManager::load_global_from(&db).expect("seed");
        ConfigManager::load_global_from(&db).expect("repair");

        let spec = builtin_agents()
            .iter()
            .find(|spec| spec.id == "default")
            .expect("default spec");
        let explore = builtin_agents()
            .iter()
            .find(|spec| spec.id == "explore")
            .expect("explore spec");

        {
            let conn = open(&db).unwrap();
            // Identity tampering on `default`.
            conn.execute(
                "UPDATE agents SET role = 'subagent', system_prompt = 'HACKED',
                    description = 'x', allowed_subagents_json = '[]', model_ref = 'prov/m1'
                 WHERE id = 'default'",
                [],
            )
            .unwrap();
            // Core tool tampering on `default`.
            put_tool(&conn, "default", "read", false, None);
            put_tool(&conn, "default", "exec", true, Some("SAFE"));
            // Core tool tampering on `explore`: a preset bump plus a tool it
            // must never bind.
            put_tool(&conn, "explore", "exec", true, Some("ALL"));
            put_tool(&conn, "explore", "write", true, Some("ALL"));
            // A custom agent from an older build, even with a retired role.
            conn.execute(
                "INSERT INTO agents (id, role, model_ref, system_prompt, description, allowed_subagents_json)
                 VALUES ('my_custom', 'hidden', 'prov/x', 'HACKED', 'x', '[]')",
                [],
            )
            .unwrap();
            put_tool(&conn, "my_custom", "read", true, None);
            // Non-core user intent on `default`.
            put_tool(&conn, "default", "code_search", false, None);
            put_tool(&conn, "default", "lsp", true, None);
        }

        let settings = load_global_from_path(&db).expect("load after tampering");

        // The closed set. `my_custom` is gone from memory *and* from the DB.
        assert_eq!(settings.agents.len(), 6);
        assert!(!settings.agents.contains_key("my_custom"));
        let mut ids: Vec<&str> = settings.agents.keys().map(String::as_str).collect();
        ids.sort_unstable();
        let mut expected = AGENTS.to_vec();
        expected.sort_unstable();
        assert_eq!(ids, expected);
        {
            let conn = open(&db).unwrap();
            let agents: i64 = conn
                .query_row("SELECT COUNT(*) FROM agents WHERE id = 'my_custom'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            let bindings: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM agent_tools WHERE agent_id = 'my_custom'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(agents, 0, "the custom agent row must be deleted");
            assert_eq!(bindings, 0, "its bindings must go with it");
        }

        // Identity, prompt, description, allowed subagents come from the spec.
        let default = &settings.agents["default"];
        assert_eq!(default.role, spec.role);
        assert_eq!(default.system_prompt, spec.prompt_marker());
        assert_eq!(default.description, spec.description);
        assert_eq!(
            default.allowed_subagents,
            spec.allowed_subagents
                .iter()
                .map(|id| (*id).to_string())
                .collect::<Vec<_>>()
        );
        // The model choice is the one thing the DB owns.
        assert_eq!(default.model_ref, "prov/m1");

        // Core bindings and presets stay exactly the spec.
        assert!(default.tools["read"].enabled, "read stays on");
        assert_eq!(default.tools["read"].preset, Some(ToolPreset::All));
        assert_eq!(default.tools["exec"].preset, Some(ToolPreset::All));

        let explore_profile = &settings.agents["explore"];
        assert_eq!(explore_profile.role, explore.role);
        assert_eq!(explore_profile.tools["exec"].preset, Some(ToolPreset::Safe));
        assert!(
            !explore_profile.tools.contains_key("write"),
            "explore must never bind write"
        );

        // Non-core intent survives, and a missing row keeps the spec default.
        assert!(!default.tools["code_search"].enabled, "explicit off sticks");
        assert!(default.tools["lsp"].enabled);
        for tool in ["webfetch", "websearch"] {
            assert!(default.tools[tool].enabled, "{tool} defaults on");
            assert_eq!(default.tools[tool].preset, None, "{tool} has no dial");
        }

        // The locked surface still resolves into a valid document.
        ConfigManager::validate(&settings).expect("tampered DB still validates");
        let _ = ConfigManager::resolve_without_catalog(settings, WorkspaceState::new(dir.path()));
    }

    #[test]
    fn a_database_with_an_unknown_role_cannot_break_the_load() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("litecode.db");
        ConfigManager::load_global_from(&db).expect("seed");
        {
            let conn = open(&db).unwrap();
            conn.execute(
                "INSERT INTO agents (id, role, model_ref, system_prompt, description, allowed_subagents_json)
                 VALUES ('ghost', 'not-a-role', '', 'x', 'x', 'not json')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO agent_tools (agent_id, tool_id, enabled, preset, allowed_tools_json)
                 VALUES ('ghost', 'read', 1, 'BOGUS', NULL)",
                [],
            )
            .unwrap();
        }

        let settings = load_global_from_path(&db).expect("a broken extra row must not fail the load");
        assert_eq!(settings.agents.len(), 6);
        assert!(!settings.agents.contains_key("ghost"));
    }

    /// A core row is skipped **before** `preset` / `allowed_tools_json` are
    /// parsed, so garbage on `'default'`'s core rows cannot break the boot load.
    /// The binding itself still comes from the spec.
    #[test]
    fn a_corrupt_core_binding_row_cannot_break_the_load() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("litecode.db");
        ConfigManager::load_global_from(&db).expect("seed");
        {
            let conn = open(&db).unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO agent_tools
                   (agent_id, tool_id, enabled, preset, allowed_tools_json)
                 VALUES ('default', 'read', 0, 'BOGUS', 'not-json')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO agent_tools
                   (agent_id, tool_id, enabled, preset, allowed_tools_json)
                 VALUES ('default', 'exec', 1, 'BOGUS', NULL)",
                [],
            )
            .unwrap();
        }

        let settings =
            load_global_from_path(&db).expect("a garbage core row must not fail the load");
        let default = &settings.agents["default"];
        assert!(default.tools["read"].enabled, "read comes from the spec");
        assert_eq!(default.tools["read"].preset, Some(ToolPreset::All));
        assert_eq!(default.tools["read"].allowed_tools, None);
        assert_eq!(default.tools["exec"].preset, Some(ToolPreset::All));
    }

    /// A full settings save writes the user's **non-core** rows only. Core
    /// bindings belong to the spec, so no dead row is written per core tool.
    #[test]
    fn a_settings_save_leaves_no_core_rows_in_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("litecode.db");
        let settings = ConfigManager::load_global_from(&db).expect("seed");
        save_global(&db, &settings).expect("save");

        let conn = open(&db).unwrap();
        let mut stmt = conn.prepare("SELECT tool_id FROM agent_tools").unwrap();
        let ids: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        let core: Vec<&String> = ids
            .iter()
            .filter(|id| tools::is_core_tool(id))
            .collect();
        assert!(
            core.is_empty(),
            "core bindings belong to the spec, not the DB: {core:?}"
        );
        // Non-core rows still round-trip, so the user's switches survive a save.
        assert!(
            ids.iter().any(|id| id == "code_search"),
            "the user's non-core rows are still written: {ids:?}"
        );
    }
}
