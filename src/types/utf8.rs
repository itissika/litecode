//! UTF-8 boundary-safe prefix and suffix cuts.
//!
//! Byte indexes from budgets land inside multibyte characters. These helpers
//! move the cut onto a character boundary so slicing user text cannot panic.

/// Longest prefix of `text` whose byte length is at most `max`.
pub fn prefix_utf8(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Longest suffix of `text` whose byte length is at most `max`.
pub fn suffix_utf8(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut start = text.len() - max;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

#[cfg(test)]
mod tests {}
