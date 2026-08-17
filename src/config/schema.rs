//! Global-layer configuration schema (`GlobalSettings`).
//!
//! Fields here are disjoint from [`super::resolved::WorkspaceState`] per CONFIG.md §1.4.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Registered adapter ids (must match `llm::adapter::registry`).
pub const ADAPTER_OPENAI_RESPONSES: &str = "openai_responses";
pub const ADAPTER_DEEPSEEK_RESPONSES: &str = "deepseek_responses";
pub const ADAPTER_MIMO_RESPONSES: &str = "mimo_responses";
pub const ADAPTER_OPENCODE: &str = "opencode";

/// LLM provider auth mode for HTTP requests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuth {
    #[default]
    Bearer,
    ApiKey,
}

/// Adapter-owned provider connection shape (serialized as `config_json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConnectionConfig {
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub auth: ProviderAuth,
}

/// Provider row — adapter instance link (`providers.<id>`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDefinition {
    #[serde(default)]
    pub id: String,
    /// Registry adapter id (`openai_responses` / `deepseek_responses` / `mimo_responses`).
    pub adapter_id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub config: ProviderConnectionConfig,
}
/// Model input modality capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCapability {
    Text,
    Image,
    Video,
    Audio,
}

impl ModelCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "text" => Some(Self::Text),
            "image" => Some(Self::Image),
            "video" => Some(Self::Video),
            "audio" => Some(Self::Audio),
            _ => None,
        }
    }
}

/// Adapter-owned model config shape (serialized as `config_json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelAdapterConfig {
    pub api_model_id: String,
    pub context_window: usize,
    pub max_tokens: u32,
    #[serde(default)]
    pub thinking_mode: Option<ThinkingMode>,
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
    #[serde(default)]
    pub json_output: bool,
    #[serde(default = "default_capabilities")]
    pub capabilities: Vec<ModelCapability>,
}

fn default_capabilities() -> Vec<ModelCapability> {
    vec![ModelCapability::Text]
}

impl Default for ModelAdapterConfig {
    fn default() -> Self {
        Self {
            api_model_id: String::new(),
            context_window: 0,
            max_tokens: 0,
            thinking_mode: None,
            reasoning_effort: None,
            json_output: false,
            capabilities: default_capabilities(),
        }
    }
}

/// Model row — adapter instance + provider link (`models.<id>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDefinition {
    pub id: String,
    /// Must match the linked provider's `adapter_id`.
    pub adapter_id: String,
    /// Link to a provider row (same adapter); not the capability source.
    pub provider_ref: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub config: ModelAdapterConfig,
}

impl ModelDefinition {
    pub fn api_model_id(&self) -> &str {
        &self.config.api_model_id
    }

    pub fn context_window(&self) -> usize {
        self.config.context_window
    }

    pub fn max_tokens(&self) -> u32 {
        self.config.max_tokens
    }

    pub fn thinking_mode(&self) -> Option<ThinkingMode> {
        self.config.thinking_mode
    }

    pub fn reasoning_effort(&self) -> Option<ReasoningEffort> {
        self.config.reasoning_effort
    }

    pub fn json_output(&self) -> bool {
        self.config.json_output
    }

    pub fn capabilities(&self) -> &[ModelCapability] {
        &self.config.capabilities
    }

    pub fn supports(&self, cap: &str) -> bool {
        self.config.capabilities.iter().any(|c| c.as_str() == cap)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingMode {
    Enabled,
    Disabled,
}

impl ThinkingMode {
    pub fn as_api_str(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    High,
    Max,
}

impl ReasoningEffort {
    pub fn as_api_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Max => "max",
        }
    }
}

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
    #[serde(default)]
    pub policy: crate::permission::ToolPolicy,
    #[serde(default)]
    pub path_mode: crate::permission::BindingPathMode,
    #[serde(default)]
    pub last_applied_preset: Option<ToolPreset>,
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
pub const PROTECTED_AGENT_IDS: &[&str] = &["default", "compaction"];

/// Built-in subagent orchestration tools (primary-only bindings).
pub const SUBAGENT_SERIES_TOOL_IDS: &[&str] = &["subagent_launch"];

fn default_temperature() -> f64 {
    0.7
}

fn default_max_steps() -> u32 {
    50
}

/// Tool catalog tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTier {
    Core,
    Optional,
    Custom,
    Mcp,
}

/// Init scope for catalog tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InitScope {
    None,
    Global,
    Workspace,
}

/// Catalog readiness (global layer; workspace-scoped readiness lives in workspace state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolReadiness {
    NotReady,
    Ready,
}

/// Tool catalog entry (global layer). Readiness lives in [`super::RuntimeCatalogState`] (global)
/// or [`super::resolved::WorkspaceState::workspace_tool_readiness`] (workspace).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCatalogEntry {
    pub id: String,
    pub tier: ToolTier,
    pub init_scope: InitScope,
    pub catalog_enabled: bool,
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

/// Custom tool definition (global layer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

fn default_timeout() -> u64 {
    120
}

impl CustomToolDefinition {
    pub fn to_json_schema(&self) -> Value {
        serde_json::json!({
            "type": self.schema.schema_type,
            "properties": self.schema.properties,
            "required": self.schema.required,
        })
    }
}

#[cfg(test)]
mod custom_tool_definition_tests {
    use super::*;

    #[test]
    fn custom_tool_json_schema() {
        let tool = CustomToolDefinition {
            name: "t".into(),
            description: String::new(),
            schema: ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({"key": {"type": "string"}}),
                required: vec!["key".into()],
            },
            command: "cmd".into(),
            args: vec![],
            timeout: 120,
        };
        let schema = tool.to_json_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["key"]["type"], "string");
        assert_eq!(schema["required"][0], "key");
    }
}

/// MCP transport type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransport {
    #[serde(rename = "stdio")]
    Stdio,
    #[serde(rename = "remote")]
    Remote {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

impl Default for McpTransport {
    fn default() -> Self {
        McpTransport::Stdio
    }
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
}

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

/// Hosted Exa MCP endpoint (anonymous free tier; optional `EXA_API_KEY` for higher limits).
pub const DEFAULT_WEBSEARCH_MCP_URL: &str = "https://mcp.exa.ai/mcp";

/// Web search backend (global layer; Exa MCP by default).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchSettings {
    /// Override Exa MCP URL; empty uses [`DEFAULT_WEBSEARCH_MCP_URL`].
    #[serde(default)]
    pub search_endpoint: Option<String>,
}

impl WebSearchSettings {
    /// Settings override, else hosted Exa MCP default.
    pub fn resolved_endpoint(&self) -> Option<String> {
        self.search_endpoint
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| Some(DEFAULT_WEBSEARCH_MCP_URL.to_string()))
    }
}

/// Global settings — providers, models, catalog, agents, extensions, auth, log.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GlobalSettings {
    #[serde(default)]
    pub providers: HashMap<String, ProviderDefinition>,
    #[serde(default)]
    pub models: HashMap<String, ModelDefinition>,
    #[serde(default)]
    pub tool_catalog: HashMap<String, ToolCatalogEntry>,
    #[serde(default)]
    pub agents: HashMap<String, AgentProfile>,
    #[serde(default)]
    pub custom_tools: Vec<CustomToolDefinition>,
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
        "providers",
        "models",
        "tool_catalog",
        "agents",
        "custom_tools",
        "mcp_servers",
        "auth",
        "log",
        "websearch",
    ];
}
