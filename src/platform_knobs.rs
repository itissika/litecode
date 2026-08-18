//! Platform semantics for thinking intensity and context window mode.
//!
//! See `docs/platform-knobs.md`. UI / session persist platform enums; adapters translate to vendor wire.

use serde::{Deserialize, Serialize};

use crate::config::schema::{ADAPTER_DEEPSEEK_RESPONSES, ADAPTER_MIMO_RESPONSES, ModelDefinition};
use crate::llm::closed_context_windows;

/// System / OpenAI-compatible Default budget (economic).
pub const CONTEXT_STANDARD_OPEN: usize = 200_000;
pub const CLOSED_DEFAULT_MAX_TOKENS: u32 = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ThinkingTier {
    Low,
    #[default]
    Medium,
    High,
}


impl ThinkingTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ContextMode {
    #[default]
    Standard,
    Max,
}


impl ContextMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Max => "max",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "standard" => Some(Self::Standard),
            "max" => Some(Self::Max),
            _ => None,
        }
    }
}

pub fn is_closed_adapter(adapter_id: &str) -> bool {
    matches!(
        adapter_id,
        ADAPTER_DEEPSEEK_RESPONSES | ADAPTER_MIMO_RESPONSES
    )
}

pub fn closed_wire_model(adapter_id: &str) -> Option<&'static str> {
    crate::llm::closed_api_model_ids(adapter_id).and_then(|ids| ids.first().copied())
}

/// Wire model id for the LLM request — always the Settings-saved `api_model_id`.
///
/// Closed adapters pick from the adapter catalog in Settings; open adapters are free text.
pub fn effective_api_model_id(model: &ModelDefinition) -> String {
    model.api_model_id().trim().to_string()
}

/// Effective context budget for `ContextPipeline`.
///
/// - **standard**: platform Default (closed = adapter default; open = 200K, never above declared max)
/// - **max**: the model's declared maximum window (closed = adapter max; open = Settings `context_window`)
pub fn effective_context_window(model: &ModelDefinition, mode: ContextMode) -> usize {
    if let Some((default, max)) = closed_context_windows(&model.adapter_id) {
        return match mode {
            ContextMode::Standard => default,
            ContextMode::Max => max,
        };
    }

    // Open / OpenAI-compatible: Max = Settings-declared window; Default = system 200K capped by that.
    let declared_max = model.context_window();
    match mode {
        ContextMode::Max => {
            if declared_max > 0 {
                declared_max
            } else {
                CONTEXT_STANDARD_OPEN
            }
        }
        ContextMode::Standard => {
            if declared_max > 0 {
                CONTEXT_STANDARD_OPEN.min(declared_max)
            } else {
                CONTEXT_STANDARD_OPEN
            }
        }
    }
}

pub fn effective_max_tokens(model: &ModelDefinition) -> u32 {
    let configured = model.max_tokens();
    if configured > 0 {
        return configured;
    }
    if is_closed_adapter(&model.adapter_id) {
        CLOSED_DEFAULT_MAX_TOKENS
    } else {
        0
    }
}

/// Map platform `thinking_tier` to legacy `ModelRequest` vendor fields (adapter-specific).
pub fn map_thinking_to_wire(
    adapter_id: &str,
    tier: ThinkingTier,
) -> (Option<String>, Option<String>) {
    match adapter_id {
        ADAPTER_DEEPSEEK_RESPONSES => match tier {
            ThinkingTier::Low => (None, Some("low".into())),
            ThinkingTier::Medium => (None, Some("high".into())),
            ThinkingTier::High => (None, Some("max".into())),
        },
        ADAPTER_MIMO_RESPONSES => match tier {
            ThinkingTier::Low => (Some("disabled".into()), None),
            ThinkingTier::Medium => (Some("enabled".into()), Some("medium".into())),
            ThinkingTier::High => (Some("enabled".into()), Some("high".into())),
        },
        _ => match tier {
            ThinkingTier::Low => (None, Some("low".into())),
            ThinkingTier::Medium => (None, Some("medium".into())),
            ThinkingTier::High => (None, Some("high".into())),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{ADAPTER_OPENAI_RESPONSES, ModelAdapterConfig, ModelCapability};

    fn closed_model(adapter_id: &str) -> ModelDefinition {
        ModelDefinition {
            id: "x".into(),
            adapter_id: adapter_id.into(),
            provider_ref: "p".into(),
            label: "X".into(),
            config: ModelAdapterConfig {
                api_model_id: String::new(),
                context_window: 0,
                max_tokens: 0,
                thinking_mode: None,
                reasoning_effort: None,
                json_output: false,
                capabilities: vec![ModelCapability::Text],
            },
        }
    }

    fn open_model(declared: usize) -> ModelDefinition {
        ModelDefinition {
            id: "o".into(),
            adapter_id: ADAPTER_OPENAI_RESPONSES.into(),
            provider_ref: "p".into(),
            label: "O".into(),
            config: ModelAdapterConfig {
                api_model_id: "gpt".into(),
                context_window: declared,
                max_tokens: 8192,
                thinking_mode: None,
                reasoning_effort: None,
                json_output: false,
                capabilities: vec![ModelCapability::Text],
            },
        }
    }

    #[test]
    fn closed_default_and_max_come_from_adapter() {
        let ds = closed_model(ADAPTER_DEEPSEEK_RESPONSES);
        assert_eq!(
            effective_context_window(&ds, ContextMode::Standard),
            256_000
        );
        assert_eq!(effective_context_window(&ds, ContextMode::Max), 1_000_000);

        let mimo = closed_model(ADAPTER_MIMO_RESPONSES);
        assert_eq!(
            effective_context_window(&mimo, ContextMode::Standard),
            256_000
        );
        assert_eq!(effective_context_window(&mimo, ContextMode::Max), 1_000_000);
    }

    #[test]
    fn open_max_uses_settings_declared_window_not_1m() {
        let m = open_model(128_000);
        assert_eq!(effective_context_window(&m, ContextMode::Max), 128_000);
        assert_eq!(effective_context_window(&m, ContextMode::Standard), 128_000);
    }

    #[test]
    fn open_default_is_200k_when_declared_allows() {
        let m = open_model(1_000_000);
        assert_eq!(effective_context_window(&m, ContextMode::Standard), 200_000);
        assert_eq!(effective_context_window(&m, ContextMode::Max), 1_000_000);
    }

    #[test]
    fn deepseek_thinking_tiers_map_to_responses_effort() {
        assert_eq!(
            map_thinking_to_wire(ADAPTER_DEEPSEEK_RESPONSES, ThinkingTier::Low),
            (None, Some("low".into()))
        );
        assert_eq!(
            map_thinking_to_wire(ADAPTER_DEEPSEEK_RESPONSES, ThinkingTier::Medium),
            (None, Some("high".into()))
        );
        assert_eq!(
            map_thinking_to_wire(ADAPTER_DEEPSEEK_RESPONSES, ThinkingTier::High),
            (None, Some("max".into()))
        );
    }

    #[test]
    fn closed_api_model_uses_saved_selection() {
        let mut model = closed_model(ADAPTER_DEEPSEEK_RESPONSES);
        model.config.api_model_id = "deepseek-v4-pro".into();
        assert_eq!(effective_api_model_id(&model), "deepseek-v4-pro");
        assert_eq!(effective_max_tokens(&model), 8192);
    }
}
