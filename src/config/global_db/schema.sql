-- Current global DB schema epoch (PRAGMA user_version = CURRENT_USER_VERSION).
-- Stepwise migration history is deleted. Incompatible DBs must be deleted and rebuilt.

CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- The only LLM state the database owns: one API key per catalog provider.
-- Provider/model facts live in provider-catalog.toml, not here.
CREATE TABLE IF NOT EXISTS provider_credentials (
    provider_id TEXT PRIMARY KEY,
    api_key     TEXT NOT NULL
);

-- The other half of LLM user state: catalog models the user switched off.
-- Stored as the exception, so an empty table means "every catalog model is on"
-- and a model added to the catalog needs no row to be selectable.
CREATE TABLE IF NOT EXISTS disabled_models (
    model_ref TEXT PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS agents (
    id                      TEXT PRIMARY KEY,
    role                    TEXT NOT NULL,
    model_ref               TEXT NOT NULL,
    system_prompt           TEXT NOT NULL DEFAULT '',
    temperature             REAL NOT NULL DEFAULT 0.7,
    max_steps               INTEGER NOT NULL DEFAULT 50,
    description             TEXT NOT NULL DEFAULT '',
    allowed_subagents_json  TEXT NOT NULL DEFAULT '[]'
);

CREATE TABLE IF NOT EXISTS agent_tools (
    agent_id              TEXT NOT NULL,
    tool_id               TEXT NOT NULL,
    enabled               INTEGER NOT NULL,
    policy_json           TEXT NOT NULL DEFAULT '{}',
    path_mode             TEXT NOT NULL DEFAULT 'unrestricted',
    last_applied_preset   TEXT,
    allowed_tools_json    TEXT,
    PRIMARY KEY (agent_id, tool_id)
);

CREATE TABLE IF NOT EXISTS custom_tools (
    id          TEXT PRIMARY KEY,
    schema_json TEXT NOT NULL,
    command     TEXT NOT NULL,
    args_json   TEXT NOT NULL DEFAULT '[]',
    timeout     INTEGER NOT NULL DEFAULT 120,
    description TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS mcp_servers (
    id             TEXT PRIMARY KEY,
    command        TEXT NOT NULL,
    args_json      TEXT NOT NULL DEFAULT '[]',
    env_json       TEXT NOT NULL DEFAULT '{}',
    transport_json TEXT NOT NULL DEFAULT '{"type":"stdio"}',
    timeout        INTEGER NOT NULL DEFAULT 60
);

CREATE TABLE IF NOT EXISTS auth (
    id    INTEGER PRIMARY KEY CHECK (id = 1),
    token TEXT
);

CREATE TABLE IF NOT EXISTS log_settings (
    id    INTEGER PRIMARY KEY CHECK (id = 1),
    level TEXT
);

CREATE TABLE IF NOT EXISTS websearch (
    id               INTEGER PRIMARY KEY CHECK (id = 1),
    search_endpoint  TEXT
);
-- `search_endpoint` is leftover; live config is `meta.websearch.api_key`.
-- Keep this table so existing DBs stay loadable without a bump.
