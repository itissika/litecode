//! Adapter registry — single source of truth for provider/model config shapes.

use serde::Serialize;
use serde_json::Value;

use crate::config::schema::{
    ADAPTER_DEEPSEEK_RESPONSES, ADAPTER_MIMO_RESPONSES, ADAPTER_OPENAI_RESPONSES, ADAPTER_OPENCODE,
    ModelAdapterConfig, ModelCapability, ProviderAuth, ProviderConnectionConfig,
    ProviderDefinition, ReasoningEffort, ThinkingMode,
};
use crate::llm::provider::LlmProvider;
use crate::types::{LitecodeError, Result};

use super::deepseek_responses::{
    API_MODEL_IDS as DEEPSEEK_API_MODEL_IDS,
    CONTEXT_WINDOW_DEFAULT as DEEPSEEK_CONTEXT_WINDOW_DEFAULT,
    CONTEXT_WINDOW_MAX as DEEPSEEK_CONTEXT_WINDOW_MAX,
    DEFAULT_ENDPOINT as DEEPSEEK_DEFAULT_ENDPOINT, DeepseekResponsesProvider,
};
use super::mimo_responses::{
    API_MODEL_IDS as MIMO_API_MODEL_IDS, CONTEXT_WINDOW_DEFAULT as MIMO_CONTEXT_WINDOW_DEFAULT,
    CONTEXT_WINDOW_MAX as MIMO_CONTEXT_WINDOW_MAX, DEFAULT_ENDPOINT as MIMO_DEFAULT_ENDPOINT,
    MimoResponsesProvider,
};
use super::openai_responses::OpenaiResponsesProvider;
use super::opencode::{DEFAULT_ENDPOINT as OPENCODE_DEFAULT_ENDPOINT, OpencodeProvider};

/// Field type exposed to Settings UI / API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    String,
    Secret,
    Number,
    Boolean,
    Enum,
    StringList,
}

/// One configurable field declared by an adapter.
#[derive(Debug, Clone, Serialize)]
pub struct FieldSchema {
    pub name: &'static str,
    pub label: &'static str,
    #[serde(rename = "type")]
    pub field_type: FieldType,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<&'static [&'static str]>,
}

/// Public adapter descriptor (API / UI).
#[derive(Debug, Clone, Serialize)]
pub struct AdapterDescriptor {
    pub id: &'static str,
    pub label: &'static str,
    pub provider_fields: &'static [FieldSchema],
    pub model_fields: &'static [FieldSchema],
    /// Official host for closed adapters. Open adapters leave this unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_endpoint: Option<&'static str>,
    /// When true, Settings can refresh model ids from `{endpoint}/models`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub remote_model_catalog: bool,
}

const AUTH_OPTIONS: &[&str] = &["bearer", "api_key"];
const THINKING_OPTIONS: &[&str] = &["enabled", "disabled"];
const REASONING_OPTIONS: &[&str] = &["high", "max"];
const CAPABILITY_OPTIONS: &[&str] = &["text", "image", "video", "audio"];

const SHARED_PROVIDER_FIELDS: &[FieldSchema] = &[
    FieldSchema {
        name: "endpoint",
        label: "Endpoint",
        field_type: FieldType::String,
        required: true,
        options: None,
    },
    FieldSchema {
        name: "api_key",
        label: "API Key",
        field_type: FieldType::Secret,
        required: true,
        options: None,
    },
    FieldSchema {
        name: "auth",
        label: "Auth",
        field_type: FieldType::Enum,
        required: true,
        options: Some(AUTH_OPTIONS),
    },
];

const CLOSED_PROVIDER_FIELDS: &[FieldSchema] = &[
    FieldSchema {
        name: "endpoint",
        label: "Endpoint",
        field_type: FieldType::String,
        required: false,
        options: None,
    },
    FieldSchema {
        name: "api_key",
        label: "API Key",
        field_type: FieldType::Secret,
        required: true,
        options: None,
    },
    FieldSchema {
        name: "auth",
        label: "Auth",
        field_type: FieldType::Enum,
        required: true,
        options: Some(AUTH_OPTIONS),
    },
];

const DEEPSEEK_MODEL_FIELDS: &[FieldSchema] = &[
    FieldSchema {
        name: "api_model_id",
        label: "API model id",
        field_type: FieldType::Enum,
        required: true,
        options: Some(DEEPSEEK_API_MODEL_IDS),
    },
    FieldSchema {
        name: "max_tokens",
        label: "Max tokens",
        field_type: FieldType::Number,
        required: false,
        options: None,
    },
    FieldSchema {
        name: "json_output",
        label: "JSON output",
        field_type: FieldType::Boolean,
        required: false,
        options: None,
    },
    FieldSchema {
        name: "capabilities",
        label: "Capabilities",
        field_type: FieldType::StringList,
        required: false,
        options: Some(CAPABILITY_OPTIONS),
    },
];

const MIMO_MODEL_FIELDS: &[FieldSchema] = &[
    FieldSchema {
        name: "api_model_id",
        label: "API model id",
        field_type: FieldType::Enum,
        required: true,
        options: Some(MIMO_API_MODEL_IDS),
    },
    FieldSchema {
        name: "max_tokens",
        label: "Max tokens",
        field_type: FieldType::Number,
        required: false,
        options: None,
    },
    FieldSchema {
        name: "json_output",
        label: "JSON output",
        field_type: FieldType::Boolean,
        required: false,
        options: None,
    },
    FieldSchema {
        name: "capabilities",
        label: "Capabilities",
        field_type: FieldType::StringList,
        required: false,
        options: Some(CAPABILITY_OPTIONS),
    },
];

const SHARED_MODEL_FIELDS: &[FieldSchema] = &[
    FieldSchema {
        name: "api_model_id",
        label: "API model id",
        field_type: FieldType::String,
        required: true,
        options: None,
    },
    FieldSchema {
        name: "context_window",
        label: "Context window",
        field_type: FieldType::Number,
        required: true,
        options: None,
    },
    FieldSchema {
        name: "max_tokens",
        label: "Max tokens",
        field_type: FieldType::Number,
        required: true,
        options: None,
    },
    FieldSchema {
        name: "thinking_mode",
        label: "Thinking mode",
        field_type: FieldType::Enum,
        required: false,
        options: Some(THINKING_OPTIONS),
    },
    FieldSchema {
        name: "reasoning_effort",
        label: "Reasoning effort",
        field_type: FieldType::Enum,
        required: false,
        options: Some(REASONING_OPTIONS),
    },
    FieldSchema {
        name: "json_output",
        label: "JSON output",
        field_type: FieldType::Boolean,
        required: false,
        options: None,
    },
    FieldSchema {
        name: "capabilities",
        label: "Capabilities",
        field_type: FieldType::StringList,
        required: true,
        options: Some(CAPABILITY_OPTIONS),
    },
];

const ADAPTERS: &[AdapterDescriptor] = &[
    AdapterDescriptor {
        id: ADAPTER_OPENAI_RESPONSES,
        label: "OpenAI Responses compatible",
        provider_fields: SHARED_PROVIDER_FIELDS,
        model_fields: SHARED_MODEL_FIELDS,
        default_endpoint: None,
        remote_model_catalog: false,
    },
    AdapterDescriptor {
        id: ADAPTER_DEEPSEEK_RESPONSES,
        label: "DeepSeek",
        provider_fields: CLOSED_PROVIDER_FIELDS,
        model_fields: DEEPSEEK_MODEL_FIELDS,
        default_endpoint: Some(DEEPSEEK_DEFAULT_ENDPOINT),
        remote_model_catalog: false,
    },
    AdapterDescriptor {
        id: ADAPTER_MIMO_RESPONSES,
        label: "MiMo Responses",
        provider_fields: CLOSED_PROVIDER_FIELDS,
        model_fields: MIMO_MODEL_FIELDS,
        default_endpoint: Some(MIMO_DEFAULT_ENDPOINT),
        remote_model_catalog: false,
    },
    AdapterDescriptor {
        id: ADAPTER_OPENCODE,
        label: "OpenCode",
        provider_fields: CLOSED_PROVIDER_FIELDS,
        model_fields: SHARED_MODEL_FIELDS,
        default_endpoint: Some(OPENCODE_DEFAULT_ENDPOINT),
        remote_model_catalog: true,
    },
];

/// All registered adapters (product surface).
pub fn list_adapters() -> &'static [AdapterDescriptor] {
    ADAPTERS
}

pub fn adapter_ids() -> impl Iterator<Item = &'static str> {
    ADAPTERS.iter().map(|a| a.id)
}

pub fn is_known_adapter(id: &str) -> bool {
    ADAPTERS.iter().any(|a| a.id == id)
}

/// Official default host for closed adapters (Settings prefill / empty-endpoint fill).
pub fn closed_default_endpoint(adapter_id: &str) -> Option<&'static str> {
    match adapter_id {
        ADAPTER_DEEPSEEK_RESPONSES => Some(DEEPSEEK_DEFAULT_ENDPOINT),
        ADAPTER_MIMO_RESPONSES => Some(MIMO_DEFAULT_ENDPOINT),
        ADAPTER_OPENCODE => Some(OPENCODE_DEFAULT_ENDPOINT),
        _ => None,
    }
}

pub fn has_remote_model_catalog(adapter_id: &str) -> bool {
    adapter_id == ADAPTER_OPENCODE
}

/// Closed-adapter context budgets: `(default, max)`.
///
/// `None` for open adapters — Max then comes from Settings `context_window`.
pub fn closed_context_windows(adapter_id: &str) -> Option<(usize, usize)> {
    match adapter_id {
        ADAPTER_DEEPSEEK_RESPONSES => {
            Some((DEEPSEEK_CONTEXT_WINDOW_DEFAULT, DEEPSEEK_CONTEXT_WINDOW_MAX))
        }
        ADAPTER_MIMO_RESPONSES => Some((MIMO_CONTEXT_WINDOW_DEFAULT, MIMO_CONTEXT_WINDOW_MAX)),
        _ => None,
    }
}

/// Allowed wire `api_model_id` values for a closed adapter (Settings dropdown).
pub fn closed_api_model_ids(adapter_id: &str) -> Option<&'static [&'static str]> {
    match adapter_id {
        ADAPTER_DEEPSEEK_RESPONSES => Some(DEEPSEEK_API_MODEL_IDS),
        ADAPTER_MIMO_RESPONSES => Some(MIMO_API_MODEL_IDS),
        _ => None,
    }
}

/// Officially supported input modalities per wire model — the adapter-owned
/// "best config" default applied when a model row omits `capabilities`.
///
/// Closed adapters are fully adapter-owned: their modality config is the
/// vendor's official support matrix, so no manual capability setup is needed.
///
/// - `mimo-v2.5`: native full-modality — text/image/video/audio input (see
///   <https://mimo.mi.com/models/zh-CN/mimo-v2.5>).
/// - `mimo-v2.5-pro`: flagship base model — text-only input.
/// - Everything else: text-only.
pub fn adapter_default_capabilities(adapter_id: &str, api_model_id: &str) -> Vec<ModelCapability> {
    match adapter_id {
        ADAPTER_MIMO_RESPONSES if api_model_id == "mimo-v2.5" => vec![
            ModelCapability::Text,
            ModelCapability::Image,
            ModelCapability::Video,
            ModelCapability::Audio,
        ],
        _ => vec![ModelCapability::Text],
    }
}

pub fn descriptor(id: &str) -> Option<&'static AdapterDescriptor> {
    ADAPTERS.iter().find(|a| a.id == id)
}

/// Provider connection is structurally ready (non-empty endpoint + key, known adapter).
pub fn provider_ready(def: &ProviderDefinition) -> bool {
    if !is_known_adapter(&def.adapter_id) {
        return false;
    }
    let endpoint = effective_endpoint(&def.adapter_id, &def.config.endpoint);
    let api_key = def.config.api_key.trim();
    !endpoint.is_empty() && !api_key.is_empty()
}

pub fn validate_provider_config(def: &ProviderDefinition) -> Result<()> {
    if !is_known_adapter(&def.adapter_id) {
        return Err(LitecodeError::Config(format!(
            "unknown adapter_id '{}' for provider '{}'",
            def.adapter_id, def.id
        )));
    }
    if effective_endpoint(&def.adapter_id, &def.config.endpoint).is_empty() {
        return Err(LitecodeError::Config(format!(
            "provider '{}' endpoint is required",
            def.id
        )));
    }
    if def.config.api_key.trim().is_empty() {
        return Err(LitecodeError::Config(format!(
            "provider '{}' api_key is required",
            def.id
        )));
    }
    Ok(())
}

pub fn validate_model_config(
    model_id: &str,
    adapter_id: &str,
    config: &ModelAdapterConfig,
) -> Result<()> {
    if !is_known_adapter(adapter_id) {
        return Err(LitecodeError::Config(format!(
            "unknown adapter_id '{adapter_id}' for model '{model_id}'"
        )));
    }
    let closed = crate::platform_knobs::is_closed_adapter(adapter_id);
    if closed {
        let api = config.api_model_id.trim();
        if api.is_empty() {
            return Err(LitecodeError::Config(format!(
                "model '{model_id}' api_model_id is required"
            )));
        }
        let allowed = closed_api_model_ids(adapter_id).unwrap_or(&[]);
        if !allowed.contains(&api) {
            return Err(LitecodeError::Config(format!(
                "model '{model_id}' api_model_id '{api}' is not in adapter catalog for '{adapter_id}'"
            )));
        }
        return Ok(());
    }
    if config.api_model_id.trim().is_empty() {
        return Err(LitecodeError::Config(format!(
            "model '{model_id}' api_model_id is required"
        )));
    }
    if config.context_window == 0 {
        return Err(LitecodeError::Config(format!(
            "model '{model_id}' context_window must be > 0"
        )));
    }
    if config.max_tokens == 0 {
        return Err(LitecodeError::Config(format!(
            "model '{model_id}' max_tokens must be > 0"
        )));
    }
    if config.capabilities.is_empty() {
        return Err(LitecodeError::Config(format!(
            "model '{model_id}' capabilities must not be empty"
        )));
    }
    Ok(())
}

fn effective_endpoint(adapter_id: &str, endpoint: &str) -> String {
    let trimmed = endpoint.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    closed_default_endpoint(adapter_id)
        .unwrap_or("")
        .to_string()
}

/// Parse provider connection JSON into typed config (adapter-owned shape).
pub fn parse_provider_config(adapter_id: &str, value: &Value) -> Result<ProviderConnectionConfig> {
    if !is_known_adapter(adapter_id) {
        return Err(LitecodeError::Config(format!(
            "unknown adapter_id '{adapter_id}'"
        )));
    }
    let endpoint = effective_endpoint(
        adapter_id,
        value.get("endpoint").and_then(|v| v.as_str()).unwrap_or(""),
    );
    let api_key = value
        .get("api_key")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let auth = match value
        .get("auth")
        .and_then(|v| v.as_str())
        .unwrap_or("bearer")
    {
        "api_key" => ProviderAuth::ApiKey,
        "bearer" => ProviderAuth::Bearer,
        other => {
            return Err(LitecodeError::Config(format!(
                "unknown auth mode '{other}'"
            )));
        }
    };
    Ok(ProviderConnectionConfig {
        endpoint,
        api_key,
        auth,
    })
}

/// Parse model config JSON into typed config (adapter-owned shape).
pub fn parse_model_config(adapter_id: &str, value: &Value) -> Result<ModelAdapterConfig> {
    if !is_known_adapter(adapter_id) {
        return Err(LitecodeError::Config(format!(
            "unknown adapter_id '{adapter_id}'"
        )));
    }
    let api_model_id = value
        .get("api_model_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let context_window = value
        .get("context_window")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let max_tokens = value
        .get("max_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let thinking_mode = match value.get("thinking_mode").and_then(|v| v.as_str()) {
        None | Some("") => None,
        Some("enabled") => Some(ThinkingMode::Enabled),
        Some("disabled") => Some(ThinkingMode::Disabled),
        Some(other) => {
            return Err(LitecodeError::Config(format!(
                "unknown thinking_mode '{other}'"
            )));
        }
    };
    let reasoning_effort = match value.get("reasoning_effort").and_then(|v| v.as_str()) {
        None | Some("") => None,
        Some("high") => Some(ReasoningEffort::High),
        Some("max") => Some(ReasoningEffort::Max),
        Some(other) => {
            return Err(LitecodeError::Config(format!(
                "unknown reasoning_effort '{other}'"
            )));
        }
    };
    let json_output = value
        .get("json_output")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let capabilities = match value.get("capabilities") {
        Some(Value::Array(arr)) => {
            let mut caps = Vec::new();
            for item in arr {
                let Some(s) = item.as_str() else {
                    return Err(LitecodeError::Config(
                        "capabilities entries must be strings".into(),
                    ));
                };
                let Some(cap) = ModelCapability::parse(s) else {
                    return Err(LitecodeError::Config(format!("unknown capability '{s}'")));
                };
                caps.push(cap);
            }
            caps
        }
        None => adapter_default_capabilities(adapter_id, &api_model_id),
        _ => {
            return Err(LitecodeError::Config(
                "capabilities must be a string array".into(),
            ));
        }
    };
    Ok(ModelAdapterConfig {
        api_model_id,
        context_window,
        max_tokens,
        thinking_mode,
        reasoning_effort,
        json_output,
        capabilities,
    })
}

/// Build an LLM client from a provider row (adapter_id selects the wire).
pub fn build_client(def: &ProviderDefinition) -> Result<Box<dyn LlmProvider>> {
    validate_provider_config(def)?;
    let endpoint = effective_endpoint(&def.adapter_id, &def.config.endpoint);
    let auth = def.config.auth;
    match def.adapter_id.as_str() {
        ADAPTER_OPENAI_RESPONSES => Ok(Box::new(OpenaiResponsesProvider::new(endpoint, auth)?)),
        ADAPTER_DEEPSEEK_RESPONSES => Ok(Box::new(DeepseekResponsesProvider::new(endpoint, auth)?)),
        ADAPTER_MIMO_RESPONSES => Ok(Box::new(MimoResponsesProvider::new(endpoint, auth)?)),
        ADAPTER_OPENCODE => Ok(Box::new(OpencodeProvider::new(endpoint, auth)?)),
        other => Err(LitecodeError::Config(format!(
            "unknown adapter_id '{other}' for provider '{}'",
            def.id
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{
        ADAPTER_DEEPSEEK_RESPONSES, ADAPTER_MIMO_RESPONSES, ADAPTER_OPENAI_RESPONSES,
        ADAPTER_OPENCODE,
    };

    #[test]
    fn mimo_v25_defaults_to_full_modality() {
        let caps = adapter_default_capabilities(ADAPTER_MIMO_RESPONSES, "mimo-v2.5");
        assert_eq!(
            caps,
            vec![
                ModelCapability::Text,
                ModelCapability::Image,
                ModelCapability::Video,
                ModelCapability::Audio,
            ]
        );
    }

    #[test]
    fn mimo_pro_and_other_adapters_default_to_text() {
        assert_eq!(
            adapter_default_capabilities(ADAPTER_MIMO_RESPONSES, "mimo-v2.5-pro"),
            vec![ModelCapability::Text]
        );
        assert_eq!(
            adapter_default_capabilities(ADAPTER_DEEPSEEK_RESPONSES, "deepseek-v4-flash"),
            vec![ModelCapability::Text]
        );
        assert_eq!(
            adapter_default_capabilities(ADAPTER_OPENAI_RESPONSES, "gpt-4o"),
            vec![ModelCapability::Text]
        );
    }

    #[test]
    fn parse_model_config_defaults_capabilities_per_wire_model() {
        let mimo = parse_model_config(
            ADAPTER_MIMO_RESPONSES,
            &serde_json::json!({ "api_model_id": "mimo-v2.5" }),
        )
        .unwrap();
        assert_eq!(
            mimo.capabilities,
            vec![
                ModelCapability::Text,
                ModelCapability::Image,
                ModelCapability::Video,
                ModelCapability::Audio,
            ]
        );

        let pro = parse_model_config(
            ADAPTER_MIMO_RESPONSES,
            &serde_json::json!({ "api_model_id": "mimo-v2.5-pro" }),
        )
        .unwrap();
        assert_eq!(pro.capabilities, vec![ModelCapability::Text]);
    }

    #[test]
    fn parse_model_config_keeps_explicit_capabilities() {
        let cfg = parse_model_config(
            ADAPTER_MIMO_RESPONSES,
            &serde_json::json!({
                "api_model_id": "mimo-v2.5",
                "capabilities": ["text", "image"],
            }),
        )
        .unwrap();
        assert_eq!(
            cfg.capabilities,
            vec![ModelCapability::Text, ModelCapability::Image]
        );
    }

    #[test]
    fn closed_adapters_expose_official_default_endpoints() {
        let deepseek = list_adapters()
            .iter()
            .find(|a| a.id == ADAPTER_DEEPSEEK_RESPONSES)
            .unwrap();
        assert_eq!(deepseek.default_endpoint, Some("https://api.deepseek.com"));
        let mimo = list_adapters()
            .iter()
            .find(|a| a.id == ADAPTER_MIMO_RESPONSES)
            .unwrap();
        assert_eq!(mimo.default_endpoint, Some("https://api.xiaomimimo.com/v1"));
        let openai = list_adapters()
            .iter()
            .find(|a| a.id == ADAPTER_OPENAI_RESPONSES)
            .unwrap();
        assert_eq!(openai.default_endpoint, None);
        let opencode = list_adapters()
            .iter()
            .find(|a| a.id == ADAPTER_OPENCODE)
            .unwrap();
        assert_eq!(opencode.default_endpoint, Some(OPENCODE_DEFAULT_ENDPOINT));
        assert!(opencode.remote_model_catalog);
        assert!(has_remote_model_catalog(ADAPTER_OPENCODE));
    }

    #[test]
    fn parse_provider_config_fills_closed_default_endpoint() {
        let cfg = parse_provider_config(
            ADAPTER_DEEPSEEK_RESPONSES,
            &serde_json::json!({ "api_key": "sk-test" }),
        )
        .unwrap();
        assert_eq!(cfg.endpoint, "https://api.deepseek.com");

        let custom = parse_provider_config(
            ADAPTER_MIMO_RESPONSES,
            &serde_json::json!({
                "endpoint": "https://token-plan-cn.xiaomimimo.com/v1",
                "api_key": "tp-test"
            }),
        )
        .unwrap();
        assert_eq!(custom.endpoint, "https://token-plan-cn.xiaomimimo.com/v1");

        let open = parse_provider_config(
            ADAPTER_OPENAI_RESPONSES,
            &serde_json::json!({ "api_key": "sk-test" }),
        )
        .unwrap();
        assert_eq!(open.endpoint, "");

        let zen = parse_provider_config(
            ADAPTER_OPENCODE,
            &serde_json::json!({ "api_key": "sk-test" }),
        )
        .unwrap();
        assert_eq!(zen.endpoint, OPENCODE_DEFAULT_ENDPOINT);

        let go = parse_provider_config(
            ADAPTER_OPENCODE,
            &serde_json::json!({
                "endpoint": "https://opencode.ai/zen/go/v1",
                "api_key": "sk-test"
            }),
        )
        .unwrap();
        assert_eq!(go.endpoint, "https://opencode.ai/zen/go/v1");
    }
}
