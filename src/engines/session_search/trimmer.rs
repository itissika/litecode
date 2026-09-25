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
    use super::*;

    use crate::authority::responses::{FunctionCallOutput, FunctionCallOutputItemParam};
    use crate::session::transcript_file::SearchableRow;
    use crate::types::{Item, user_text};

    use super::super::sparse;

    /// Trim as the index does: `start` is arbitrary and the span is the text.
    fn trimmed(text: &str, role: ContentRole) -> (usize, &str) {
        trim_span(text, 7, 7 + text.chars().count(), role)
    }

    #[test]
    fn a_result_keeps_only_its_head() {
        let text: String = "x".repeat(OUTCOME_CHARS * 3);
        let (end, kept) = trimmed(&text, ContentRole::Outcome);
        assert_eq!(kept.chars().count(), OUTCOME_CHARS);
        assert_eq!(end, 7 + OUTCOME_CHARS, "the span ends where the text does");
        assert!(text.starts_with(kept), "the kept range is a prefix");
    }

    #[test]
    fn a_result_under_the_budget_is_left_alone() {
        let text = "a short result";
        assert_eq!(trimmed(text, ContentRole::Outcome), (7 + text.len(), text));
    }

    /// The line a hit renders from is the line it points at, so nothing that
    /// carries intent is allowed to move.
    #[test]
    fn everything_that_is_not_a_result_is_untouched() {
        let text: String = "说".repeat(OUTCOME_CHARS * 3);
        let end = 7 + text.chars().count();
        for role in [
            ContentRole::Conversation,
            ContentRole::Reasoning,
            ContentRole::Action,
            ContentRole::Unknown,
        ] {
            assert_eq!(trimmed(&text, role), (end, text.as_str()), "{role:?}");
        }
    }

    /// The budget is chars: a multi-byte row must not be cut mid-character.
    #[test]
    fn the_cut_lands_on_a_character_boundary() {
        let text = "结".repeat(OUTCOME_CHARS + 1);
        let (_, kept) = trimmed(&text, ContentRole::Outcome);
        assert_eq!(kept.chars().count(), OUTCOME_CHARS);
        assert!(kept.chars().all(|c| c == '结'));
    }

    fn row(sid: &str, seq: i64, kind: &str, item: &Item) -> SearchableRow {
        let value = serde_json::to_value(item).expect("serialize item");
        let item_type = value
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("unknown")
            .to_string();
        SearchableRow {
            session_id: sid.into(),
            seq,
            kind: kind.into(),
            item_type,
            body: Some(value.to_string()),
            body_ref: None,
        }
    }

    fn result(sid: &str, seq: i64, output: &str) -> SearchableRow {
        row(
            sid,
            seq,
            "item/tool_result",
            &Item::FunctionCallOutput(FunctionCallOutputItemParam {
                call_id: "c1".into(),
                output: FunctionCallOutput::Text(output.into()),
                id: None,
                status: None,
            }),
        )
    }

    /// `(char_start, char_end, text)` for every chunk of one row, in order.
    fn chunks(dir: &std::path::Path, seq: i64) -> Vec<(i64, i64, String)> {
        let conn = rusqlite::Connection::open(sparse::sparse_index_path(dir)).expect("open");
        let mut stmt = conn
            .prepare("SELECT char_start, char_end, text FROM rows WHERE seq = ?1 ORDER BY chunk")
            .unwrap();
        let rows = stmt
            .query_map([seq], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap();
        rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
    }

    /// End to end, through the real builder: a result loses its tail in the
    /// index, a message keeps every char, and in both cases the stored span
    /// still measures exactly the stored text.
    ///
    /// That last part is the one that matters. A hit renders its physical line
    /// from `char_start`, so a span that disagreed with its text would point the
    /// reader at the wrong line — and the line is the navigation.
    #[test]
    fn the_index_keeps_a_results_head_and_a_messages_whole_body() {
        let dir = tempfile::TempDir::new().unwrap();
        let body: String = "segment: the quick brown fox jumps over the lazy dog.\n"
            .repeat(200)
            .to_string();
        let rows = vec![
            result("S1", 0, &body),
            row("S1", 1, "item/user", &user_text(&body)),
        ];
        sparse::build_index(&rows, dir.path()).expect("build index");

        let cut = chunks(dir.path(), 0);
        assert!(cut.len() >= 2, "a long result is more than one chunk");
        for (start, end, text) in cut {
            assert_eq!(text.chars().count(), OUTCOME_CHARS, "the tail is gone");
            assert_eq!(end - start, OUTCOME_CHARS as i64, "the span ends with it");
            let source: String = body.chars().skip(start as usize).take(OUTCOME_CHARS).collect();
            assert_eq!(text, source, "a result chunk is still its own head");
        }

        let whole = chunks(dir.path(), 1);
        let kept: String = whole.iter().map(|(_, _, text)| text.as_str()).collect();
        // Compared with trailing whitespace aside: the chunker already drops the
        // newline at the very end of a row, and that is not this change.
        assert_eq!(kept.trim_end(), body.trim_end(), "a message is not trimmed");
        assert!(kept.chars().count() > OUTCOME_CHARS * 2, "nor cut to a head");
        for (start, end, text) in whole {
            assert_eq!(end - start, text.chars().count() as i64, "its span is intact");
        }
    }
}
