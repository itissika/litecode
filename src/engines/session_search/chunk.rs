//! Lossless chunking, **hard cut only** (locked 2026-09-21).
//!
//! The product embeds one transcript row per vector and its window is 512 tokens
//! with `stride: 0`, so everything past the head of a long row never reaches the
//! model. This module turns one long row into several chunks **without losing a
//! character**.
//!
//! Losslessness is a numeric invariant rather than a promise: every chunk carries
//! the char range `[start, end)` it covers, and the ranges of one row must tile
//! `[0, source_chars)` with no gap. `uncovered_chars` is checked on every board
//! run.
//!
//! # Why hard only
//!
//! The cut is plain fixed-size cutting: the furthest character offset that fits
//! the token budget, ignoring line structure. The semantic lane is wide recall —
//! landing the right row matters, the entry point inside the row is refined later.
//!
//! The boundary-first modes (`Boundary`, `Snap`) were archived with the rest of
//! the pre-lock machinery: `Boundary` gave back a whole line at every cut (p50
//! 350 tokens against a 448 budget), cost ~8% more vectors for the same text and
//! measured 1.6 points **worse** on dense P@1. Line structure does not pay for
//! itself here.

use serde::{Deserialize, Serialize};
use tokenizers::Tokenizer;

use super::tokenizer::token_len;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ChunkCfg {
    /// Token budget per chunk. Kept below the product window (512) so the
    /// tokenizer never has to truncate what we hand it.
    pub tokens: usize,
    /// Keep the row's head+tail projection as an extra **anchor** vector next to
    /// the faithful chunks.
    ///
    /// Measured: a faithful split alone *loses* ranking — each 448-token slice is
    /// a weaker match than the whole head+tail summary, and the row's best chunk
    /// ranks below where the summary used to (dense P@1 0.074 → 0.049 at
    /// `--fetch-k 64`). The anchor restores the ranking while the chunks supply
    /// the coverage the anchor cannot have.
    pub anchor: bool,
}

impl Default for ChunkCfg {
    fn default() -> Self {
        Self {
            tokens: 448,
            anchor: true,
        }
    }
}

/// One embeddable slice of a row. `index`/`total` are 0-based/1-based.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub index: usize,
    pub total: usize,
    /// Char offsets into the source text, `[start, end)`.
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Split `text` into chunks of at most `cfg.tokens` tokens each, by hard cut.
///
/// Returns exactly one chunk for any text that already fits the budget; returns
/// an empty vector only for an empty text. The cuts ignore line structure: a
/// line that cannot fit the budget at all is cut mid-line, and one that does is
/// simply included whole.
///
/// Implementation: encode the text **once**, walk its token boundaries, and
/// greedily pack up to `budget` tokens per chunk; each candidate slice is then
/// re-tokenized standalone to keep the "≤ budget" contract exact (tokenization
/// is not perfectly context-free at the cut). The original char-level binary
/// search (`hard_cut_ranges`) stays as the fallback when the tokenizer cannot
/// produce offsets. This removes the per-cut re-tokenization storm that made a
/// full rebuild take minutes.
pub fn chunk_text(tk: &Tokenizer, text: &str, cfg: &ChunkCfg) -> Vec<Chunk> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n == 0 {
        return Vec::new();
    }
    let budget = cfg.tokens.max(1);

    let ranges = token_cut_ranges(tk, text, &chars, budget)
        .unwrap_or_else(|| hard_cut_ranges(tk, &chars, budget));

    let total = ranges.len();
    ranges
        .into_iter()
        .enumerate()
        .map(|(index, (start, end))| Chunk {
            index,
            total,
            start,
            end,
            text: chars[start..end].iter().collect(),
        })
        .collect()
}

/// One encode pass + greedy whole-token packing. `None` when the tokenizer
/// cannot encode the text (the caller then falls back to [`hard_cut_ranges`]).
fn token_cut_ranges(
    tk: &Tokenizer,
    text: &str,
    chars: &[char],
    budget: usize,
) -> Option<Vec<(usize, usize)>> {
    let n = chars.len();
    let enc = tk.encode_char_offsets(text, false).ok()?;
    let ends: Vec<usize> = enc
        .get_offsets()
        .iter()
        .map(|&(_, end)| end.min(n))
        .collect();
    let n_tokens = ends.len();
    // No tokens at all (e.g. whitespace-only) or the whole text fits.
    if n_tokens == 0 || n_tokens <= budget {
        return Some(vec![(0, n)]);
    }

    let mut ranges = Vec::new();
    let mut start = 0usize;
    let mut t0 = 0usize;
    while start < n {
        while t0 < n_tokens && ends[t0] <= start {
            t0 += 1;
        }
        if t0 >= n_tokens {
            // Only characters beyond the last token remain.
            ranges.push((start, n));
            break;
        }
        let mut t1 = (t0 + budget).min(n_tokens);
        let mut end = if t1 == n_tokens { n } else { ends[t1 - 1] };
        // Tokenization at a cut is not perfectly context-free: verify the slice
        // standalone and step back token by token when it overshoots.
        loop {
            if end <= start {
                break;
            }
            let slice: String = chars[start..end].iter().collect();
            if token_len(tk, &slice) <= budget {
                break;
            }
            if t1 - t0 <= 1 {
                end = hard_end(tk, chars, start, budget);
                break;
            }
            t1 -= 1;
            end = if t1 == n_tokens { n } else { ends[t1 - 1] };
        }
        if end <= start {
            // Degenerate token spans; the char-level cutter always progresses.
            end = hard_end(tk, chars, start, budget);
        }
        ranges.push((start, end));
        start = end;
    }
    Some(ranges)
}

/// The original char-level cutter: the furthest char offset whose slice
/// tokenizes to ≤ budget. Kept as the fallback path.
fn hard_cut_ranges(tk: &Tokenizer, chars: &[char], budget: usize) -> Vec<(usize, usize)> {
    let n = chars.len();
    let mut ranges = Vec::new();
    let mut start = 0usize;
    while start < n {
        let end = hard_end(tk, chars, start, budget);
        debug_assert!(end > start, "chunker must make progress");
        ranges.push((start, end));
        start = end;
    }
    ranges
}

/// Furthest character offset that fits the budget, with no regard for line
/// structure — the plain "hard cut".
fn hard_end(tk: &Tokenizer, chars: &[char], start: usize, budget: usize) -> usize {
    let n = chars.len();
    let mut lo = start + 1;
    let mut hi = n;
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if fits(tk, chars, start, mid, budget) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo.max(start + 1)
}

fn fits(tk: &Tokenizer, chars: &[char], a: usize, b: usize, budget: usize) -> bool {
    let s: String = chars[a..b].iter().collect();
    token_len(tk, &s) <= budget
}

/// Chars of `[0, source_chars)` that no chunk covers. Must be 0.
pub fn uncovered_chars(chunks: &[Chunk], source_chars: usize) -> usize {
    if source_chars == 0 {
        return 0;
    }
    let mut cursor = 0usize;
    for c in chunks {
        cursor = cursor.max(c.end);
    }
    source_chars.saturating_sub(cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tk() -> Tokenizer {
        // Same loader the chunker callers use: `open` strips the 32768-token
        // truncation the shipped tokenizer.json carries, and the tests must see
        // the tokenizer the lane actually chunks with.
        super::super::tokenizer::open().expect("load tokenizer")
    }

    fn assert_lossless(tk: &Tokenizer, text: &str, cfg: &ChunkCfg) -> Vec<Chunk> {
        let chunks = chunk_text(tk, text, cfg);
        let n = text.chars().count();
        assert!(!chunks.is_empty(), "non-empty text must yield chunks");
        assert_eq!(
            uncovered_chars(&chunks, n),
            0,
            "chunker dropped characters ({} of {n})",
            uncovered_chars(&chunks, n)
        );
        // Every chunk is inside the budget, so the product window never truncates.
        for c in &chunks {
            assert!(
                token_len(tk, &c.text) <= cfg.tokens,
                "chunk {} is {} tokens, budget {}",
                c.index,
                token_len(tk, &c.text),
                cfg.tokens
            );
        }
        // Reassembly: hard cuts do not overlap, so the source must rebuild exactly.
        let rebuilt: String = chunks.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(rebuilt, text, "reassembly differs from the source");
        chunks
    }

    #[test]
    fn short_text_is_one_chunk() {
        let tk = tk();
        let cfg = ChunkCfg::default();
        let chunks = assert_lossless(&tk, "one short line\n", &cfg);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].total, 1);
    }

    #[test]
    fn long_text_is_cut_into_tiles() {
        let tk = tk();
        let cfg = ChunkCfg { tokens: 128, anchor: false };
        let text: String = (0..400)
            .map(|i| format!("line {i}: the retry backoff doubles until it hits the cap.\n"))
            .collect();
        let chunks = assert_lossless(&tk, &text, &cfg);
        assert!(chunks.len() > 3, "expected several chunks, got {}", chunks.len());
        // Tiling: chunk k+1 starts exactly where chunk k ended.
        for pair in chunks.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
    }

    #[test]
    fn one_giant_line_is_hard_cut() {
        let tk = tk();
        let cfg = ChunkCfg::default();
        let text = "x".repeat(20_000);
        let chunks = assert_lossless(&tk, &text, &cfg);
        assert!(chunks.len() > 1);
    }

    #[test]
    fn cjk_and_no_trailing_newline() {
        let tk = tk();
        let cfg = ChunkCfg::default();
        let text = "这是一个很长的思考过程，没有任何换行符，需要按字符硬切。".repeat(200);
        let chunks = assert_lossless(&tk, &text, &cfg);
        assert!(chunks.len() > 1);
    }

    #[test]
    fn text_past_the_tokenizer_truncation_limit_is_still_tiled() {
        // The shipped tokenizer.json truncates at 32768 tokens. Before `open`
        // stripped that, the one-shot encode stopped at the limit and the whole
        // tail of the row became a single "chunk" (430k-char reasoning row:
        // chunk 75 alone was 317k chars). `assert_lossless` checks the real
        // contract: every chunk is ≤ budget.
        let tk = tk();
        let cfg = ChunkCfg::default();
        let text = "这是一段很长的思考过程，需要按字符硬切，不能有任何字符丢失。".repeat(6000);
        assert!(
            token_len(&tk, &text) > 32_768,
            "test text must exceed the tokenizer's shipped truncation limit"
        );
        let chunks = assert_lossless(&tk, &text, &cfg);
        assert!(chunks.len() > 100, "expected many chunks, got {}", chunks.len());
    }
}
