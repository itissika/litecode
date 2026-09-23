//! The two product codecs.
//!
//! [EndpointKind](crate::provider_catalog::EndpointKind) is the only codec
//! selector: providers are data, protocols are code. A codec receives a fully
//! resolved [ResolvedModel] and never inspects a provider id.

pub(crate) mod chat;
pub(crate) mod chat_synth;
pub(crate) mod chat_usage;
pub(super) mod http;
pub(super) mod replay;
pub(super) mod responses;
pub(super) mod responses_harden;
pub(super) mod sse;
pub(super) mod stream_contract;
pub(super) mod wire_dump;

use std::sync::Arc;

use reqwest::RequestBuilder;

use crate::llm::provider::LlmProvider;
use crate::provider_catalog::{AuthKind, EndpointKind, ResolvedModel};
use crate::types::Result;

/// Build the codec for a resolved model.
///
/// The match is exhaustive on purpose: a new [EndpointKind] does not compile
/// until its codec exists, and every kind has exactly one factory here.
pub(crate) fn build(model: Arc<ResolvedModel>) -> Result<Box<dyn LlmProvider>> {
    match model.endpoint_type {
        EndpointKind::Responses => Ok(Box::new(responses::ResponsesCodec::new(model)?)),
        EndpointKind::ChatCompletions => Ok(Box::new(chat::ChatCompletionsCodec::new(model)?)),
    }
}

/// Diagnostic prefix for upstream failures: names the catalog provider and the
/// protocol it speaks, never a vendor-specific implementation.
pub(crate) fn error_prefix(model: &ResolvedModel) -> String {
    match model.endpoint_type {
        EndpointKind::Responses => format!("provider '{}'", model.provider_name),
        EndpointKind::ChatCompletions => {
            format!("provider '{}' (Chat Completions)", model.provider_name)
        }
    }
}

/// User agent every request identifies itself with.
pub(crate) fn user_agent() -> String {
    format!("litecode/{}", env!("CARGO_PKG_VERSION"))
}

/// Apply the catalog auth mode.
pub(crate) fn apply_auth(builder: RequestBuilder, auth: AuthKind, api_key: &str) -> RequestBuilder {
    match auth {
        AuthKind::Bearer => builder.header("authorization", format!("Bearer {api_key}")),
        AuthKind::ApiKey => builder.header("api-key", api_key),
    }
}

/// Render catalog headers for one request.
///
/// `{{session_id}}` is the only template and falls back to `global` when the
/// request carries no session.
pub(crate) fn render_headers(
    headers: &[(String, String)],
    session_id: Option<&str>,
) -> Vec<(String, String)> {
    let session = session_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or("global");
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.replace("{{session_id}}", session)))
        .collect()
}
