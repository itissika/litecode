//! Hard gate: forbid residual death-list dialect tokens under `src/`.
//!
//! This file may name the banned needles. `authority.rs` / `transcript.rs` may
//! mention them only in hard-rule comments (lines that are `//` / `//!` comments).
//!
//! Phase 2: Chat Completions / homemade Chat stream dialect is banned under `src/`
//! outside `llm/adapter/` (`chat/completions` and `reasoning_content` are allowed
//! inside the Chat Completions codec).
//!
//! Phase 5: homemade L2 stream deltas (`TextDelta` / `ReasoningDelta` /
//! `ToolCallStreaming`), method `buffer/message`, and dual `messages` load keys
//! are banned outside `llm/adapter/` (this gate file exempt).

use std::fs;
use std::path::{Path, PathBuf};

fn walk_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_rs_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn is_comment_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//") || t.starts_with("//!") || t.starts_with("///")
}

/// Session seq/surface (ticket 01): catalogued for later G3 full-src scan. Not asserted yet.
const SESSION_SEQ_SURFACE_NEEDLES: &[&str] = &[
    "buffer_index",
    "bufferIndex",
    "kept_from_seq",
    "checkpoint_seq",
    "compact_checkpoint",
    "liveItemRowId",
    "orderProjection",
    "committedIdentity",
];

fn mental_model_or_spec_text() -> Option<String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidates = [
        root.join("dev/plans/session-seq-surface/MENTAL-MODEL.md"),
        root.join(".scratch/session-seq-surface/spec.md"),
        root.join("src/session/DATA.md"),
    ];
    for path in candidates {
        if let Ok(text) = fs::read_to_string(&path) {
            return Some(text);
        }
    }
    None
}

#[test]
fn session_seq_g1_envelope_vocab_matches_mental_model() {
    let event_src =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/session/event.rs"))
            .expect("event.rs");
    assert!(
        event_src.contains("pub seq: Seq") || event_src.contains("pub seq: u64"),
        "SessionEvent.seq must exist"
    );
    assert!(
        event_src.contains("pub surface_op"),
        "SessionEvent.surface_op must exist"
    );
    assert!(
        event_src.contains("pub source_seqs"),
        "SessionEvent.source_seqs must exist"
    );
    assert!(
        !event_src.contains("buffer_index"),
        "new envelope module must not take buffer_index as identity"
    );

    let Some(doc) = mental_model_or_spec_text() else {
        panic!("session domain documentation must exist for G1 vocab");
    };
    for needle in ["SessionEvent", "type", "source_seqs"] {
        assert!(
            doc.contains(needle),
            "domain doc must name `{needle}` so types stay aligned"
        );
    }
    assert!(
        SESSION_SEQ_SURFACE_NEEDLES.contains(&"buffer_index"),
        "A/B/C needles must be catalogued (full scan is G3)"
    );

    use crate::session::event::{EventDraft, EventLog, EventType};
    use crate::session::surface::{SurfaceOp, derive_messages, derive_transcript_items};
    use crate::types::{item_text_preview, user_text};

    let mut log = EventLog::new();
    for text in ["d0", "d1", "d2", "d3", "d4"] {
        log.append(
            EventDraft::surface_item(EventType::ItemUser, &user_text(text), SurfaceOp::Append)
                .expect("draft"),
        )
        .expect("append");
    }
    let mut summary = EventDraft::surface_item(
        EventType::ItemUser,
        &user_text("summary"),
        SurfaceOp::Replace { start: 0, end: 1 },
    )
    .expect("draft");
    summary.source_seqs = Some(vec![0, 1]);
    log.append(summary).expect("replace");
    let texts: Vec<_> = derive_messages(log.events())
        .expect("derive_messages")
        .iter()
        .map(item_text_preview)
        .collect();
    assert_eq!(texts, vec!["summary", "d2", "d3", "d4"]);
    let t: Vec<_> = derive_transcript_items(log.events())
        .expect("transcript")
        .iter()
        .map(item_text_preview)
        .collect();
    assert_eq!(t, vec!["d0", "d1", "d2", "d3", "d4"]);
}

/// ISSUE-013 / g2: after Compact persist, working set reloads from fold — not an
/// in-memory pi summary+kept reconstruction. Pipeline must not use `checkpoint_seq()`
/// as the compact/commit cursor.
///
/// Behavior coverage (product invariant):
/// - `tests/working_set_identity.rs::keep_recent_compact_reader_matches_fold_checkpoint_then_kept`
/// - `tests/stage_a_ctx_consistency.rs::pipeline_compact_reloads_working_set_from_fold`
///
/// This gate keeps item-level + dialect rails so a silent regression cannot
/// reintroduce the old cursor or the pi-view rebuild path.
#[test]
fn session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let compact_src =
        fs::read_to_string(root.join("src/context_pipeline/compact.rs")).expect("compact.rs");
    let compact = production_lines(&compact_src);

    // Item-level: compact_transcript persists via SessionMutation::Compact, then
    // reloads the working set from fold (`working_set_blocking`) — not a hand-built
    // summary+kept view.
    let body = fn_item_body(&compact, "pub async fn compact_transcript");
    let persist = body
        .find("SessionMutation::Compact")
        .expect("compact_transcript must persist via SessionMutation::Compact");
    assert!(
        body[persist..].contains("working_set_blocking"),
        "after Compact persist, compact_transcript must reload via working_set_blocking (fold)"
    );

    // Thin dialect ban across context_pipeline (same scan style as g4/g6).
    let forbidden: &[(&str, &str)] = &[
        (
            "checkpoint_seq()",
            "pipeline must not use checkpoint_seq() as compact/commit cursor",
        ),
        (
            "Align in-memory working set with the pi view",
            "must not reconstruct the pi summary+kept view after persist",
        ),
    ];
    let mut files = Vec::new();
    walk_rs_files(&root.join("src/context_pipeline"), &mut files);
    files.sort();
    let mut hits = Vec::new();
    for path in &files {
        let contents = fs::read_to_string(path).unwrap_or_default();
        let mut in_tests = false;
        for (i, line) in contents.lines().enumerate() {
            if line.trim() == "#[cfg(test)]" {
                in_tests = true;
            }
            if in_tests || is_comment_line(line) {
                continue;
            }
            for (needle, why) in forbidden {
                if line.contains(needle) {
                    hits.push(format!("{}:{}: `{needle}` ({why})", path.display(), i + 1));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "g2 dialect residuals in context_pipeline:\n{}",
        hits.join("\n")
    );
}

/// Body of the first item whose signature contains `sig`, through the next
/// sibling item at the same indent (g6-style). Pass `production_lines` input so
/// cfg(test) stubs cannot satisfy a production-path gate.
fn fn_item_body<'a>(source: &'a str, sig: &str) -> &'a str {
    let start = source
        .find(sig)
        .unwrap_or_else(|| panic!("missing item `{sig}`"));
    let body = &source[start..];
    let end = [
        "\n    pub async fn ",
        "\n    pub(crate) async fn ",
        "\n    pub fn ",
        "\n    async fn ",
        "\n    fn ",
        "\nfn ",
    ]
    .into_iter()
    .filter_map(|marker| body[1..].find(marker).map(|i| i + 1))
    .min()
    .unwrap_or(body.len());
    &body[..end]
}

/// Source lines that ship, with `#[cfg(test)]` bodies removed.
///
/// A gate about the production path must not be satisfied or tripped by a test
/// that happens to mention the same name.
fn production_lines(contents: &str) -> String {
    let mut out = String::new();
    let mut test_depth: i32 = 0;
    let mut pending_test_mod = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[cfg(test)]") {
            pending_test_mod = true;
            continue;
        }
        if pending_test_mod && trimmed.starts_with("mod ") {
            pending_test_mod = false;
            if trimmed.ends_with(';') {
                continue;
            }
            test_depth = 1;
            continue;
        }
        if test_depth > 0 {
            test_depth += trimmed.matches('{').count() as i32;
            test_depth -= trimmed.matches('}').count() as i32;
            if test_depth <= 0 {
                test_depth = 0;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}
