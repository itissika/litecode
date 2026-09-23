//! Per-model codec cache.
//!
//! Codecs are built from a resolved catalog model and never hold credentials, so
//! a key change does not invalidate the cache and a cache key can never leak a
//! secret.

use std::collections::HashMap;
use std::sync::Arc;

use crate::llm::{LlmProvider, provider_from_model};
use crate::provider_catalog::ResolvedModel;
use crate::types::{LitecodeError, Result};

pub struct ProviderRegistry {
    cache: HashMap<String, Arc<dyn LlmProvider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    pub fn get(&mut self, model: &Arc<ResolvedModel>) -> Result<Arc<dyn LlmProvider>> {
        if let Some(existing) = self.cache.get(&model.reference) {
            return Ok(Arc::clone(existing));
        }
        let codec = Arc::from(provider_from_model(Arc::clone(model))?);
        self.cache.insert(model.reference.clone(), Arc::clone(&codec));
        Ok(codec)
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Credential for one catalog provider, read from the resolved settings.
pub fn provider_api_key(
    resolved: &crate::config::resolved::ResolvedConfig,
    provider_id: &str,
) -> Result<String> {
    resolved
        .provider_api_key(provider_id)
        .map(str::to_string)
        .ok_or_else(|| {
            LitecodeError::Config(format!(
                "provider '{provider_id}' has no API key yet: add one in Settings → Providers"
            ))
        })
}
