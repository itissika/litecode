//! Raw provider-catalog.toml contract.
//!
//! The types here only describe the file. Everything the runtime consumes lives
//! in [super::resolve], built once per process from a validated raw catalog.
//!
//! Unknown fields, unknown enum values, duplicate ids and dangling references
//! are hard errors: the product never guesses a value that is not written down.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Catalog structure version supported by this build.
pub const SUPPORTED_VERSION: u32 = 1;

/// Wire protocol family a provider or model speaks.
///
/// Closed set: adding a variant means writing a codec that produces the
/// authority shape. This enum is what decouples provider count from codec count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointKind {
    /// OpenAI Responses - the authority shape, zero conversion.
    Responses,
    /// OpenAI Chat Completions - one conversion pass.
    ChatCompletions,
}

impl EndpointKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Responses => "responses",
            Self::ChatCompletions => "chat_completions",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "responses" => Some(Self::Responses),
            "chat_completions" => Some(Self::ChatCompletions),
            _ => None,
        }
    }

    /// Every endpoint kind, in codec-factory order.
    pub const ALL: &'static [EndpointKind] = &[Self::Responses, Self::ChatCompletions];

    /// Request path appended to the catalog base URL.
    pub fn request_path(self) -> &'static str {
        match self {
            Self::Responses => "/responses",
            Self::ChatCompletions => "/chat/completions",
        }
    }

    /// Request URL for a base endpoint (trailing slashes tolerated).
    pub fn request_url(self, endpoint: &str) -> String {
        format!(
            "{}{}",
            endpoint.trim_end_matches('/'),
            self.request_path()
        )
    }

    /// Input modalities this codec actually serializes.
    ///
    /// A declared modality that is not implemented is a catalog load error, not
    /// a silent intersection at request time.
    pub fn supports_modality(self, modality: Modality) -> bool {
        match self {
            Self::Responses => true,
            Self::ChatCompletions => modality == Modality::Text,
        }
    }
}

/// How the API key is presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    /// `Authorization: Bearer <key>`.
    Bearer,
    /// `api-key: <key>`.
    ApiKey,
}

impl AuthKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bearer => "bearer",
            Self::ApiKey => "api_key",
        }
    }
}

/// Input modality a model accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    Text,
    Image,
    Video,
    Audio,
    /// Responses `input_file` parts. No Chat Completions host accepts them.
    Pdf,
}

impl Modality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Pdf => "pdf",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "text" => Some(Self::Text),
            "image" => Some(Self::Image),
            "video" => Some(Self::Video),
            "audio" => Some(Self::Audio),
            "pdf" => Some(Self::Pdf),
            _ => None,
        }
    }

    pub const ALL: &'static [Modality] = &[
        Self::Text,
        Self::Image,
        Self::Video,
        Self::Audio,
        Self::Pdf,
    ];
}

/// Named response repair implemented by a codec.
///
/// Each variant exists because one vendor's payloads cannot be deserialized
/// into the authority types as-is. A variant is a promise that the codec knows
/// how to read that shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsagePatch {
    /// Payload already matches the authority shape.
    #[default]
    None,
    /// MiMo: fill empty `usage.*_tokens_details` objects before deserialization.
    FillEmptyTokenDetails,
    /// Vendor `max` effort literal: normalize it to authority `xhigh` (the
    /// authority enum has none), and fill the token-detail and total-token
    /// members the same payloads omit.
    MapMaxEffortToXhigh,
}

impl UsagePatch {
    pub const ALL: &'static [UsagePatch] = &[
        Self::None,
        Self::FillEmptyTokenDetails,
        Self::MapMaxEffortToXhigh,
    ];
}

/// Named protocol behavior that is not universal.
///
/// The escape hatch is data + a codec implementation - never a provider id
/// check inside the codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderQuirk {
    /// DeepSeek: thinking mode ignores temperature, so omit it while a tier is sent.
    OmitTemperatureWhenThinking,
    /// Ark Coding (Doubao Responses): send `thinking.type` next to
    /// `reasoning.effort`; the low tier disables thinking.
    ThinkingTypeSwitch,
    /// DeepSeek / MiMo: a thinking request that carries tools must pass every
    /// earlier assistant turn's reasoning text back, so missing reasoning is
    /// synthesized before the input is serialized.
    ReasoningReplay,
}

impl ProviderQuirk {
    pub const ALL: &'static [ProviderQuirk] = &[
        Self::OmitTemperatureWhenThinking,
        Self::ThinkingTypeSwitch,
        Self::ReasoningReplay,
    ];
}

/// Which field a Chat Completions replay writes reasoning back into.
///
/// Only keys the codec implements are accepted; a typo is a load error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningKey {
    #[default]
    ReasoningContent,
}

impl ReasoningKey {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReasoningContent => "reasoning_content",
        }
    }

    pub const ALL: &'static [ReasoningKey] = &[Self::ReasoningContent];
}

/// Platform tier -> vendor literal. All three slots are required: a partially
/// filled mapping is an error, not an intention.
///
/// Vendors with fewer than three levels write the clamp explicitly
/// (`low = "high"` for a model whose floor is high).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReasoningTiers {
    /// Literal sent for Thinking OFF (compaction). Absent = send no control at
    /// all, which is correct for vendors whose default is thinking off and
    /// wrong for vendors whose default is thinking on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off: Option<String>,
    pub low: String,
    pub medium: String,
    pub high: String,
}

/// Model-level reasoning declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawReasoning {
    /// Tier mapping. Absent = inherit the provider's tiers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiers: Option<ReasoningTiers>,
    /// Replay write-back key (Chat Completions codec).
    #[serde(default)]
    pub key: ReasoningKey,
}

/// One catalog provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawProvider {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub visible: bool,
    /// Base URL only - the path comes from `endpoint_type`.
    pub endpoint: String,
    #[serde(default = "default_endpoint_kind")]
    pub endpoint_type: EndpointKind,
    #[serde(default = "default_auth")]
    pub auth: AuthKind,
    /// Reasoning tiers shared by the whole catalog, for vendors that normalize
    /// one effort vocabulary. Model-level tiers win.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiers: Option<ReasoningTiers>,
    /// Named non-universal behaviors this provider's codec path opts into.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quirks: Vec<ProviderQuirk>,
    /// Extra request headers. `{{session_id}}` is the only allowed placeholder.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

/// One wire model. Every selectable model has an explicit entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawModel {
    /// Wire model id, copied verbatim.
    pub id: String,
    /// Owning provider id.
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Overrides the provider base URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Overrides the provider protocol.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_type: Option<EndpointKind>,
    /// Default context budget.
    #[serde(default = "default_context_window")]
    pub context_window: usize,
    /// Upper bound for context_mode = max. Absent means `context_window`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_max: Option<usize>,
    #[serde(default = "default_max_output")]
    pub max_output: u32,
    #[serde(default = "default_modalities")]
    pub modalities: Vec<Modality>,
    /// `false` models are marked in the agent picker instead of failing later.
    #[serde(default = "default_true")]
    pub tool_call: bool,
    #[serde(default)]
    pub json_output: bool,
    /// `false` = never send `temperature`.
    #[serde(default = "default_true")]
    pub temperature: bool,
    /// `stream_options.include_usage`.
    #[serde(default = "default_true")]
    pub stream_usage: bool,
    #[serde(default)]
    pub usage_patch: UsagePatch,
    /// Absent = the model accepts no reasoning control and one is never sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<RawReasoning>,
    /// Escape hatch: merged into the request body. Reserved codec keys are rejected.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub extra_body: Map<String, Value>,
}

/// The whole file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawCatalog {
    pub version: u32,
    #[serde(default)]
    pub providers: Vec<RawProvider>,
    #[serde(default)]
    pub models: Vec<RawModel>,
}

fn default_true() -> bool {
    true
}

fn default_auth() -> AuthKind {
    AuthKind::Bearer
}

fn default_endpoint_kind() -> EndpointKind {
    EndpointKind::ChatCompletions
}

fn default_modalities() -> Vec<Modality> {
    vec![Modality::Text]
}

fn default_context_window() -> usize {
    256_000
}

fn default_max_output() -> u32 {
    32_768
}

/// Request-body keys a codec owns; `extra_body` must not redefine them.
pub const RESERVED_BODY_KEYS: &[&str] = &[
    "model",
    "input",
    "instructions",
    "messages",
    "tools",
    "tool_choice",
    "stream",
    "stream_options",
    "max_output_tokens",
    "max_tokens",
    "max_completion_tokens",
    "temperature",
    "reasoning",
    "reasoning_effort",
    "thinking",
    "text",
    "response_format",
];

/// Header names a codec owns; catalog headers must not shadow them.
pub const RESERVED_HEADER_NAMES: &[&str] =
    &["authorization", "api-key", "x-api-key", "content-type", "accept", "user-agent"];

/// The only template placeholder allowed in catalog header values.
pub const SESSION_ID_PLACEHOLDER: &str = "{{session_id}}";
