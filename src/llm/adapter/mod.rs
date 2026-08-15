//! Private provider wire adapters. Not `pub` from `llm`.
//!
//! # Adapter selection
//!
//! Product adapters are registered in [`registry`] (`openai_responses`,
//! `deepseek_responses`, `mimo_responses`). All three speak Responses JSON/SSE;
//! vendor-tolerant hardening stays inside this module.
//!
//! # Streaming contract
//!
//! All live authority [`crate::types::StreamEvents`] leave this module through
//! [`stream_contract::forward_stream_event`]: tool `function_call_arguments.delta`
//! is never forwarded before an `output_item.added` for that item id (synthesized
//! when the provider omits it).

mod deepseek_responses;
mod mimo_responses;
mod openai_responses;
mod registry;
mod responses_sse;
mod stream_contract;

use crate::config::schema::ProviderDefinition;
use crate::llm::provider::LlmProvider;
use crate::types::Result;

/// Construct a boxed provider from a provider row (adapter_id selects the wire).
pub(super) fn from_definition(def: &ProviderDefinition) -> Result<Box<dyn LlmProvider>> {
    registry::build_client(def)
}

/// Public registry surface for settings API / validation (re-exported via `llm`).
pub mod public {
    pub use super::registry::{
        AdapterDescriptor, FieldSchema, FieldType, adapter_default_capabilities,
        closed_api_model_ids, closed_context_windows, closed_default_endpoint, is_known_adapter,
        list_adapters, parse_model_config, parse_provider_config, provider_ready,
        validate_model_config, validate_provider_config,
    };
}
