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




/// True if `needle` appears as an identifier token (not as a suffix of a longer ident).
fn identifier_boundary_contains(line: &str, needle: &str) -> bool {
    let mut search_from = 0;
    while let Some(rel) = line[search_from..].find(needle) {
        let idx = search_from + rel;
        let before_ok = idx == 0
            || !line
                .as_bytes()
                .get(idx - 1)
                .copied()
                .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_');
        let after = idx + needle.len();
        // Needles ending in `::` are path prefixes; only require a leading boundary.
        let after_ok = if needle.ends_with("::") {
            true
        } else {
            line.as_bytes()
                .get(after)
                .copied()
                .map(|b| !b.is_ascii_alphanumeric() && b != b'_')
                .unwrap_or(true)
        };
        if before_ok && after_ok {
            return true;
        }
        search_from = idx + needle.len();
    }
    false
}












/// ISSUE-013: compact LLM calls must go through the unified stream provider entry
/// (`complete_with_stream_events`). context_pipeline must not invent vendor thinking
/// strings or call a raw `provider.complete(`.
///
/// Behavior coverage: `src/context_pipeline/compact.rs` unit tests
/// (`compact_request_returns_summary_on_provider_success` and siblings) exercise
/// `call_llm_compact_with_timeout` via `LlmProvider::complete_with_stream_events`
/// stubs. This gate keeps an item-level symbol check on the production helper plus
/// dialect bans (same style as other death-list forbidden-token scans).
#[test]
fn compact_llm_path_does_not_bypass_unified_provider() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let compact_src =
        fs::read_to_string(root.join("src/context_pipeline/compact.rs")).expect("compact.rs");
    let compact = production_lines(&compact_src);

    // Item-level / symbol: the production compact LLM helper must reference the
    // unified stream entry — not a whole-file "must contain" substring.
    let body = fn_item_body(&compact, "fn call_llm_compact_with_timeout");
    assert!(
        body.contains("complete_with_stream_events"),
        "call_llm_compact_with_timeout must send through complete_with_stream_events"
    );
    let stripped = body.replace("complete_with_stream_events", "");
    assert!(
        !stripped.contains("provider.complete("),
        "call_llm_compact_with_timeout must not call provider.complete("
    );

    // Dialect ban: context_pipeline must not invent vendor thinking strings or
    // call complete() (keep/refine prior forbidden-token style).
    let mut pipeline_hits = Vec::new();
    let mut pipeline_files = Vec::new();
    walk_rs_files(&root.join("src/context_pipeline"), &mut pipeline_files);
    for path in pipeline_files {
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        // Skip cfg(test) modules so stub impls of complete_with_stream_events do
        // not dilute the production dialect check.
        let prod = production_lines(&text);
        let stripped = prod.replace("complete_with_stream_events", "");
        if prod.contains("thinking_mode:")
            || prod.contains("reasoning_effort:")
            || stripped.contains("provider.complete(")
        {
            pipeline_hits.push(rel);
        }
    }
    assert!(
        pipeline_hits.is_empty(),
        "context_pipeline must not invent vendor thinking strings or call complete(): {pipeline_hits:?}"
    );

    let mut hits = Vec::new();
    let mut files = Vec::new();
    walk_rs_files(&root.join("src"), &mut files);
    for path in files {
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if rel.starts_with("src/llm/codec/") || rel == "src/types/death_list_gate.rs" {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let prod = production_lines(&text);
        let stripped = prod.replace("complete_with_stream_events", "");
        if stripped.contains(".complete(&request")
            || stripped.contains("provider.complete(")
            || stripped.contains("LlmProvider::complete")
        {
            hits.push(rel);
        }
    }
    assert!(
        hits.is_empty(),
        "non-adapter complete() leftover in {hits:?}"
    );

    // Thinking literals live in the catalog now: a codec may only know the named
    // placements, never a vendor's field vocabulary.
    let responses_codec =
        fs::read_to_string(root.join("src/llm/codec/responses.rs")).expect("responses.rs");
    assert!(
        !responses_codec.contains("thinking_mode"),
        "the responses codec must not know a vendor thinking field"
    );
    assert!(
        !responses_codec.contains("\"reasoning_effort\""),
        "reasoning_effort belongs to the chat codec"
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


/// A streamed item's row is opened, updated and settled — never appended.
///
/// The generic item write is the one that produced two rows for one provider
/// item: a snapshot appended at `output_item.added` and the settled copy appended
/// again by the commit. The projection must reach the log only through the
/// lifecycle commands, so this gate trips the moment the append comes back.
#[test]
fn stream_projection_uses_only_the_lifecycle_writes() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/runtime/stream_projection.rs");
    let contents = fs::read_to_string(&path).expect("stream_projection.rs");
    let body = production_lines(&contents);

    assert!(
        body.contains("begin_stream_item")
            && body.contains("update_stream_item")
            && body.contains("seal_stream_item"),
        "the projection must drive the explicit lifecycle commands"
    );

    let hits: Vec<String> = body
        .lines()
        .enumerate()
        .filter(|(_, line)| identifier_boundary_contains(line, "persist_item"))
        .map(|(i, _)| format!("{}:{}: `persist_item`", path.display(), i + 1))
        .collect();
    assert!(
        hits.is_empty(),
        "a streamed item must not go through the generic append:
{}",
        hits.join(
            "
"
        )
    );
}
