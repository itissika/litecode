//! Global-layer configuration schema (`GlobalSettings`).
//!
//! Fields here are disjoint from [`super::resolved::WorkspaceState`] per CONFIG.md §1.4.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::config::binding_scope::BindingScope;
use serde_json::Value;

/// Agent role (`primary` / `subagent` / `hidden`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum AgentRole {
    #[default]
    Primary,
    Subagent,
    Hidden,
}

/// Per-tool permission preset for configurable-strategy tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ToolPreset {
    All,
    Safe,
}

/// Per-agent tool binding (`agents.<id>.tools.<tool>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentToolBinding {
    pub enabled: bool,
    /// Permission intent for tools with a dial. Fixed tools (MCP, bind-only
    /// builtins, custom tools without rules) keep `None`; policy and path mode
    /// are derived at execution time, never stored.
    #[serde(default)]
    pub preset: Option<ToolPreset>,
    /// Static MCP server tool allowlist. `None` preserves the server's full catalog.
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
}

/// Agent profile (global layer; uses `role` + `model_ref`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentProfile {
    pub role: AgentRole,
    pub model_ref: String,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tools: HashMap<String, AgentToolBinding>,
    /// Primary only: subagent ids this agent may launch (`subagent_launch`). Empty = none allowed.
    #[serde(default)]
    pub allowed_subagents: Vec<String>,
}

impl Default for AgentProfile {
    fn default() -> Self {
        Self {
            role: AgentRole::Primary,
            model_ref: String::new(),
            system_prompt: String::new(),
            temperature: default_temperature(),
            max_steps: default_max_steps(),
            description: String::new(),
            tools: HashMap::new(),
            allowed_subagents: Vec::new(),
        }
    }
}

/// Agent ids that cannot be deleted via settings API.
pub const PROTECTED_AGENT_IDS: &[&str] = &["default"];

/// One-card capability id for the six subagent orchestration members.
pub const SUBAGENT_CAPABILITY_ID: &str = "subagent_tools";

/// Concrete model functions behind [`SUBAGENT_CAPABILITY_ID`].
/// These are members, not separate bindings / settings cards.
pub const SUBAGENT_MEMBER_TOOL_IDS: &[&str] = &[
    "subagent_launch",
    "subagent_wait",
    "subagent_stop",
    "subagent_list",
    "subagent_send",
    "subagent_delete",
];

/// Task-board / memory / human-Ask tools. Subagent role does not bind these (tool-set gate).
/// `knowledge` and `litecode_workspace` stay on the subagent tool set.
pub const PRIMARY_ONLY_TOOL_IDS: &[&str] = &["plan", "todo", "memory", "ask_user", "browser"];

fn default_temperature() -> f64 {
    0.7
}

fn default_max_steps() -> u32 {
    50
}

/// Bind-card kind shown on Agents (not a persist gate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvailableKind {
    Core,
    Engine,
    Custom,
    Mcp,
}

/// Where a bind-card definition comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOrigin {
    Builtin,
    Global,
    Workspace,
}

/// Whether the Agents card shows ALL/SAFE. `fixed` is bind on/off only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionSurface {
    Preset,
    #[default]
    Fixed,
}

/// Declared call surface copied from the tool. Not a user-selectable mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CallSurface {
    #[default]
    Direct,
    Code,
}

/// Runtime state of a capability, kept separate from its definition/binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvailableStatus {
    Ready,
    Warming,
    Disabled,
    Failed,
    Down,
}

/// One currently bindable tool card for this workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableTool {
    pub id: String,
    pub kind: AvailableKind,
    pub origin: ToolOrigin,
    #[serde(default)]
    pub overridden: bool,
    /// Backend declaration of whether this tool has an ALL/SAFE dial.
    #[serde(default)]
    pub permission_surface: PermissionSurface,
    /// `code` tools are reached through `run_code`. The Agents card badges this.
    #[serde(default)]
    pub surface: CallSurface,
    /// Concrete model functions behind this card. MCP members are filled after
    /// the handshake; every other capability declares them statically here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
    /// Human label for a grouped custom suite. Other cards use their id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Role/depth scope; the Agents UI and runtime selection consume the same table.
    #[serde(default)]
    pub scope: BindingScope,
    /// Runtime status projection. Absent for definitions with no engine (builtin/custom).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<AvailableStatus>,
}

/// Workspace-engine configured intent (from engines.json).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolReadiness {
    NotReady,
    Ready,
}

/// JSON schema fragment for a custom tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSchema {
    #[serde(rename = "type")]
    pub schema_type: String,
    #[serde(default)]
    pub properties: Value,
    #[serde(default)]
    pub required: Vec<String>,
}

/// A first-class custom-tool suite. Members reference it by name and inherit
/// its surface and permission rules; the suite owns exactly one binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomSuiteDefinition {
    pub surface: CallSurface,
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<crate::permission::PolicyRule>,
}

/// One custom tool. A standalone tool writes `surface`; a suite member writes
/// `suite` and inherits the suite surface/rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CustomToolDefinition {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub schema: ToolSchema,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    /// Non-empty on a standalone tool: Agents can switch ALL/SAFE. Suite members
    /// must leave this empty; the suite owns the rules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<crate::permission::PolicyRule>,
    /// Standalone call surface. Exactly one of `surface` / `suite` is present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<CallSurface>,
    /// Suite reference. When present, surface and rules are inherited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite: Option<String>,
    /// Code-surface result schema. Required on code, forbidden on direct.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomToolDefinitionWire {
    name: String,
    #[serde(default)]
    description: String,
    schema: ToolSchema,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default = "default_timeout")]
    timeout: u64,
    #[serde(default)]
    rules: Vec<crate::permission::PolicyRule>,
    #[serde(default)]
    surface: Option<CallSurface>,
    #[serde(default)]
    suite: Option<String>,
    #[serde(default)]
    output_schema: Option<Value>,
    /// Legacy fields are parsed only to produce an actionable rewrite error.
    #[serde(default)]
    data_schema: Option<Value>,
    #[serde(default)]
    suite_label: Option<String>,
}

impl<'de> Deserialize<'de> for CustomToolDefinition {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = CustomToolDefinitionWire::deserialize(deserializer)?;
        if wire.data_schema.is_some() {
            return Err(serde::de::Error::custom(
                "`data_schema` was removed; code tools use `output_schema` (object schema, required on code)",
            ));
        }
        if wire.suite_label.is_some() {
            return Err(serde::de::Error::custom(
                "`suite_label` was removed; declare the label once in the suite (`suites.<name>.label`)",
            ));
        }
        let has_suite = wire
            .suite
            .as_deref()
            .is_some_and(|suite| !suite.trim().is_empty());
        match (wire.surface.is_some(), has_suite) {
            (false, false) => {
                return Err(serde::de::Error::custom(
                    "custom tool needs exactly one of `surface` (standalone) or `suite` (member)",
                ));
            }
            (true, true) => {
                return Err(serde::de::Error::custom(
                    "custom tool cannot set both `surface` and `suite`; suite members inherit surface",
                ));
            }
            _ => {}
        }
        Ok(Self {
            name: wire.name,
            description: wire.description,
            schema: wire.schema,
            command: wire.command,
            args: wire.args,
            timeout: wire.timeout,
            rules: wire.rules,
            surface: wire.surface,
            suite: wire.suite.filter(|suite| !suite.trim().is_empty()),
            output_schema: wire.output_schema,
        })
    }
}

fn default_timeout() -> u64 {
    120
}

impl CustomToolDefinition {
    /// Raw standalone surface; suite members return `None` until resolved.
    pub fn declared_surface(&self) -> Option<CallSurface> {
        self.surface
    }

    /// Surface used by runtime/schema code. Suite members are normally resolved
    /// before they reach this point; `None` falls back to direct for legacy
    /// in-memory callers/tests.
    pub fn effective_surface(&self) -> CallSurface {
        self.surface.unwrap_or(CallSurface::Direct)
    }

    pub fn is_suite_member(&self) -> bool {
        self.suite
            .as_deref()
            .is_some_and(|suite| !suite.trim().is_empty())
    }

    pub fn to_json_schema(&self) -> Value {
        serde_json::json!({
            "type": self.schema.schema_type,
            "properties": self.schema.properties,
            "required": self.schema.required,
        })
    }
}

/// One actionable validation problem. The settings API and refresh/validate
/// paths share this shape so the same bad definition does not produce four
/// different messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomToolDiagnostic {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite: Option<String>,
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    pub expected: String,
    pub fix: String,
    pub message: String,
}

#[cfg(test)]
mod custom_tool_definition_tests {
    use super::*;

    fn body(surface: Option<&str>) -> serde_json::Value {
        let mut body = serde_json::json!({
            "name": "probe",
            "command": "echo",
            "schema": { "type": "object", "properties": {}, "required": [] }
        });
        if let Some(surface) = surface {
            body["surface"] = serde_json::Value::String(surface.into());
        }
        body
    }

    #[test]
    fn suite_member_omits_surface_and_old_fields_are_rejected() {
        let member: CustomToolDefinition = serde_json::from_value(serde_json::json!({
            "name": "deck_a",
            "suite": "deck",
            "command": "echo",
            "schema": { "type": "object", "properties": {}, "required": [] }
        }))
        .expect("suite member");
        assert_eq!(member.surface, None);
        assert_eq!(member.suite.as_deref(), Some("deck"));

        let both = serde_json::from_value::<CustomToolDefinition>(serde_json::json!({
            "name": "deck_a",
            "surface": "code",
            "suite": "deck",
            "command": "echo",
            "schema": { "type": "object", "properties": {}, "required": [] }
        }))
        .unwrap_err();
        assert!(both.to_string().contains("both"), "{both}");

        let old = serde_json::from_value::<CustomToolDefinition>(serde_json::json!({
            "name": "old",
            "surface": "code",
            "data_schema": { "type": "object" },
            "command": "echo",
            "schema": { "type": "object", "properties": {}, "required": [] }
        }))
        .unwrap_err();
        assert!(old.to_string().contains("output_schema"), "{old}");

        let labelled = serde_json::from_value::<CustomToolDefinition>(serde_json::json!({
            "name": "old",
            "surface": "direct",
            "suite_label": "Deck",
            "command": "echo",
            "schema": { "type": "object", "properties": {}, "required": [] }
        }))
        .unwrap_err();
        assert!(labelled.to_string().contains("suites"), "{labelled}");
    }

    #[test]
    fn custom_tool_surface_is_explicit_and_accepts_code() {
        let missing: Result<CustomToolDefinition, _> = serde_json::from_value(body(None));
        assert!(
            missing.is_err(),
            "surface must be explicit on a standalone tool"
        );

        let direct: CustomToolDefinition = serde_json::from_value(body(Some("direct"))).unwrap();
        assert_eq!(direct.surface, Some(CallSurface::Direct));

        let code: CustomToolDefinition = serde_json::from_value(body(Some("code"))).unwrap();
        assert_eq!(code.surface, Some(CallSurface::Code));
    }
}

/// MCP transport type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[derive(Default)]
pub enum McpTransport {
    #[serde(rename = "stdio")]
    #[default]
    Stdio,
    #[serde(rename = "remote")]
    Remote {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

/// Default MCP tool-call timeout when the definition omits `timeout`.
pub const DEFAULT_MCP_TOOL_TIMEOUT_SECS: u64 = 60;

fn default_mcp_tool_timeout() -> u64 {
    DEFAULT_MCP_TOOL_TIMEOUT_SECS
}

fn is_default_mcp_tool_timeout(value: &u64) -> bool {
    *value == DEFAULT_MCP_TOOL_TIMEOUT_SECS
}

/// MCP server definition (global layer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerDefinition {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub transport: McpTransport,
    /// Tool-call timeout in seconds. Omitted or `0` means [`DEFAULT_MCP_TOOL_TIMEOUT_SECS`].
    #[serde(default = "default_mcp_tool_timeout")]
    #[serde(skip_serializing_if = "is_default_mcp_tool_timeout")]
    pub timeout: u64,
}

impl Default for McpServerDefinition {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            transport: McpTransport::Stdio,
            timeout: DEFAULT_MCP_TOOL_TIMEOUT_SECS,
        }
    }
}

impl McpServerDefinition {
    pub fn call_timeout_secs(&self) -> u64 {
        if self.timeout == 0 {
            DEFAULT_MCP_TOOL_TIMEOUT_SECS
        } else {
            self.timeout
        }
    }
}

#[cfg(test)]
mod mcp_server_definition_tests {}

/// Deprecated placeholder kept on [`GlobalSettings`] for DB schema stability.
/// Serve inbound auth is **only** `LITECODE_TOKEN`; this field is never loaded or
/// written as a real token (see `global_db::{load,save}_auth`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthSettings {
    #[serde(default)]
    pub token: Option<String>,
}

/// Process-wide log level (global layer; log files land under workspace).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogSettings {
    #[serde(default)]
    pub level: Option<String>,
}

/// Hosted Exa MCP endpoint (not user-configurable).
pub const DEFAULT_WEBSEARCH_MCP_URL: &str = "https://mcp.exa.ai/mcp";

/// Web search backend (global layer; Exa MCP).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchSettings {
    /// Exa API key. Empty uses env `EXA_API_KEY` if set, else anonymous.
    #[serde(default)]
    pub api_key: Option<String>,
}

/// Global settings — provider credentials, agents, extensions, auth, log.
///
/// Provider and model facts live in the provider catalog, never here: this map
/// is the only thing the user owns per provider, and it is the only secret.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GlobalSettings {
    /// Catalog provider id -> API key. Serialization is skipped so a secret can
    /// never leak through a settings payload.
    #[serde(default, skip_serializing)]
    pub provider_credentials: HashMap<String, String>,
    /// Catalog model refs the user switched off, stored as the exception rather
    /// than the rule: a model is selectable unless it is named here, so a model
    /// added to the catalog is on without any write.
    #[serde(default)]
    pub disabled_models: HashSet<String>,
    #[serde(default)]
    pub agents: HashMap<String, AgentProfile>,
    #[serde(default)]
    pub custom_tools: Vec<CustomToolDefinition>,
    #[serde(default)]
    pub custom_suites: HashMap<String, CustomSuiteDefinition>,
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerDefinition>,
    #[serde(default)]
    pub auth: AuthSettings,
    #[serde(default)]
    pub log: LogSettings,
    #[serde(default)]
    pub websearch: WebSearchSettings,
}

impl GlobalSettings {
    /// Top-level field names owned exclusively by the global layer (for partition tests).
    pub const FIELD_NAMES: &'static [&'static str] = &[
        "provider_credentials",
        "disabled_models",
        "agents",
        "custom_tools",
        "custom_suites",
        "mcp_servers",
        "auth",
        "log",
        "websearch",
    ];
}
