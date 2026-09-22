//! Tokenizer used by the sparse lane's chunker and its budget checks.
//!
//! Loads the same `tokenizer.json` the embedder uses, but **without** the
//! truncation the shipped file carries (`truncation.max_length = 32768`).
//! That limit must not apply here: the chunker encodes a row once to find its
//! cut points, and with truncation on, the tail of a >32k-token row collapses
//! into one giant "chunk" (observed on a 430k-char reasoning row: 76 chunks,
//! the last one 317k chars). The embedder sets its own truncation, so this
//! instance cannot leak a longer sequence into the model.

use tokenizers::Tokenizer;

use crate::types::{LitecodeError, Result};

pub fn open() -> Result<Tokenizer> {
    let dir = crate::engines::code_search::model_dir()?;
    let path = dir.join("tokenizer.json");
    let mut tk = Tokenizer::from_file(&path)
        .map_err(|e| LitecodeError::Config(format!("tokenizer {}: {e}", path.display())))?;
    tk.with_truncation(None)
        .map_err(|e| LitecodeError::Config(format!("tokenizer {}: {e}", path.display())))?;
    Ok(tk)
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
