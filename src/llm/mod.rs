//! LLM product surface — authority `Item` / `ModelRequest` only.
//!
//! Wire dialects live exclusively under private `codec/`. There are exactly two
//! codecs, selected by the catalog's [EndpointKind](crate::provider_catalog::EndpointKind);
//! providers themselves are data.

mod codec;
mod provider;
mod replay_compat;
mod request;

use std::sync::Arc;

pub use provider::LlmProvider;
pub use request::{ModelRequest, ToolDef};

/// Replay rules: ids never go on the wire, ciphertext only to its producer,
/// reasoning text is never dropped.
pub(crate) use replay_compat::{producers_for_seqs, strip_foreign_ciphertext};

/// Folding a provider's stream into canonical `Item`s.
///
/// The dialect stays inside `codec`; what crosses the boundary is the product
/// concept: the stream and the terminal payload describe the same items, and
/// this is how one becomes the other.
pub(crate) use codec::stream_contract::{StreamItemAccumulator, item_id_of, mark_items_incomplete};

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
