//! Private provider wire adapters. Not `pub` from `llm`.
//!
//! # Adapter selection
//!
//! Product adapters are registered in [`registry`] (`openai_responses`,
//! `deepseek_responses`, `mimo_responses`, `opencode`, `ark_coding`). OpenAI /
//! DeepSeek / MiMo / Ark Coding Plan speak Responses JSON/SSE; OpenCode uses the
//! Chat Completions codec in [`chat_completions`]. Vendor-tolerant hardening
//! stays in this directory.
//!
//! # Streaming contract
//!
//! All live authority [`crate::types::StreamEvents`] leave this module through
//! [`stream_contract::forward_stream_event`]: tool `function_call_arguments.delta`
//! is never forwarded before an `output_item.added` for that item id (synthesized
//! when the provider omits it).

mod ark_coding;
mod chat_completions;
mod deepseek_responses;
mod mimo_responses;
mod openai_responses;
mod opencode;
mod registry;
mod responses_sse;
mod stream_contract;

use crate::config::schema::ProviderDefinition;
use crate::llm::provider::LlmProvider;
use crate::types::{LitecodeError, Result};

/// Preserve useful transport diagnostics without exposing credentials or bodies.
///
/// `reqwest::Error::Display` may include a full request URL. Keep only its
/// origin and path, intentionally discarding query and fragment components.
pub(super) fn transport_error(stage: &str, error: &reqwest::Error) -> LitecodeError {
    use std::error::Error as _;

    let kind = if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_request() {
        "request"
    } else if error.is_body() {
        "response_body"
    } else if error.is_decode() {
        "decode"
    } else {
        "transport"
    };
    let url = error
        .url()
        .map(|url| {
            let authority = match url.port() {
                Some(port) => format!("{}:{port}", url.host_str().unwrap_or("<unknown-host>")),
                None => url.host_str().unwrap_or("<unknown-host>").to_string(),
            };
            format!("{}://{}{}", url.scheme(), authority, url.path())
        })
        .unwrap_or_else(|| "<unavailable>".to_string());
    let mut causes = Vec::new();
    let mut source = error.source();
    while let Some(cause) = source {
        causes.push(cause.to_string());
        source = cause.source();
    }
    let cause = if causes.is_empty() {
        "<unavailable>".to_string()
    } else {
        causes.join(": ")
    };

    tracing::warn!(
        stage,
        kind,
        timeout = error.is_timeout(),
        connect = error.is_connect(),
        request = error.is_request(),
        body = error.is_body(),
        decode = error.is_decode(),
        url,
        cause,
        "LLM HTTP transport failed"
    );
    LitecodeError::Llm(format!(
        "{stage} failed ({kind}; timeout={}; connect={}; request={}; body={}; decode={}; url={url}; cause={cause})",
        error.is_timeout(),
        error.is_connect(),
        error.is_request(),
        error.is_body(),
        error.is_decode(),
    ))
}

/// Construct a boxed provider from a provider row (adapter_id selects the wire).
pub(super) fn from_definition(def: &ProviderDefinition) -> Result<Box<dyn LlmProvider>> {
    registry::build_client(def)
}

/// Public registry surface for settings API / validation (re-exported via `llm`).
pub mod public {
    pub use super::registry::{
        AdapterDescriptor, FieldSchema, FieldType, adapter_default_capabilities,
        apply_owned_modality_capabilities, closed_api_model_ids, closed_context_windows,
        closed_default_endpoint, has_remote_model_catalog, is_known_adapter, list_adapters,
        parse_model_config, parse_provider_config, provider_ready, validate_model_config,
        validate_provider_config,
    };
}

pub(crate) use chat_completions::{
    models_get_url as chat_models_url, parse_model_catalog as parse_chat_model_catalog,
};
