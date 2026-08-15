//! Bridge helpers: global settings → runtime-facing views.
//!
//! Agents may have an empty `model_ref` until a structurally-ready model exists
//! (adapter-first empty seed). Handshake / display paths must tolerate that;
//! turn resolve still hard-fails via `llm_resolve`.

use crate::config::AgentConfig;
use crate::config::resolved::ResolvedConfig;
use crate::config::schema::{
    AgentProfile, AgentRole, GlobalSettings, ReasoningEffort, ThinkingMode,
};
use crate::types::LitecodeError;

pub const DEFAULT_CONTEXT_WINDOW: usize = 200_000;

/// List primary agents for wire handshake / UI picker.
pub fn primary_agent_infos(resolved: &ResolvedConfig) -> Vec<(String, String)> {
    let mut agents: Vec<_> = resolved
        .agents()
        .iter()
        .filter(|(_, p)| p.role == AgentRole::Primary)
        .map(|(id, p)| (id.clone(), p.description.clone()))
        .collect();
    agents.sort_by(|a, b| a.0.cmp(&b.0));
    agents
}

/// Map a global agent profile to the runtime `AgentConfig` view.
pub fn agent_config_from_profile(profile: &AgentProfile) -> AgentConfig {
    AgentConfig {
        role: role_to_string(profile.role),
        model_ref: profile.model_ref.clone(),
        system_prompt: profile.system_prompt.clone(),
        description: profile.description.clone(),
        temperature: profile.temperature,
        max_steps: profile.max_steps,
    }
}

/// Resolve runtime agent config for `agent_name` from a resolved view.
pub fn agent_config_for(
    resolved: &ResolvedConfig,
    agent_name: &str,
) -> Result<AgentConfig, LitecodeError> {
    resolved
        .agents()
        .get(agent_name)
        .map(agent_config_from_profile)
        .ok_or_else(|| {
            LitecodeError::Config(format!(
                "agent '{agent_name}' not found in configuration (seed/import required)"
            ))
        })
}

/// Context window for display / soft budget. Empty `model_ref` → default window (no panic).
pub fn context_window_for_agent(global: &GlobalSettings, agent_name: &str) -> usize {
    let model_ref = model_ref_for_agent(global, agent_name);
    if model_ref.is_empty() {
        return DEFAULT_CONTEXT_WINDOW;
    }

    global
        .models
        .get(&model_ref)
        .map(|m| m.context_window())
        .unwrap_or_else(|| {
            tracing::warn!(
                agent = %agent_name,
                model_ref = %model_ref,
                "context_window: model not in registry, using DEFAULT_CONTEXT_WINDOW"
            );
            DEFAULT_CONTEXT_WINDOW
        })
}

/// Max tokens for soft paths. Empty `model_ref` → 0 (turn resolve will hard-fail separately).
pub fn max_tokens_for_agent(global: &GlobalSettings, agent_name: &str) -> u32 {
    let model_ref = model_ref_for_agent(global, agent_name);
    if model_ref.is_empty() {
        return 0;
    }

    global
        .models
        .get(&model_ref)
        .map(|m| m.max_tokens())
        .unwrap_or_else(|| {
            tracing::warn!(
                agent = %agent_name,
                model_ref = %model_ref,
                "max_tokens: model not in registry, using 8192"
            );
            8192
        })
}

pub fn thinking_mode_for_agent(global: &GlobalSettings, agent_name: &str) -> Option<ThinkingMode> {
    let model_ref = model_ref_for_agent(global, agent_name);
    if model_ref.is_empty() {
        return None;
    }
    global
        .models
        .get(&model_ref)
        .and_then(|m| m.thinking_mode())
}

pub fn reasoning_effort_for_agent(
    global: &GlobalSettings,
    agent_name: &str,
) -> Option<ReasoningEffort> {
    let model_ref = model_ref_for_agent(global, agent_name);
    if model_ref.is_empty() {
        return None;
    }
    global
        .models
        .get(&model_ref)
        .and_then(|m| m.reasoning_effort())
}

pub fn json_output_for_agent(global: &GlobalSettings, agent_name: &str) -> bool {
    let model_ref = model_ref_for_agent(global, agent_name);
    if model_ref.is_empty() {
        return false;
    }
    global
        .models
        .get(&model_ref)
        .map(|m| m.json_output())
        .unwrap_or(false)
}

fn model_ref_for_agent(global: &GlobalSettings, agent_name: &str) -> String {
    global
        .agents
        .get(agent_name)
        .map(|a| a.model_ref.clone())
        .or_else(|| global.agents.get("default").map(|a| a.model_ref.clone()))
        .unwrap_or_default()
}

/// Warn when agents lack a model landing point (expected after empty LLM seed).
pub fn warn_bridge_fallbacks(global: &GlobalSettings) {
    for (agent_id, profile) in &global.agents {
        if profile.model_ref.is_empty() {
            tracing::warn!(
                agent = %agent_id,
                "agent model_ref is empty; configure a ready model in Settings"
            );
            continue;
        }
        if !global.models.contains_key(&profile.model_ref) {
            tracing::warn!(
                agent = %agent_id,
                model_ref = %profile.model_ref,
                "agent model_ref missing from models registry"
            );
        }
    }
}

/// Wire / handshake display id for an agent profile's seeded `model_ref`.
/// Empty / missing catalog → empty string (no soft fallback to raw model_ref).
/// Turn execution must use `llm_resolve`, which hard-fails if unset.
pub fn api_model_id_for_agent(global: &GlobalSettings, agent_name: &str) -> String {
    let model_ref = model_ref_for_agent(global, agent_name);
    if model_ref.is_empty() {
        return String::new();
    }

    global
        .models
        .get(&model_ref)
        .map(|m| m.api_model_id().to_string())
        .unwrap_or_default()
}

fn role_to_string(role: AgentRole) -> String {
    match role {
        AgentRole::Primary => "primary".into(),
        AgentRole::Subagent => "subagent".into(),
        AgentRole::Hidden => "hidden".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{
        ADAPTER_OPENAI_RESPONSES, AgentProfile, ModelAdapterConfig, ModelCapability,
        ModelDefinition, ProviderAuth, ProviderConnectionConfig, ProviderDefinition,
    };
    use std::collections::HashMap;

    fn sample_global() -> GlobalSettings {
        GlobalSettings {
            providers: HashMap::from([(
                "main".into(),
                ProviderDefinition {
                    id: "main".into(),
                    adapter_id: ADAPTER_OPENAI_RESPONSES.into(),
                    label: "Main".into(),
                    config: ProviderConnectionConfig {
                        endpoint: "https://api.example.com/v1".into(),
                        api_key: "sk-test".into(),
                        auth: ProviderAuth::Bearer,
                    },
                },
            )]),
            models: HashMap::from([
                (
                    "default".into(),
                    ModelDefinition {
                        id: "default".into(),
                        adapter_id: ADAPTER_OPENAI_RESPONSES.into(),
                        provider_ref: "main".into(),
                        label: "Default".into(),
                        config: ModelAdapterConfig {
                            api_model_id: "api-default".into(),
                            context_window: 100_000,
                            max_tokens: 12_345,
                            thinking_mode: None,
                            reasoning_effort: None,
                            json_output: false,
                            capabilities: vec![ModelCapability::Text],
                        },
                    },
                ),
                (
                    "compaction".into(),
                    ModelDefinition {
                        id: "compaction".into(),
                        adapter_id: ADAPTER_OPENAI_RESPONSES.into(),
                        provider_ref: "main".into(),
                        label: "Compaction".into(),
                        config: ModelAdapterConfig {
                            api_model_id: "api-compact".into(),
                            context_window: 200_000,
                            max_tokens: 4_096,
                            thinking_mode: None,
                            reasoning_effort: None,
                            json_output: false,
                            capabilities: vec![ModelCapability::Text],
                        },
                    },
                ),
            ]),
            agents: HashMap::from([
                (
                    "default".into(),
                    AgentProfile {
                        model_ref: "default".into(),
                        ..Default::default()
                    },
                ),
                (
                    "compaction".into(),
                    AgentProfile {
                        model_ref: "compaction".into(),
                        ..Default::default()
                    },
                ),
            ]),
            ..Default::default()
        }
    }

    #[test]
    fn max_tokens_for_agent_reads_models_table() {
        let global = sample_global();
        assert_eq!(max_tokens_for_agent(&global, "default"), 12_345);
        assert_eq!(max_tokens_for_agent(&global, "compaction"), 4_096);
    }

    #[test]
    fn context_window_for_agent_reads_models_table() {
        let global = sample_global();
        assert_eq!(context_window_for_agent(&global, "default"), 100_000);
        assert_eq!(DEFAULT_CONTEXT_WINDOW, 200_000);
    }

    #[test]
    fn empty_model_ref_is_soft_empty_for_handshake() {
        let mut global = sample_global();
        global.agents.insert(
            "default".into(),
            AgentProfile {
                model_ref: String::new(),
                ..Default::default()
            },
        );
        assert_eq!(api_model_id_for_agent(&global, "default"), "");
        assert_eq!(max_tokens_for_agent(&global, "default"), 0);
        assert_eq!(
            context_window_for_agent(&global, "default"),
            DEFAULT_CONTEXT_WINDOW
        );
        assert!(thinking_mode_for_agent(&global, "default").is_none());
        assert!(!json_output_for_agent(&global, "default"));
    }
}
