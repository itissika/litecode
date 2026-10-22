//! Bridge helpers: global settings + catalog → runtime-facing views.
//!
//! Agents may have an empty `model_ref` until the user configures a provider.
//! Handshake / display paths must tolerate that; a turn still hard-fails in
//! [`crate::runtime::llm_resolve`].

use std::collections::{HashMap, HashSet};

use crate::config::AgentConfig;
use crate::config::resolved::ResolvedConfig;
use crate::config::schema::{AgentProfile, AgentRole, GlobalSettings};
use crate::provider_catalog::ProviderCatalog;
use crate::types::LitecodeError;

/// Whether a catalog provider can be called right now (non-blank credential).
fn provider_keyed(credentials: &HashMap<String, String>, provider_id: &str) -> bool {
    credentials
        .get(provider_id)
        .is_some_and(|key| !key.trim().is_empty())
}

/// Whether a reference can run right now: declared in the catalog and its
/// provider holds a credential.
///
/// The Settings → Models switch is deliberately not consulted: it governs the
/// pickers, not the resolver (see [`ResolvedConfig::active_models`]).
fn reference_usable(
    catalog: &ProviderCatalog,
    credentials: &HashMap<String, String>,
    reference: &str,
) -> bool {
    catalog
        .model(reference.trim())
        .is_some_and(|model| provider_keyed(credentials, &model.provider_id))
}

/// First model a user can actually run right now: catalog order, provider has a
/// credential, not switched off in Settings → Models.
///
/// Tool-calling models win: the agent loop needs them, so a catalog whose first
/// selectable model cannot call tools must not become everyone's fallback.
pub fn fallback_model_ref(
    catalog: &ProviderCatalog,
    credentials: &HashMap<String, String>,
    disabled: &HashSet<String>,
) -> Option<String> {
    let usable: Vec<_> = catalog
        .models()
        .iter()
        .filter(|model| provider_keyed(credentials, &model.provider_id))
        .filter(|model| !disabled.contains(&model.reference))
        .collect();
    usable
        .iter()
        .find(|model| model.tool_call)
        .or_else(|| usable.first())
        .map(|model| model.reference.clone())
}

/// Auto-heal every agent's `model_ref` in place, and return the repaired ids.
///
/// A ref that is empty, no longer declared in the catalog, or whose provider
/// lost its credential cannot run a turn, so it collapses to
/// [`fallback_model_ref`]. Nothing changes when the agent is already usable or
/// when no model is selectable at all (the turn then reports the usual
/// pick-a-model error).
///
/// Callers: every settings commit (`SettingsWriter`) and the boot-time bundle
/// load (`ConfigManager::load_runtime_bundle_from`).
pub fn repair_agent_models(
    settings: &mut GlobalSettings,
    catalog: &ProviderCatalog,
) -> Vec<String> {
    let Some(fallback) = fallback_model_ref(
        catalog,
        &settings.provider_credentials,
        &settings.disabled_models,
    ) else {
        return Vec::new();
    };
    let mut repaired = Vec::new();
    for (id, profile) in settings.agents.iter_mut() {
        if reference_usable(catalog, &settings.provider_credentials, &profile.model_ref) {
            continue;
        }
        profile.model_ref = fallback.clone();
        repaired.push(id.clone());
    }
    repaired.sort();
    repaired
}

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
    
    
    
    
    
    

}
