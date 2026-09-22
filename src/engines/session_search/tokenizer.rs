//! Tokenizer used by the sparse lane's chunker and its budget checks.
//!
//! Loads the same `tokenizer.json` the embedder uses, but **without** the
//! truncation the shipped file carries (`truncation.max_length = 32768`).
//! That limit must not apply here: the chunker encodes a row once to find its
//! cut points, and with truncation on, the tail of a >32k-token row collapses
//! into one giant "chunk" (observed on a 430k-char reasoning row: 76 chunks,
//! the last one 317k chars). The embedder sets its own truncation, so this
//! instance cannot leak a longer sequence into the model.

use std::sync::{Arc, OnceLock};

use tokenizers::Tokenizer;

use crate::types::{LitecodeError, Result};

/// The shared tokenizer, loaded once per process.
///
/// Loading `tokenizer.json` costs about two seconds. That is fine once, and
/// ruinous per refresh: the reconcile calls it on every pass, so an idle refresh
/// of an unchanged corpus used to cost the same as a full rebuild. Everything
/// that needs a tokenizer should come through here.
pub fn shared() -> Result<Arc<Tokenizer>> {
    static TK: OnceLock<Arc<Tokenizer>> = OnceLock::new();
    if let Some(tk) = TK.get() {
        return Ok(Arc::clone(tk));
    }
    let tk = Arc::new(load()?);
    // A racing thread may have won; either instance is equivalent, so keep the
    // one that landed first and drop ours.
    let _ = TK.set(Arc::clone(&tk));
    Ok(TK.get().map(Arc::clone).unwrap_or(tk))
}

fn load() -> Result<Tokenizer> {
    let dir = crate::engines::code_search::model_dir()?;
    let path = dir.join("tokenizer.json");
    let mut tk = Tokenizer::from_file(&path)
        .map_err(|e| LitecodeError::Config(format!("tokenizer {}: {e}", path.display())))?;
    tk.with_truncation(None)
        .map_err(|e| LitecodeError::Config(format!("tokenizer {}: {e}", path.display())))?;
    Ok(tk)
}

/// Load a private tokenizer. Prefer [`shared`]; this exists for callers that
/// need to mutate truncation or for tests that measure the load itself.
#[allow(dead_code)]
pub fn open() -> Result<Tokenizer> {
    load()
}

/// Full token count (truncation is stripped in [`open`] on purpose: the chunk
/// budget must count every token the product window would have to drop).
pub fn token_len(tk: &Tokenizer, text: &str) -> usize {
    match tk.encode(text, false) {
        Ok(enc) => enc.len(),
        // Fallback: mixed CJK/Latin ≈ 3 chars per token.
        Err(_) => text.chars().count() / 3 + 1,
    }
}
