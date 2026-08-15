//! Per-provider LLM client cache keyed by settings revision.

use std::collections::HashMap;
use std::sync::Arc;

use crate::config::schema::ProviderDefinition;
use crate::llm::{LlmProvider, provider_from_definition};
use crate::types::{LitecodeError, Result};

pub struct ProviderRegistry {
    cache: HashMap<String, Arc<dyn LlmProvider>>,
    loaded_revision: u64,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            loaded_revision: 0,
        }
    }

    pub fn invalidate_if_stale(&mut self, revision: u64) {
        if revision != self.loaded_revision {
            self.cache.clear();
            self.loaded_revision = revision;
        }
    }

    pub fn get(
        &mut self,
        provider: &ProviderDefinition,
        revision: u64,
    ) -> Result<Arc<dyn LlmProvider>> {
        self.invalidate_if_stale(revision);
        let cache_key = format!("{}:{}", provider.adapter_id, provider.id);
        if let Some(existing) = self.cache.get(&cache_key) {
            return Ok(Arc::clone(existing));
        }
        let client = Arc::from(provider_from_definition(provider)?);
        self.cache.insert(cache_key, Arc::clone(&client));
        Ok(client)
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn provider_api_key(def: &ProviderDefinition) -> Result<String> {
    let key = def.config.api_key.trim();
    if key.is_empty() {
        return Err(LitecodeError::Config(format!(
            "provider '{}' api_key is required",
            def.id
        )));
    }
    Ok(key.to_string())
}
