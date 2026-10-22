//! How much of a row the sparse index is allowed to keep.
//!
//! The index is a memory, not a second copy of the filesystem. Tool results are
//! 46% of its rows and half its text, and a machine listing repeats the query
//! string once per line it lists — which is enough repetition to outrank what a
//! person actually said. So a result contributes only its head.
//!
//! Nothing else is trimmed. What was said, what was thought, and what was called
//! are the record; the output of a call is a by-product of it.
//!
//! Head only, and that is the point. The kept range is a *prefix* of the chunk,
//! so the chunk's `start` and every offset inside its text still name the same
//! place in the source. A hit renders its physical line from `char_start`, so a
//! span that drifted would point a reader at the wrong line — the line number is
//! what the caller navigates by. The tail is dropped on purpose: the caller can
//! open the row and read upwards from the cut.

use super::ranking::ContentRole;

/// Chars of an `Outcome` chunk the index keeps.
///
/// The same order as a conversational turn (messages: median 107 chars, p90
/// 947) and far below a machine listing (results: median 1133). Measured on the
/// live index: at 300, result text falls to 32% of its bytes and the number of
/// rows that still match a query falls by about a fifth.
const OUTCOME_CHARS: usize = 400;

/// The span end and text an indexed chunk should carry, given its role.
///
/// Returns `(end, text)`. The caller keeps the chunk's own `start`, so a trimmed
/// row still begins exactly where it always did.
pub fn trim_span(text: &str, start: usize, end: usize, role: ContentRole) -> (usize, &str) {
    if role != ContentRole::Outcome {
        return (end, text);
    }
    // A chunk's text is exactly `source[start..end]`, so `end - start` chars is
    // what it holds. Cut the text at the budget and move `end` with it.
    let Some((cut, _)) = text.char_indices().nth(OUTCOME_CHARS) else {
        return (end, text);
    };
    (start + OUTCOME_CHARS, &text[..cut])
}

#[cfg(test)]
mod tests {
    

    
    
    

    

}
