//! LLM product surface — authority `Item` / `ModelRequest` only.
//!
//! Wire dialects live exclusively under private `codec/`. There are exactly two
//! codecs, selected by the catalog's [EndpointKind](crate::provider_catalog::EndpointKind);
//! providers themselves are data.

mod codec;
mod provider;
mod request;

use std::sync::Arc;

pub use provider::LlmProvider;
pub use request::{ModelRequest, ToolDef};

use crate::provider_catalog::ResolvedModel;
use crate::types::Result;

/// Compact request context: codec + credentials + wire model id.
///
/// No vendor thinking strings. Callers map a [crate::runtime::TurnLlmBinding]
/// via `compact_call()`; [ModelRequest::compact] always sets thinking Off.
#[derive(Clone, Copy)]
pub struct CompactLlmCall<'a> {
    pub provider: &'a dyn LlmProvider,
    pub api_key: &'a str,
    pub model: &'a str,
}

/// Construct the codec that serves a resolved catalog model.
pub fn provider_from_model(model: Arc<ResolvedModel>) -> Result<Box<dyn LlmProvider>> {
    codec::build(model)
}
