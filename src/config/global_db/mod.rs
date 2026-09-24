use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};

use crate::config::schema::{
    AgentProfile, AgentRole, AgentToolBinding, AuthSettings, CustomToolDefinition, GlobalSettings,
    LogSettings, McpServerDefinition, McpTransport, ToolPreset, WebSearchSettings,
};
use crate::types::{LitecodeError, Result};

pub mod legacy;
pub mod migrate;

/// Current global DB schema version (exposed for tests/consumers to assert
/// against without reaching into the private migration internals).
pub const fn current_user_version() -> i32 {
    migrate::CURRENT_USER_VERSION
}
mod builtin_prompts;
mod seed;
pub mod tools;

pub use builtin_prompts::{
    COMPACTION_PROMPT, DEFAULT_DESCRIPTION, DEFAULT_PROMPT, EXPLORE_DESCRIPTION, EXPLORE_PROMPT,
    GENERAL_DESCRIPTION, GENERAL_PROMPT, ORCHESTRATOR_DESCRIPTION, ORCHESTRATOR_PROMPT,
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
        if old != 0 && !migrate::can_migrate_in_place(old) {
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

pub mod store {
    use super::*;

    pub fn load(conn: &Connection) -> Result<GlobalSettings> {
        Ok(GlobalSettings {
            provider_credentials: provider_credentials(conn)?,
            disabled_models: disabled_models(conn)?,
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

    /// Credentials only. The one-shot legacy migration uses this instead of the
    /// full settings load, so it never depends on tables it does not touch.
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
            "SELECT tool_id, enabled, policy_json, path_mode, last_applied_preset, allowed_tools_json FROM agent_tools WHERE agent_id = ?1",
        )?;
        let mut rows = stmt.query([agent_id])?;
        let mut map = HashMap::new();
        while let Some(row) = rows.next()? {
            let tool_id: String = row.get(0)?;
            let policy_json: String = row.get(2)?;
            let path_mode_str: String = row.get(3)?;
            let last_preset: Option<String> = row.get(4)?;
            let allowed_tools_json: Option<String> = row.get(5)?;
            let policy = serde_json::from_str(&policy_json).map_err(|e| {
                LitecodeError::Config(format!("invalid policy_json for {tool_id}: {e}"))
            })?;
            let allowed_tools = allowed_tools_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(|e| {
                    LitecodeError::Config(format!("invalid allowed_tools_json for {tool_id}: {e}"))
                })?;
            let binding = AgentToolBinding {
                enabled: row.get::<_, i64>(1)? != 0,
                policy,
                path_mode: parse_path_mode(&path_mode_str)?,
                last_applied_preset: last_preset.as_deref().map(parse_preset).transpose()?,
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
        let allowed_tools_json = binding
            .allowed_tools
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        conn.execute(
            "INSERT INTO agent_tools (agent_id, tool_id, enabled, policy_json, path_mode, last_applied_preset, allowed_tools_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(agent_id, tool_id) DO UPDATE SET
               enabled = excluded.enabled,
               policy_json = excluded.policy_json,
               path_mode = excluded.path_mode,
               last_applied_preset = excluded.last_applied_preset,
               allowed_tools_json = excluded.allowed_tools_json",
            params![
                agent_id,
                tool_id,
                binding.enabled as i64,
                policy_json,
                path_mode,
                last_preset,
                allowed_tools_json,
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

    #[test]
    fn open_migrates_v5_without_archive() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("litecode.db");
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "
                CREATE TABLE tool_catalog (
                    id TEXT PRIMARY KEY,
                    tier TEXT NOT NULL,
                    init_scope TEXT NOT NULL,
                    catalog_enabled INTEGER NOT NULL
                );
                CREATE TABLE agents (
                    id TEXT PRIMARY KEY,
                    role TEXT NOT NULL,
                    model_ref TEXT NOT NULL,
                    system_prompt TEXT NOT NULL DEFAULT '',
                    temperature REAL NOT NULL DEFAULT 0.7,
                    max_steps INTEGER NOT NULL DEFAULT 50,
                    description TEXT NOT NULL DEFAULT '',
                    allowed_subagents_json TEXT NOT NULL DEFAULT '[]'
                );
                CREATE TABLE agent_tools (
                    agent_id TEXT NOT NULL,
                    tool_id TEXT NOT NULL,
                    enabled INTEGER NOT NULL,
                    policy_json TEXT NOT NULL DEFAULT '{}',
                    path_mode TEXT NOT NULL DEFAULT 'unrestricted',
                    last_applied_preset TEXT,
                    PRIMARY KEY (agent_id, tool_id)
                );
                CREATE TABLE mcp_servers (
                    id TEXT PRIMARY KEY,
                    command TEXT NOT NULL,
                    args_json TEXT NOT NULL DEFAULT '[]',
                    env_json TEXT NOT NULL DEFAULT '{}',
                    transport_json TEXT NOT NULL DEFAULT '{\"type\":\"stdio\"}'
                );
                INSERT INTO agents (id, role, model_ref) VALUES ('default', 'primary', '');
                PRAGMA user_version = 5;
                ",
            )
            .unwrap();
        }

        let conn = open(&db).expect("v5 must migrate in place");
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, migrate::CURRENT_USER_VERSION);
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".bak-"))
            .collect();
        assert!(backups.is_empty(), "v5 must not be archived: {backups:?}");
        let agents: i64 = conn
            .query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))
            .unwrap();
        assert_eq!(agents, 1);
        let allowed_tools: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('agent_tools') WHERE name='allowed_tools_json'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(allowed_tools, 1);
        let timeout: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('mcp_servers') WHERE name='timeout'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(timeout, 1);
    }

    #[test]
    fn leftover_websearch_endpoint_meta_is_ignored_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("litecode.db");
        let _ = open(&db).unwrap();
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('websearch.search_endpoint', 'https://old.example/mcp')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('websearch.api_key', 'exa-secret')",
                [],
            )
            .unwrap();
        }

        let settings = load_global_from_path(&db).unwrap();
        assert_eq!(settings.websearch.api_key.as_deref(), Some("exa-secret"));

        save_global(&db, &settings).unwrap();
        let conn = Connection::open(&db).unwrap();
        let leftover: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'websearch.search_endpoint'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(leftover, "https://old.example/mcp");
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, migrate::CURRENT_USER_VERSION);
    }
}
