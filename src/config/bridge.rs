//! Bridge helpers: global settings + catalog → runtime-facing views.
//!
//! Agents may have an empty `model_ref` until the user configures a provider.
//! Handshake / display paths must tolerate that; a turn still hard-fails in
//! [`crate::runtime::llm_resolve`].

use crate::config::AgentConfig;
use crate::config::resolved::ResolvedConfig;
use crate::config::schema::{AgentProfile, AgentRole};
use crate::types::LitecodeError;

/// List primary agents for wire handshake / UI picker.
pub fn primary_agent_infos(resolved: &ResolvedConfig) -> Vec<(String, String)> {
    let mut agents: Vec<_> = resolved
        .agents()
        .iter()
        .filter(|(_, profile)| profile.role == AgentRole::Primary)
        .map(|(id, profile)| (id.clone(), profile.description.clone()))
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

/// Warn once per start when an agent still points at a model the catalog does
/// not declare, or at a provider without a key.
///
/// This is deliberately a warning: Settings must stay reachable so the user can
/// fix the reference. A turn still hard-fails in `llm_resolve`.
pub fn warn_unresolved_agent_models(resolved: &ResolvedConfig) {
    for (agent_id, profile) in resolved.agents() {
        if profile.model_ref.is_empty() {
            tracing::warn!(
                agent = %agent_id,
                "agent has no model_ref; configure a provider key in Settings → Providers \
                 and assign a model in Settings → Agents"
            );
            continue;
        }
        let Some(model) = resolved.catalog().model(&profile.model_ref) else {
            tracing::warn!(
                agent = %agent_id,
                model_ref = %profile.model_ref,
                "agent model_ref is not in the provider catalog; pick a model again in Settings → Agents"
            );
            continue;
        };
        if resolved.provider_api_key(&model.provider_id).is_none() {
            tracing::warn!(
                agent = %agent_id,
                model_ref = %profile.model_ref,
                provider = %model.provider_id,
                "agent model's provider has no API key yet"
            );
        }
    }
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
    use crate::config::resolved::WorkspaceState;
    use crate::config::schema::{AgentProfile, GlobalSettings};
    use crate::provider_catalog::ProviderCatalog;
    use std::path::Path;
    use std::sync::Arc;

    const CATALOG: &str = r#"
version = 1
[[providers]]
id = "openai"
name = "OpenAI"
endpoint = "https://api.openai.com/v1"
endpoint_type = "responses"

[[models]]
id = "gpt"
provider_id = "openai"
"#;

    fn resolved(credentials: &[(&str, &str)]) -> ResolvedConfig {
        let mut global = GlobalSettings::default();
        for (provider, key) in credentials {
            global
                .provider_credentials
                .insert((*provider).into(), (*key).into());
        }
        global.agents.insert(
            "default".into(),
            AgentProfile {
                model_ref: "openai/gpt".into(),
                ..Default::default()
            },
        );
        let catalog = Arc::new(ProviderCatalog::parse(CATALOG, Path::new("t.toml")).unwrap());
        crate::config::resolved::resolve(global, WorkspaceState::new("/tmp/ws"), catalog)
    }

    #[test]
    fn active_models_require_a_credential() {
        let without = resolved(&[]);
        assert!(without.active_models().is_empty());
        assert!(without.model_for_agent("default").is_none());

        let with = resolved(&[("openai", "sk-test")]);
        assert_eq!(with.active_models().len(), 1);
        assert_eq!(
            with.model_for_agent("default").map(|m| m.reference.clone()),
            Some("openai/gpt".to_string())
        );
    }

    #[test]
    fn agent_config_carries_the_model_ref() {
        let config = agent_config_for(&resolved(&[]), "default").unwrap();
        assert_eq!(config.model_ref, "openai/gpt");
        assert!(agent_config_for(&resolved(&[]), "ghost").is_err());
    }

    #[test]
    fn primary_agent_infos_lists_primary_agents() {
        let infos = primary_agent_infos(&resolved(&[]));
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].0, "default");
    }
}
