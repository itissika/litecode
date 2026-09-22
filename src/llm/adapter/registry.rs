//! Adapter registry — single source of truth for provider/model config shapes.

use serde::Serialize;
use serde_json::Value;

use crate::config::schema::{
    ADAPTER_ARK_CODING, ADAPTER_COMMANDCODE, ADAPTER_DEEPSEEK_RESPONSES, ADAPTER_MIMO_RESPONSES,
    ADAPTER_OPENAI_RESPONSES, ADAPTER_OPENCODE, ModelAdapterConfig, ModelCapability,
    ModelDefinition, ProviderAuth, ProviderConnectionConfig, ProviderDefinition,
};
use crate::llm::provider::LlmProvider;
use crate::types::{LitecodeError, Result};

use super::ark_coding::{ArkCodingProvider, DEFAULT_ENDPOINT as ARK_DEFAULT_ENDPOINT};
use super::commandcode::{
    DEFAULT_ENDPOINT as COMMANDCODE_DEFAULT_ENDPOINT, CommandcodeProvider,
};
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

/// Command Code documents Bearer as the only auth for `/chat/completions`
/// (`x-api-key` is Anthropic-SDK-on-`/messages` only), so the Settings form
/// must not offer the api_key mode — it could only produce a 401.
const COMMANDCODE_AUTH_OPTIONS: &[&str] = &["bearer"];

const COMMANDCODE_PROVIDER_FIELDS: &[FieldSchema] = &[
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
        options: Some(COMMANDCODE_AUTH_OPTIONS),
    },
];

const DEEPSEEK_MODEL_FIELDS: &[FieldSchema] = &[
    FieldSchema {
        name: "api_model_id",
        label: "API model id",
        field_type: FieldType::String,
        required: true,
        options: None,
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
        // Live ids come from the remote `/models` catalog; no static enum.
        field_type: FieldType::String,
        required: true,
        options: None,
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
        // Official GET {endpoint}/models — same OpenAI list shape as OpenCode.
        remote_model_catalog: true,
    },
    AdapterDescriptor {
        id: ADAPTER_MIMO_RESPONSES,
        label: "MiMo Responses",
        provider_fields: CLOSED_PROVIDER_FIELDS,
        model_fields: MIMO_MODEL_FIELDS,
        default_endpoint: Some(MIMO_DEFAULT_ENDPOINT),
        // Official GET {endpoint}/v1/models — same OpenAI list shape as
        // OpenCode (`https://mimo.mi.com/docs/zh-CN/api/model/list-models`).
        remote_model_catalog: true,
    },
    AdapterDescriptor {
        id: ADAPTER_OPENCODE,
        label: "OpenCode",
        provider_fields: CLOSED_PROVIDER_FIELDS,
        model_fields: SHARED_MODEL_FIELDS,
        default_endpoint: Some(OPENCODE_DEFAULT_ENDPOINT),
        remote_model_catalog: true,
    },
    AdapterDescriptor {
        id: ADAPTER_ARK_CODING,
        label: "Ark Coding Plan",
        provider_fields: CLOSED_PROVIDER_FIELDS,
        model_fields: SHARED_MODEL_FIELDS,
        default_endpoint: Some(ARK_DEFAULT_ENDPOINT),
        // Coding Plan has no dedicated /models SKU list; GET {base}/models
        // returns the general inference catalog and is not usable here.
        remote_model_catalog: false,
    },
    AdapterDescriptor {
        id: ADAPTER_COMMANDCODE,
        label: "Command Code",
        provider_fields: COMMANDCODE_PROVIDER_FIELDS,
        model_fields: SHARED_MODEL_FIELDS,
        default_endpoint: Some(COMMANDCODE_DEFAULT_ENDPOINT),
        // Official GET {endpoint}/models — same OpenAI list shape as OpenCode.
        // Anthropic ids are filtered out (see `catalog_supported_ids`).
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
        ADAPTER_ARK_CODING => Some(ARK_DEFAULT_ENDPOINT),
        ADAPTER_COMMANDCODE => Some(COMMANDCODE_DEFAULT_ENDPOINT),
        _ => None,
    }
}

pub fn has_remote_model_catalog(adapter_id: &str) -> bool {
    ADAPTERS
        .iter()
        .find(|a| a.id == adapter_id)
        .is_some_and(|a| a.remote_model_catalog)
}

/// Command Code serves Anthropic models only on the sibling `/messages`
/// endpoint and rejects them on `/chat/completions` with HTTP 400 ("wrong
/// endpoint for the model"), so ids matching this predicate cannot work on the
/// adapter's wire. Catalog ids are un-prefixed for Anthropic (`claude-*`) and
/// vendor-prefixed for everything else (`deepseek/...`).
fn is_messages_only_model(api_model_id: &str) -> bool {
    let lower = api_model_id.trim().to_ascii_lowercase();
    lower.starts_with("anthropic/") || lower.contains("claude")
}

/// MiMo lists its speech models (`*-asr`, `*-tts*`) on the same `/models`
/// endpoint but serves them on dedicated ASR/TTS endpoints the Responses
/// adapter does not implement, so the Settings picker must not offer them.
fn is_speech_only_model(api_model_id: &str) -> bool {
    let lower = api_model_id.trim().to_ascii_lowercase();
    lower.ends_with("-asr") || lower.contains("-tts")
}

/// Filter a fetched `/models` catalog down to ids this adapter can actually
/// serve on its wire.
pub fn catalog_supported_ids(adapter_id: &str, ids: Vec<String>) -> Vec<String> {
    match adapter_id {
        ADAPTER_COMMANDCODE => ids
            .into_iter()
            .filter(|id| !is_messages_only_model(id))
            .collect(),
        ADAPTER_MIMO_RESPONSES => ids
            .into_iter()
            .filter(|id| !is_speech_only_model(id))
            .collect(),
        _ => ids,
    }
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

/// Static fallback wire ids for closed adapters. Both closed adapters fetch a
/// remote catalog, so Settings validation no longer treats this as an
/// allowlist — it only backs surfaces without a live `/models` list.
pub fn closed_api_model_ids(adapter_id: &str) -> Option<&'static [&'static str]> {
    match adapter_id {
        ADAPTER_DEEPSEEK_RESPONSES => Some(DEEPSEEK_API_MODEL_IDS),
        ADAPTER_MIMO_RESPONSES => Some(MIMO_API_MODEL_IDS),
        _ => None,
    }
}

/// MiMo wire ids whose vendor matrix is full-modality input (text / image /
/// video / audio). The v2.6 line is the current flagship; `mimo-v2.5` stays
/// selectable until its 2026-10-21 retirement.
const MIMO_FULL_MODALITY_IDS: &[&str] = &[
    "mimo-v2.5",
    "mimo-v2.6-pro",
    "mimo-v2.6-flash",
    "mimo-v2.6-pro-ultraspeed",
];

/// Officially supported input modalities per wire model — the adapter-owned
/// "best config" default applied when a model row omits `capabilities`.
///
/// Closed adapters are fully adapter-owned: their modality config is the
/// vendor's official support matrix, so no manual capability setup is needed.
///
/// - MiMo full-modality line (`mimo-v2.5`, `mimo-v2.6-pro`, `mimo-v2.6-flash`,
///   `mimo-v2.6-pro-ultraspeed`): native text/image/video/audio input — the
///   per-model pages list 输入模态 文本、图像、视频、音频
///   (see <https://mimo.mi.com/models/zh-CN/mimo-v2.6-pro>).
/// - `mimo-v2.5-pro`: non-multimodal base model — text-only input.
/// - DeepSeek **Flash line** (`deepseek-flash`, plus the retired
///   `deepseek-v4-flash` / `deepseek-v4-flash-vision-exp` aliases that are
///   served by the same V4.1-Flash model): text + image. The **Pro line**
///   (`deepseek-v4-pro`) does not accept images. `/models` does not return
///   modalities, so the line is decided from the id.
///   <https://api-docs.deepseek.com/guides/vision>
/// - Ark Coding Plan `doubao-seed-2.1-turbo`: text + image (Coding Plan `/responses` P2).
/// - Command Code: every model is text-only on LiteCode's Chat Completions
///   codec, which cannot serialize image parts — declaring image would make
///   `validate_llm_input_capabilities` admit a screenshot the wire then drops.
/// - Everything else: text-only.
pub fn adapter_default_capabilities(adapter_id: &str, api_model_id: &str) -> Vec<ModelCapability> {
    match adapter_id {
        ADAPTER_MIMO_RESPONSES if MIMO_FULL_MODALITY_IDS.contains(&api_model_id.trim()) => vec![
            ModelCapability::Text,
            ModelCapability::Image,
            ModelCapability::Video,
            ModelCapability::Audio,
        ],
        ADAPTER_DEEPSEEK_RESPONSES if deepseek_accepts_images(api_model_id) => {
            vec![ModelCapability::Text, ModelCapability::Image]
        }
        ADAPTER_ARK_CODING if api_model_id.eq_ignore_ascii_case("doubao-seed-2.1-turbo") => {
            vec![ModelCapability::Text, ModelCapability::Image]
        }
        _ => vec![ModelCapability::Text],
    }
}

/// Official DeepSeek vision support: the **Flash** line accepts images —
/// `deepseek-flash`, plus the retired `deepseek-v4-flash` /
/// `deepseek-v4-flash-vision-exp` aliases that resolve to the same V4.1-Flash
/// model. The **Pro** line does not. Unknown ids stay text-only so a typo
/// cannot admit a screenshot the vendor then rejects with 400.
fn deepseek_accepts_images(api_model_id: &str) -> bool {
    let id = api_model_id.trim().to_ascii_lowercase();
    id.contains("flash") || id.contains("vision")
}

/// Adapters whose modality matrix is overwritten on Settings read/write.
pub fn adapter_owns_modality_matrix(adapter_id: &str) -> bool {
    matches!(
        adapter_id,
        ADAPTER_DEEPSEEK_RESPONSES
            | ADAPTER_MIMO_RESPONSES
            | ADAPTER_ARK_CODING
            | ADAPTER_COMMANDCODE
    )
}

/// Apply the adapter-owned modality matrix so stale `["text"]` rows cannot stick.
pub fn apply_owned_modality_capabilities(model: &mut ModelDefinition) {
    if adapter_owns_modality_matrix(&model.adapter_id) {
        model.config.capabilities =
            adapter_default_capabilities(&model.adapter_id, &model.config.api_model_id);
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
    if adapter_id == ADAPTER_COMMANDCODE && is_messages_only_model(&config.api_model_id) {
        return Err(LitecodeError::Config(format!(
            "model '{model_id}' api_model_id '{}' is served by the Command Code Anthropic Messages \
             endpoint, which this adapter does not implement; pick a non-Anthropic id from the \
             adapter catalog",
            config.api_model_id
        )));
    }
    if closed {
        let api = config.api_model_id.trim();
        if api.is_empty() {
            return Err(LitecodeError::Config(format!(
                "model '{model_id}' api_model_id is required"
            )));
        }
        if !has_remote_model_catalog(adapter_id) {
            let allowed = closed_api_model_ids(adapter_id).unwrap_or(&[]);
            if !allowed.contains(&api) {
                return Err(LitecodeError::Config(format!(
                    "model '{model_id}' api_model_id '{api}' is not in adapter catalog for '{adapter_id}'"
                )));
            }
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
        ADAPTER_ARK_CODING => Ok(Box::new(ArkCodingProvider::new(endpoint, auth)?)),
        ADAPTER_COMMANDCODE => Ok(Box::new(CommandcodeProvider::new(endpoint, auth)?)),
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
        ADAPTER_ARK_CODING, ADAPTER_COMMANDCODE, ADAPTER_DEEPSEEK_RESPONSES, ADAPTER_MIMO_RESPONSES,
        ADAPTER_OPENAI_RESPONSES, ADAPTER_OPENCODE,
    };

    #[test]
    fn mimo_full_modality_ids_default_to_all_input_modalities() {
        // Official per-model pages: 输入模态 文本、图像、视频、音频.
        for id in [
            "mimo-v2.5",
            "mimo-v2.6-pro",
            "mimo-v2.6-flash",
            "mimo-v2.6-pro-ultraspeed",
        ] {
            assert_eq!(
                adapter_default_capabilities(ADAPTER_MIMO_RESPONSES, id),
                vec![
                    ModelCapability::Text,
                    ModelCapability::Image,
                    ModelCapability::Video,
                    ModelCapability::Audio,
                ],
                "{id}"
            );
        }
        // A typo must not admit media the vendor rejects with 400.
        assert_eq!(
            adapter_default_capabilities(ADAPTER_MIMO_RESPONSES, "mimo-oops"),
            vec![ModelCapability::Text]
        );
    }

    /// Official DeepSeek Vision: the Flash line takes images, the Pro line does
    /// not. The retired `deepseek-v4-flash` / `-vision-exp` aliases now resolve
    /// to the same V4.1-Flash model, so they stay image-capable.
    #[test]
    fn deepseek_flash_line_defaults_to_text_and_image() {
        for id in [
            "deepseek-flash",
            "deepseek-v4-flash",
            "deepseek-v4-flash-vision-exp",
        ] {
            assert_eq!(
                adapter_default_capabilities(ADAPTER_DEEPSEEK_RESPONSES, id),
                vec![ModelCapability::Text, ModelCapability::Image],
                "{id}"
            );
        }
        // A typo must not admit a screenshot the vendor rejects with 400.
        assert_eq!(
            adapter_default_capabilities(ADAPTER_DEEPSEEK_RESPONSES, "deepseek-oops"),
            vec![ModelCapability::Text]
        );
    }

    #[test]
    fn mimo_pro_and_other_adapters_default_to_text() {
        assert_eq!(
            adapter_default_capabilities(ADAPTER_MIMO_RESPONSES, "mimo-v2.5-pro"),
            vec![ModelCapability::Text]
        );
        assert_eq!(
            adapter_default_capabilities(ADAPTER_DEEPSEEK_RESPONSES, "deepseek-v4-pro"),
            vec![ModelCapability::Text]
        );
        assert_eq!(
            adapter_default_capabilities(ADAPTER_OPENAI_RESPONSES, "gpt-4o"),
            vec![ModelCapability::Text]
        );
        assert_eq!(
            adapter_default_capabilities(ADAPTER_ARK_CODING, "deepseek-v4-flash"),
            vec![ModelCapability::Text]
        );
        assert_eq!(
            adapter_default_capabilities(ADAPTER_ARK_CODING, "doubao-seed-2.1-turbo"),
            vec![ModelCapability::Text, ModelCapability::Image]
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

        let v26 = parse_model_config(
            ADAPTER_MIMO_RESPONSES,
            &serde_json::json!({ "api_model_id": "mimo-v2.6-pro" }),
        )
        .unwrap();
        assert_eq!(
            v26.capabilities,
            vec![
                ModelCapability::Text,
                ModelCapability::Image,
                ModelCapability::Video,
                ModelCapability::Audio,
            ]
        );
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
    fn parse_model_config_ignores_legacy_thinking_keys() {
        let cfg = parse_model_config(
            ADAPTER_OPENAI_RESPONSES,
            &serde_json::json!({
                "api_model_id": "gpt-4o",
                "thinking_mode": "enabled",
                "reasoning_effort": "high",
            }),
        )
        .expect("legacy thinking keys must not fail parse");
        assert_eq!(cfg.api_model_id, "gpt-4o");
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
        let ark = list_adapters()
            .iter()
            .find(|a| a.id == ADAPTER_ARK_CODING)
            .unwrap();
        assert_eq!(ark.default_endpoint, Some(ARK_DEFAULT_ENDPOINT));
        assert!(!ark.remote_model_catalog);
        assert!(!has_remote_model_catalog(ADAPTER_ARK_CODING));
        let commandcode = list_adapters()
            .iter()
            .find(|a| a.id == ADAPTER_COMMANDCODE)
            .unwrap();
        assert_eq!(
            commandcode.default_endpoint,
            Some(COMMANDCODE_DEFAULT_ENDPOINT)
        );
        assert_eq!(commandcode.label, "Command Code");
        assert!(commandcode.remote_model_catalog);
        assert!(has_remote_model_catalog(ADAPTER_COMMANDCODE));
        assert!(deepseek.remote_model_catalog);
        assert!(has_remote_model_catalog(ADAPTER_DEEPSEEK_RESPONSES));
        assert!(mimo.remote_model_catalog);
        assert!(has_remote_model_catalog(ADAPTER_MIMO_RESPONSES));
        for adapter in [deepseek, mimo] {
            let api_field = adapter
                .model_fields
                .iter()
                .find(|f| f.name == "api_model_id")
                .unwrap();
            assert!(api_field.options.is_none());
        }
    }

    #[test]
    fn remote_catalog_adapters_accept_live_model_ids() {
        // `remote_model_catalog` is on, so `API_MODEL_IDS` is a fallback list —
        // not an allowlist — and a live `/models` id is never rejected.
        let deepseek = ModelAdapterConfig {
            api_model_id: "deepseek-flash".into(),
            ..ModelAdapterConfig::default()
        };
        validate_model_config("flash", ADAPTER_DEEPSEEK_RESPONSES, &deepseek).unwrap();
        let mimo = ModelAdapterConfig {
            api_model_id: "mimo-v2.6-pro".into(),
            ..ModelAdapterConfig::default()
        };
        validate_model_config("v26", ADAPTER_MIMO_RESPONSES, &mimo).unwrap();
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

        let ark = parse_provider_config(
            ADAPTER_ARK_CODING,
            &serde_json::json!({ "api_key": "sk-ark" }),
        )
        .unwrap();
        assert_eq!(ark.endpoint, ARK_DEFAULT_ENDPOINT);

        let cmd = parse_provider_config(
            ADAPTER_COMMANDCODE,
            &serde_json::json!({ "api_key": "sk-cmd" }),
        )
        .unwrap();
        assert_eq!(cmd.endpoint, COMMANDCODE_DEFAULT_ENDPOINT);
    }

    /// Real ids from `GET https://api.commandcode.ai/provider/v1/models`:
    /// Anthropic ids are un-prefixed (`claude-*`), everything else is
    /// `vendor/model` (or bare for OpenAI's `gpt-*`).
    #[test]
    fn commandcode_catalog_hides_messages_only_models() {
        let ids = vec![
            "claude-sonnet-5".into(),
            "claude-opus-4-8".into(),
            "claude-haiku-4-5-20251001".into(),
            "gpt-5.6-sol".into(),
            "deepseek/deepseek-v4-flash".into(),
            "zai-org/GLM-5.3".into(),
            "Qwen/Qwen3.8-Max".into(),
            "google/gemini-3.8-flash".into(),
            "poolside/laguna-s-2.1-free".into(),
        ];
        let filtered = catalog_supported_ids(ADAPTER_COMMANDCODE, ids);
        assert_eq!(
            filtered,
            vec![
                "gpt-5.6-sol".to_string(),
                "deepseek/deepseek-v4-flash".to_string(),
                "zai-org/GLM-5.3".to_string(),
                "Qwen/Qwen3.8-Max".to_string(),
                "google/gemini-3.8-flash".to_string(),
                "poolside/laguna-s-2.1-free".to_string(),
            ]
        );
    }

    #[test]
    fn catalog_filter_is_identity_for_other_adapters() {
        let ids = vec!["claude-sonnet-5".to_string()];
        assert_eq!(catalog_supported_ids(ADAPTER_OPENCODE, ids.clone()), ids);
    }

    /// Real ids from `GET https://api.xiaomimimo.com/v1/models` (2026-09-22):
    /// ASR/TTS ids share the list but are served on speech endpoints the
    /// Responses adapter does not implement.
    #[test]
    fn mimo_catalog_hides_speech_models() {
        let ids: Vec<String> = [
            "mimo-v2.5",
            "mimo-v2.5-asr",
            "mimo-v2.5-pro",
            "mimo-v2.5-tts",
            "mimo-v2.5-tts-voiceclone",
            "mimo-v2.5-tts-voicedesign",
            "mimo-v2.6-flash",
            "mimo-v2.6-pro",
            "mimo-v2.6-pro-ultraspeed",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        assert_eq!(
            catalog_supported_ids(ADAPTER_MIMO_RESPONSES, ids),
            vec![
                "mimo-v2.5",
                "mimo-v2.5-pro",
                "mimo-v2.6-flash",
                "mimo-v2.6-pro",
                "mimo-v2.6-pro-ultraspeed",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
        );
    }

    #[test]
    fn commandcode_rejects_messages_only_model_ids_at_config_time() {
        let claude = ModelAdapterConfig {
            api_model_id: "claude-sonnet-5".into(),
            context_window: 1_000_000,
            max_tokens: 8192,
            capabilities: vec![ModelCapability::Text],
            ..ModelAdapterConfig::default()
        };
        let err = validate_model_config("cc", ADAPTER_COMMANDCODE, &claude).unwrap_err();
        assert!(err.to_string().contains("Anthropic Messages"), "{err}");

        let deepseek = ModelAdapterConfig {
            api_model_id: "deepseek/deepseek-v4-flash".into(),
            ..claude
        };
        validate_model_config("cc", ADAPTER_COMMANDCODE, &deepseek).unwrap();
    }

    #[test]
    fn commandcode_offers_bearer_auth_only() {
        let descriptor = list_adapters()
            .iter()
            .find(|a| a.id == ADAPTER_COMMANDCODE)
            .unwrap();
        let auth = descriptor
            .provider_fields
            .iter()
            .find(|f| f.name == "auth")
            .unwrap();
        assert_eq!(auth.options, Some(&["bearer"][..]));
    }

    #[test]
    fn commandcode_modality_matrix_is_adapter_owned_text_only() {
        let mut model = ModelDefinition {
            id: "mm".into(),
            adapter_id: ADAPTER_COMMANDCODE.into(),
            provider_ref: "cc".into(),
            label: "MM".into(),
            config: ModelAdapterConfig {
                api_model_id: "gpt-5.6-sol".into(),
                context_window: 1_000_000,
                max_tokens: 8192,
                capabilities: vec![ModelCapability::Text, ModelCapability::Image],
                ..ModelAdapterConfig::default()
            },
        };
        apply_owned_modality_capabilities(&mut model);
        assert_eq!(model.config.capabilities, vec![ModelCapability::Text]);
    }
}
