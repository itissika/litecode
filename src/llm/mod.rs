//! LLM product surface — authority `Item` / `ModelRequest` only.
//!
//! Wire dialects live exclusively under private `codec/`. One codec per
//! [EndpointKind](crate::provider_catalog::EndpointKind);
//! providers themselves are data.

mod codec;
mod provider;
pub mod reconnect;
mod replay_compat;
mod request;

use std::sync::Arc;

pub use provider::LlmProvider;
pub use request::{ModelRequest, ToolDef};

/// Replay rules: ids stay off the wire except a summary-replay host's own
/// reasoning id, ciphertext only to the provider, protocol, and model that
/// minted it, summary replay drops reasoning any other provider produced, and
/// a rewritten Messages prefix drops the signature minted against that prefix.
pub(crate) use replay_compat::{
    ItemProducer, producers_for_seqs, retain_own_reasoning, strip_foreign_ciphertext,
    strip_rebased_signatures,
};

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
    /// The summarizer's own context window. The compact prompt is sized to it,
    /// not to the session being compacted. `0` means unknown: no cap.
    pub context_window: usize,
}

/// Construct the codec that serves a resolved catalog model.
pub fn provider_from_model(model: Arc<ResolvedModel>) -> Result<Box<dyn LlmProvider>> {
    codec::build(model)
}
