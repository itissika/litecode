//! LLM product surface — authority `Item` / `ModelRequest` only.
//!
//! Wire dialects live exclusively under private `adapter/`. Adapter registry is the
//! single source of truth for provider/model config shapes.

mod adapter;
mod provider;
mod request;

pub use provider::LlmProvider;
pub use request::{ModelRequest, ToolDef};

pub use adapter::public::{
    AdapterDescriptor, FieldSchema, FieldType, adapter_default_capabilities,
    apply_owned_modality_capabilities, closed_api_model_ids, closed_context_windows,
    closed_default_endpoint, has_remote_model_catalog, is_known_adapter, list_adapters,
    parse_model_config, parse_provider_config, provider_ready, validate_model_config,
    validate_provider_config,
};

pub(crate) use adapter::{chat_models_url, parse_chat_model_catalog};

use crate::config::schema::ProviderDefinition;
use crate::types::Result;

/// Construct a boxed provider from a provider row. Registry / tests must not name adapter types.
pub fn provider_from_definition(def: &ProviderDefinition) -> Result<Box<dyn LlmProvider>> {
    adapter::from_definition(def)
}
