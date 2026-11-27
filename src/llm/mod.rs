//! LLM product surface — authority `Item` / `ModelRequest` only.
//!
//! Wire dialects live exclusively under private `codec/`. One codec per
//! [EndpointKind](crate::provider_catalog::EndpointKind);
//! providers themselves are data.

mod codec;
pub mod http_runtime;
pub(crate) mod port;
mod provider;
pub mod reconnect;
mod replay_compat;
mod request;

use std::sync::Arc;

pub use provider::LlmProvider;
pub use request::{
    COMPACTION_INSTRUCTION, CompactionRequestContext, ModelRequest, RequestPurpose, ToolDef,
};

/// Replay rules: ids stay off the wire except a summary-replay host's own
/// reasoning id; ciphertext and signatures only travel to the producer that
/// minted them.
pub(crate) use replay_compat::{
    FixedPrefix, ItemProducer, fixed_prefix_digests, producers_for_seqs,
};

#[cfg(test)]
pub(crate) use replay_compat::strip_rebased_signatures;

/// Folding a provider's stream into canonical `Item`s.
///
/// The dialect stays inside `codec`; what crosses the boundary is the product
/// concept: the stream and the terminal payload describe the same items, and
/// this is how one becomes the other.
pub(crate) use codec::stream_contract::{StreamItemAccumulator, item_id_of, mark_items_incomplete};

use crate::provider_catalog::ResolvedModel;
use crate::types::Result;

/// Construct the codec that serves a resolved catalog model.
pub fn provider_from_model(model: Arc<ResolvedModel>) -> Result<Box<dyn LlmProvider>> {
    codec::build(model)
}
