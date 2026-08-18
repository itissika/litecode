use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};

use crate::config::schema::{
    AgentProfile, AgentRole, AgentToolBinding, AuthSettings, CustomToolDefinition, GlobalSettings,
    InitScope, LogSettings, McpServerDefinition, McpTransport, ModelAdapterConfig, ModelDefinition,
    ProviderConnectionConfig, ProviderDefinition, ToolCatalogEntry, ToolPreset, ToolTier,
    WebSearchSettings,
};
use crate::types::{LitecodeError, Result};

pub mod migrate;

/// Current global DB schema version (exposed for tests/consumers to assert
/// against without reaching into the private migration internals).
pub const fn current_user_version() -> i32 {
    migrate::CURRENT_USER_VERSION
}
mod seed;
pub mod tools;

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

/// Windows: prefer `%LOCALAPPDATA%\litecode\litecode.db`.
///
/// If that file does not exist yet but the legacy XDG-style path under the user
/// profile already has a DB, keep using the legacy path so existing installs are
/// not silently split across two databases. New installs get LOCALAPPDATA.
#[cfg(windows)]
fn windows_default_db_path() -> PathBuf {
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|p| p.join("litecode").join("litecode.db"));

    let legacy = xdg_default_db_path();

    if let Some(ref local_path) = local {
        if local_path.is_file() {
            return local_path.clone();
        }
    }
    if legacy.is_file() {
        return legacy;
    }
    local.unwrap_or(legacy)
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

/// Delete-and-rebuild: archive the incompatible file, drop SQLite sidecars, leave path empty.
fn rebuild_incompatible_db(path: &Path, old_version: i32) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let bak_name = format!("litecode.db.bak-v{old_version}-{stamp}");
    let bak = parent.join(bak_name);

    // Close any lingering WAL before rename by removing sidecars after rename.
    match std::fs::rename(path, &bak) {
        Ok(()) => {
            tracing::warn!(
                old_version,
                current = migrate::CURRENT_USER_VERSION,
                from = %path.display(),
                backup = %bak.display(),
                "incompatible global DB; archived and will recreate (delete-and-rebuild)"
            );
        }
        Err(e) => {
            // Fallback: delete in place if rename fails (e.g. cross-volume).
            tracing::warn!(
                old_version,
                error = %e,
                path = %path.display(),
                "incompatible global DB; rename failed, deleting in place"
            );
            std::fs::remove_file(path).map_err(|rm| {
                LitecodeError::Config(format!(
                    "failed to remove incompatible global DB {}: {rm} (rename error: {e})",
                    path.display()
                ))
            })?;
        }
    }
    remove_sqlite_sidecars(path);
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
            // Idempotent catalog row repair on a *current* schema DB — not schema migration.
            seed::ensure_core_catalog(conn)?;
            seed::ensure_optional_catalog(conn)?;
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

pub fn agent_tools_for(
    conn: &Connection,
    agent_id: &str,
) -> Result<HashMap<String, AgentToolBinding>> {
    store::load_agent_tools(conn, agent_id)
}

pub mod store {
    use super::*;

    pub fn load(conn: &Connection) -> Result<GlobalSettings> {
        Ok(GlobalSettings {
            providers: load_providers(conn)?,
            models: load_models(conn)?,
            tool_catalog: load_tool_catalog(conn)?,
            agents: load_agents(conn)?,
            custom_tools: load_custom_tools(conn)?,
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
        tx.execute("DELETE FROM mcp_servers", [])?;
        tx.execute("DELETE FROM tool_catalog", [])?;
        tx.execute("DELETE FROM models", [])?;
        tx.execute("DELETE FROM providers", [])?;

        for provider in settings.providers.values() {
            save_provider(&tx, provider)?;
        }
        for model in settings.models.values() {
            upsert_model(&tx, model)?;
        }
        for entry in settings.tool_catalog.values() {
            upsert_catalog_entry(
                &tx,
                &entry.id,
                entry.tier,
                entry.init_scope,
                entry.catalog_enabled,
            )?;
        }
        for (id, profile) in &settings.agents {
            upsert_agent(
                &tx,
                id,
                profile.role,
                &profile.model_ref,
                &profile.system_prompt,
                profile.temperature,
                profile.max_steps,
                &profile.description,
                &profile.allowed_subagents,
            )?;
            for (tool_id, binding) in &profile.tools {
                upsert_agent_tool(&tx, id, tool_id, binding)?;
            }
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

    fn load_providers(conn: &Connection) -> Result<HashMap<String, ProviderDefinition>> {
        let mut stmt = conn.prepare("SELECT id, adapter_id, label, config_json FROM providers")?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let adapter_id: String = row.get(1)?;
            let label: String = row.get(2)?;
            let config_json: String = row.get(3)?;
            let config: ProviderConnectionConfig =
                serde_json::from_str(&config_json).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
            Ok((
                id.clone(),
                ProviderDefinition {
                    id,
                    adapter_id,
                    label,
                    config,
                },
            ))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (id, def) = row?;
            map.insert(id, def);
        }
        Ok(map)
    }

    pub fn save_provider(conn: &Connection, provider: &ProviderDefinition) -> Result<()> {
        let config_json = serde_json::to_string(&provider.config)?;
        conn.execute(
            "INSERT INTO providers (id, adapter_id, label, config_json)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
               adapter_id = excluded.adapter_id,
               label = excluded.label,
               config_json = excluded.config_json",
            params![
                provider.id,
                provider.adapter_id,
                provider.label,
                config_json
            ],
        )?;
        Ok(())
    }

    pub fn load_models(conn: &Connection) -> Result<HashMap<String, ModelDefinition>> {
        let mut stmt =
            conn.prepare("SELECT id, adapter_id, provider_ref, label, config_json FROM models")?;
        let rows = stmt.query_map([], |row| {
            let config_json: String = row.get(4)?;
            let config: ModelAdapterConfig = serde_json::from_str(&config_json).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            Ok(ModelDefinition {
                id: row.get(0)?,
                adapter_id: row.get(1)?,
                provider_ref: row.get(2)?,
                label: row.get(3)?,
                config,
            })
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let mut model = row?;
            crate::llm::apply_owned_modality_capabilities(&mut model);
            map.insert(model.id.clone(), model);
        }
        Ok(map)
    }

    pub fn upsert_model(conn: &Connection, model: &ModelDefinition) -> Result<()> {
        let config_json = serde_json::to_string(&model.config)?;
        conn.execute(
            "INSERT INTO models (id, adapter_id, provider_ref, label, config_json)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
               adapter_id = excluded.adapter_id,
               provider_ref = excluded.provider_ref,
               label = excluded.label,
               config_json = excluded.config_json",
            params![
                model.id,
                model.adapter_id,
                model.provider_ref,
                model.label,
                config_json
            ],
        )?;
        Ok(())
    }

    fn load_tool_catalog(conn: &Connection) -> Result<HashMap<String, ToolCatalogEntry>> {
        let mut stmt =
            conn.prepare("SELECT id, tier, init_scope, catalog_enabled FROM tool_catalog")?;
        let mut rows = stmt.query([])?;
        let mut map = HashMap::new();
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let entry = ToolCatalogEntry {
                id: id.clone(),
                tier: parse_tier(row.get(1)?)?,
                init_scope: parse_init_scope(row.get(2)?)?,
                catalog_enabled: row.get::<_, i64>(3)? != 0,
            };
            map.insert(id, entry);
        }
        Ok(map)
    }

    pub fn upsert_catalog_entry(
        conn: &Connection,
        id: &str,
        tier: ToolTier,
        init_scope: InitScope,
        catalog_enabled: bool,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO tool_catalog (id, tier, init_scope, catalog_enabled)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
               tier = excluded.tier,
               init_scope = excluded.init_scope,
               catalog_enabled = excluded.catalog_enabled",
            params![
                id,
                tier_to_str(tier),
                init_scope_to_str(init_scope),
                catalog_enabled as i64
            ],
        )?;
        Ok(())
    }

    fn load_agents(conn: &Connection) -> Result<HashMap<String, AgentProfile>> {
        let mut stmt = conn.prepare(
            "SELECT id, role, model_ref, system_prompt, temperature, max_steps, description,
                    allowed_subagents_json
             FROM agents",
        )?;
        let mut rows = stmt.query([])?;
        let mut map = HashMap::new();
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let allowed_json: String = row.get(7)?;
            let allowed_subagents: Vec<String> =
                serde_json::from_str(&allowed_json).map_err(|e| {
                    LitecodeError::Config(format!(
                        "invalid allowed_subagents_json for agent '{id}': {e}"
                    ))
                })?;
            let mut profile = AgentProfile {
                role: parse_role(row.get(1)?)?,
                model_ref: row.get(2)?,
                system_prompt: row.get(3)?,
                temperature: row.get(4)?,
                max_steps: row.get(5)?,
                description: row.get(6)?,
                tools: HashMap::new(),
                allowed_subagents,
            };
            profile.tools = load_agent_tools(conn, &id)?;
            map.insert(id, profile);
        }
        Ok(map)
    }

    pub fn load_agent_tools(
        conn: &Connection,
        agent_id: &str,
    ) -> Result<HashMap<String, AgentToolBinding>> {
        let mut stmt = conn.prepare(
            "SELECT tool_id, enabled, policy_json, path_mode, last_applied_preset FROM agent_tools WHERE agent_id = ?1",
        )?;
        let mut rows = stmt.query([agent_id])?;
        let mut map = HashMap::new();
        while let Some(row) = rows.next()? {
            let tool_id: String = row.get(0)?;
            let policy_json: String = row.get(2)?;
            let path_mode_str: String = row.get(3)?;
            let last_preset: Option<String> = row.get(4)?;
            let policy = serde_json::from_str(&policy_json).map_err(|e| {
                LitecodeError::Config(format!("invalid policy_json for {tool_id}: {e}"))
            })?;
            let binding = AgentToolBinding {
                enabled: row.get::<_, i64>(1)? != 0,
                policy,
                path_mode: parse_path_mode(&path_mode_str)?,
                last_applied_preset: last_preset.as_deref().map(parse_preset).transpose()?,
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
        temperature: f64,
        max_steps: u32,
        description: &str,
        allowed_subagents: &[String],
    ) -> Result<()> {
        let allowed_json = serde_json::to_string(allowed_subagents)?;
        conn.execute(
            "INSERT INTO agents (id, role, model_ref, system_prompt, temperature, max_steps, description, allowed_subagents_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
               role = excluded.role,
               model_ref = excluded.model_ref,
               system_prompt = excluded.system_prompt,
               temperature = excluded.temperature,
               max_steps = excluded.max_steps,
               description = excluded.description,
               allowed_subagents_json = excluded.allowed_subagents_json",
            params![
                id,
                role_to_str(role),
                model_ref,
                system_prompt,
                temperature,
                max_steps,
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
        let policy_json = serde_json::to_string(&binding.policy)?;
        let path_mode = path_mode_to_str(binding.path_mode);
        let last_preset = binding
            .last_applied_preset
            .map(preset_to_str)
            .map(String::from);
        conn.execute(
            "INSERT INTO agent_tools (agent_id, tool_id, enabled, policy_json, path_mode, last_applied_preset)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(agent_id, tool_id) DO UPDATE SET
               enabled = excluded.enabled,
               policy_json = excluded.policy_json,
               path_mode = excluded.path_mode,
               last_applied_preset = excluded.last_applied_preset",
            params![
                agent_id,
                tool_id,
                binding.enabled as i64,
                policy_json,
                path_mode,
                last_preset,
            ],
        )?;
        Ok(())
    }

    fn load_custom_tools(conn: &Connection) -> Result<Vec<CustomToolDefinition>> {
        let mut stmt = conn.prepare(
            "SELECT id, schema_json, command, args_json, timeout, description FROM custom_tools",
        )?;
        let rows = stmt.query_map([], |row| {
            let schema_json: String = row.get(1)?;
            let args_json: String = row.get(3)?;
            Ok(CustomToolDefinition {
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
            })
        })?;
        let mut tools = Vec::new();
        for row in rows {
            tools.push(row?);
        }
        Ok(tools)
    }

    pub fn upsert_custom_tool(conn: &Connection, custom: &CustomToolDefinition) -> Result<()> {
        let schema_json = serde_json::to_string(&custom.schema)?;
        let args_json = serde_json::to_string(&custom.args)?;
        conn.execute(
            "INSERT INTO custom_tools (id, schema_json, command, args_json, timeout, description)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
               schema_json = excluded.schema_json,
               command = excluded.command,
               args_json = excluded.args_json,
               timeout = excluded.timeout,
               description = excluded.description",
            params![
                custom.name,
                schema_json,
                custom.command,
                args_json,
                custom.timeout,
                custom.description
            ],
        )?;
        Ok(())
    }

    fn load_mcp_servers(conn: &Connection) -> Result<HashMap<String, McpServerDefinition>> {
        let mut stmt = conn
            .prepare("SELECT id, command, args_json, env_json, transport_json FROM mcp_servers")?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let args_json: String = row.get(2)?;
            let env_json: String = row.get(3)?;
            let transport_json: String = row.get(4)?;
            let transport: McpTransport = serde_json::from_str(&transport_json).unwrap_or_default();
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
            "INSERT INTO mcp_servers (id, command, args_json, env_json, transport_json)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
               command = excluded.command,
               args_json = excluded.args_json,
               env_json = excluded.env_json,
               transport_json = excluded.transport_json",
            params![id, mcp.command, args_json, env_json, transport_json],
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
        let search_endpoint = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'websearch.search_endpoint'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(WebSearchSettings {
            search_endpoint: search_endpoint.filter(|s| !s.is_empty()),
        })
    }

    fn save_websearch(conn: &Connection, websearch: &WebSearchSettings) -> Result<()> {
        if let Some(endpoint) = &websearch.search_endpoint {
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('websearch.search_endpoint', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [endpoint],
            )?;
        } else {
            conn.execute(
                "DELETE FROM meta WHERE key = 'websearch.search_endpoint'",
                [],
            )?;
        }
        Ok(())
    }

    fn tier_to_str(tier: ToolTier) -> &'static str {
        match tier {
            ToolTier::Core => "core",
            ToolTier::Optional => "optional",
            ToolTier::Custom => "custom",
            ToolTier::Mcp => "mcp",
        }
    }

    fn parse_tier(s: String) -> Result<ToolTier> {
        match s.as_str() {
            "core" => Ok(ToolTier::Core),
            "optional" => Ok(ToolTier::Optional),
            "custom" => Ok(ToolTier::Custom),
            "mcp" => Ok(ToolTier::Mcp),
            _ => Err(LitecodeError::Config(format!("unknown tool tier: {s}"))),
        }
    }

    fn init_scope_to_str(scope: InitScope) -> &'static str {
        match scope {
            InitScope::None => "none",
            InitScope::Global => "global",
            InitScope::Workspace => "workspace",
        }
    }

    fn parse_init_scope(s: String) -> Result<InitScope> {
        match s.as_str() {
            "none" => Ok(InitScope::None),
            "global" => Ok(InitScope::Global),
            "workspace" => Ok(InitScope::Workspace),
            _ => Err(LitecodeError::Config(format!("unknown init_scope: {s}"))),
        }
    }

    fn role_to_str(role: AgentRole) -> &'static str {
        match role {
            AgentRole::Primary => "primary",
            AgentRole::Subagent => "subagent",
            AgentRole::Hidden => "hidden",
        }
    }

    fn parse_role(s: String) -> Result<AgentRole> {
        match s.as_str() {
            "primary" => Ok(AgentRole::Primary),
            "subagent" => Ok(AgentRole::Subagent),
            "hidden" => Ok(AgentRole::Hidden),
            _ => Err(LitecodeError::Config(format!("unknown agent role: {s}"))),
        }
    }

    fn path_mode_to_str(mode: crate::permission::BindingPathMode) -> &'static str {
        match mode {
            crate::permission::BindingPathMode::WorkspaceOnly => "workspace_only",
            crate::permission::BindingPathMode::Unrestricted => "unrestricted",
        }
    }

    fn parse_path_mode(s: &str) -> Result<crate::permission::BindingPathMode> {
        match s {
            "workspace_only" => Ok(crate::permission::BindingPathMode::WorkspaceOnly),
            "unrestricted" => Ok(crate::permission::BindingPathMode::Unrestricted),
            _ => Err(LitecodeError::Config(format!("unknown path_mode: {s}"))),
        }
    }

    fn preset_to_str(preset: ToolPreset) -> &'static str {
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
    use rusqlite::Connection;

    #[test]
    fn open_archives_incompatible_db_and_recreates() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("litecode.db");

        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch("PRAGMA user_version = 2;").unwrap();
        }

        let conn = open(&db).expect("open must rebuild incompatible db");
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, migrate::CURRENT_USER_VERSION);

        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("litecode.db.bak-v2-"))
            .collect();
        assert_eq!(
            backups.len(),
            1,
            "expected one archived backup, got {backups:?}"
        );
    }
}
