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

/// Forbidden substrings (exact). Kept here so the gate file may mention them.
/// Phase 1 kernel dialect — banned everywhere under `src/` (except exemptions).
const FORBIDDEN_EXACT: &[&str] = &[
    "ContentBlockStart",
    "ContentBlockStop",
    "UserContentPart",
    "assemble_assistant_blocks",
    "struct StreamOutput",
    "messages_to_llm_format",
    "Message::User",
    "Message::Assistant",
    "ContentBlock::",
    "reasoning_content",
    "chat/completions",
    "sse_parse_openai",
    "sse_parse_deepseek",
    "OpenaiChatProvider",
    "pub enum StreamEvent",
    "ToolCallDelta",
    "deepseek_chat",
];

/// Phase 5: homemade stream / buffer dialect — outside adapter only.
/// Matched with identifier boundaries so `ResponseTextDeltaEvent` is allowed.
const PHASE5_OUTSIDE_ADAPTER: &[&str] = &[
    "TextDelta",
    "ReasoningDelta",
    "ToolCallStreaming",
    "buffer/message",
];

/// Phase 5: dual `buffer/load` keys — banned in `client_protocol/`.
const PHASE5_CLIENT_PROTOCOL_EXACT: &[&str] = &["\"messages\": items", "\"messages\": items_value"];

/// Phase 3: chat JSON construction banned outside adapter (and authority/transcript
/// serde smoke strings). Matches the old PreparedView.formatted / tail_reminders wire shape.
const CHAT_ROLE_USER_NEEDLE: &str = "\"role\": \"user\"";

/// Phase 3: PreparedView must not resurrect a `formatted` chat JSON field.
const FORBIDDEN_LLMVIEW_FORMATTED: &str = "pub formatted";

/// R3: fabricated Chat item / call ids — banned everywhere under `src/` (including adapter).
/// This gate file may list them; production code must allocate turn-stable / provider ids.
const FORBIDDEN_FAKE_CHAT_IDS: &[&str] = &[
    "msg_chat_stream",
    "rs_chat_stream",
    "msg_chat_translate",
    "rs_chat_translate",
    "call_chat_",
];

/// R4: ToolStart/ToolEnd conversation-semantic bypass — banned under `src/` (this gate exempt).
const FORBIDDEN_R4_TOOL_BYPASS: &[&str] = &[
    "ToolStarted",
    "ToolFinished",
    "WireEvent::ToolStart",
    "WireEvent::ToolEnd",
    "tool_start",
    "tool_end",
];

/// R6: persistence / RPC isomorphic with Item transcript — banned under `src/` (this gate exempt).
const FORBIDDEN_R6_PERSISTENCE_RPC: &[&str] = &[
    "FROM messages",
    "INTO messages",
    "TABLE messages",
    "ensure_messages_schema",
    "revert_messages",
    "RevertMessages",
    "session/revert-messages",
    "[snipped: tool result without matching function_call]",
    "message_role(",
];

/// R9: peripheral dialect / renamed APIs must not regress under `src/` (this gate exempt).
const FORBIDDEN_R9_PERIPHERAL: &[&str] = &[
    "HookMessage",
    "inject_messages",
    "custom_tool_to_legacy",
    "assemble_system_prompt",
    "compact_messages",
    "CustomToolConfig",
];

/// R9: `LlmView` renamed to `PreparedView` — ban bare identifier (not suffixes).
const FORBIDDEN_R9_LLMVIEW: &str = "LlmView";

/// Model Selection Contract (`docs/model-selection-contract.md`): killed symbols.
const FORBIDDEN_MODEL_SELECTION: &[&str] = &[
    "model_override",
    "effective_api_model",
    "session_effective_model",
    "set_model_override",
    "resolve_turn_llm",
    "using model_ref as api id",
];

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

fn line_has_enum_content_block(line: &str) -> bool {
    let Some(idx) = line.find("enum ContentBlock") else {
        return false;
    };
    let before_ok = idx == 0
        || !line
            .as_bytes()
            .get(idx - 1)
            .copied()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_');
    let after = idx + "enum ContentBlock".len();
    let after_ok = line
        .as_bytes()
        .get(after)
        .copied()
        .map(|b| !b.is_ascii_alphanumeric() && b != b'_')
        .unwrap_or(true);
    before_ok && after_ok
}

/// Match bare `SseFormat` but not `ProviderSseFormat`.
fn line_has_bare_sse_format(line: &str) -> bool {
    identifier_boundary_contains(line, "SseFormat")
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

fn check_types_pub_enum_message(path: &Path, contents: &str) -> Vec<String> {
    let rel = path_slash(path);
    if !rel.contains("/types/") {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for (i, line) in contents.lines().enumerate() {
        if is_comment_line(line) {
            continue;
        }
        if line.contains("pub enum Message") {
            hits.push(format!(
                "{}:{}: forbidden `pub enum Message` in types/",
                path.display(),
                i + 1
            ));
        }
    }
    hits
}

fn skip_file_entirely(path: &Path) -> bool {
    path.file_name().and_then(|s| s.to_str()) == Some("death_list_gate.rs")
}

fn allow_hard_rule_comment_mentions(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|s| s.to_str()),
        Some("authority.rs" | "transcript.rs")
    )
}

fn path_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn is_adapter_path(path: &Path) -> bool {
    path_slash(path).contains("/llm/codec/")
}

/// The provider catalog states protocol facts (the request path, the replay key)
/// as data; it never builds a Chat dialect.
fn is_provider_catalog_path(path: &Path) -> bool {
    path_slash(path).contains("/provider_catalog/")
}

fn is_client_protocol_path(path: &Path) -> bool {
    path_slash(path).contains("/client_protocol/")
}

/// Authority / transcript may embed Responses JSON with `"role": "user"` in tests.
fn allow_chat_role_user_json(path: &Path) -> bool {
    is_adapter_path(path)
        || matches!(
            path.file_name().and_then(|s| s.to_str()),
            Some("authority.rs" | "transcript.rs")
        )
}

fn check_file(path: &Path) -> Vec<String> {
    if skip_file_entirely(path) {
        return Vec::new();
    }
    let Ok(contents) = fs::read_to_string(path) else {
        return vec![format!("{}: failed to read", path.display())];
    };
    let hard_rule_comments_ok = allow_hard_rule_comment_mentions(path);
    let in_adapter = is_adapter_path(path) || is_provider_catalog_path(path);
    let in_client_protocol = is_client_protocol_path(path);
    let allow_role_user = allow_chat_role_user_json(path);
    let mut hits = Vec::new();
    for (i, line) in contents.lines().enumerate() {
        if hard_rule_comments_ok && is_comment_line(line) {
            continue;
        }
        for needle in FORBIDDEN_EXACT {
            if *needle == "chat/completions" || *needle == "reasoning_content" {
                if in_adapter {
                    continue;
                }
            }
            if line.contains(needle) {
                hits.push(format!(
                    "{}:{}: forbidden `{needle}`",
                    path.display(),
                    i + 1
                ));
            }
        }
        for needle in FORBIDDEN_FAKE_CHAT_IDS {
            if line.contains(needle) {
                hits.push(format!(
                    "{}:{}: R3 forbidden fake Chat id `{needle}`",
                    path.display(),
                    i + 1
                ));
            }
        }
        for needle in FORBIDDEN_R4_TOOL_BYPASS {
            let hit = if needle.ends_with("Started")
                || needle.ends_with("Finished")
                || needle.contains("::")
            {
                identifier_boundary_contains(line, needle)
            } else {
                line.contains(needle)
            };
            if hit {
                hits.push(format!(
                    "{}:{}: R4 forbidden ToolStart/ToolEnd bypass `{needle}`",
                    path.display(),
                    i + 1
                ));
            }
        }
        for needle in FORBIDDEN_R6_PERSISTENCE_RPC {
            if line.contains(needle) {
                hits.push(format!(
                    "{}:{}: R6 forbidden persistence/RPC dialect `{needle}`",
                    path.display(),
                    i + 1
                ));
            }
        }
        for needle in FORBIDDEN_R9_PERIPHERAL {
            if line.contains(needle) {
                hits.push(format!(
                    "{}:{}: R9 forbidden peripheral dialect `{needle}`",
                    path.display(),
                    i + 1
                ));
            }
        }
        if !is_comment_line(line) {
            for needle in FORBIDDEN_MODEL_SELECTION {
                let hit = if needle.contains(' ') {
                    line.contains(needle)
                } else {
                    identifier_boundary_contains(line, needle)
                };
                if hit {
                    hits.push(format!(
                        "{}:{}: model-selection contract forbidden `{needle}`",
                        path.display(),
                        i + 1
                    ));
                }
            }
        }
        if identifier_boundary_contains(line, FORBIDDEN_R9_LLMVIEW) {
            hits.push(format!(
                "{}:{}: R9 forbidden `{FORBIDDEN_R9_LLMVIEW}` (use PreparedView)",
                path.display(),
                i + 1
            ));
        }
        if line.contains(FORBIDDEN_LLMVIEW_FORMATTED) {
            hits.push(format!(
                "{}:{}: forbidden `{FORBIDDEN_LLMVIEW_FORMATTED}` (Phase 3: no PreparedView.formatted)",
                path.display(),
                i + 1
            ));
        }
        if !allow_role_user && !is_comment_line(line) && line.contains(CHAT_ROLE_USER_NEEDLE) {
            hits.push(format!(
                "{}:{}: forbidden chat JSON `{CHAT_ROLE_USER_NEEDLE}` (only llm/adapter/ or authority/transcript smoke)",
                path.display(),
                i + 1
            ));
        }
        if line_has_enum_content_block(line) {
            hits.push(format!(
                "{}:{}: forbidden `enum ContentBlock`",
                path.display(),
                i + 1
            ));
        }

        if line_has_bare_sse_format(line) {
            hits.push(format!(
                "{}:{}: forbidden `SseFormat` (Chat SSE dialect removed)",
                path.display(),
                i + 1
            ));
        }
        if identifier_boundary_contains(line, "StreamEvent::") {
            hits.push(format!(
                "{}:{}: forbidden homemade `StreamEvent::`",
                path.display(),
                i + 1
            ));
        }

        if !in_adapter {
            for needle in PHASE5_OUTSIDE_ADAPTER {
                let hit = if *needle == "buffer/message" {
                    line.contains(needle)
                } else {
                    identifier_boundary_contains(line, needle)
                };
                if hit {
                    hits.push(format!(
                        "{}:{}: Phase 5 forbidden `{needle}` (outside llm/adapter/)",
                        path.display(),
                        i + 1
                    ));
                }
            }
        }

        if in_client_protocol {
            for needle in PHASE5_CLIENT_PROTOCOL_EXACT {
                if line.contains(needle) {
                    hits.push(format!(
                        "{}:{}: Phase 5 dual-messages key `{needle}` banned in client_protocol/",
                        path.display(),
                        i + 1
                    ));
                }
            }
        }
    }
    hits.extend(check_types_pub_enum_message(path, &contents));
    hits
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
    for needle in ["SessionLog", "kind", "cites"] {
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

#[test]
fn session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let compact =
        fs::read_to_string(root.join("src/context_pipeline/compact.rs")).expect("compact.rs");
    let persist = compact
        .find("SessionMutation::Compact")
        .expect("compact persist must go through SessionMutation::Compact");
    assert!(
        compact[persist..].contains("working_set_blocking"),
        "after compact persist, working set must reload from fold"
    );
    assert!(
        !compact.contains("Align in-memory working set with the pi view"),
        "must not reconstruct the pi summary+kept view after persist"
    );
    let pipeline = fs::read_to_string(root.join("src/context_pipeline/mod.rs")).expect("mod.rs");
    assert!(
        !pipeline.contains("checkpoint_seq()"),
        "pipeline must not use checkpoint_seq as the compact/commit cursor"
    );
}

#[test]
fn compact_llm_path_does_not_bypass_unified_provider() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let compact =
        fs::read_to_string(root.join("src/context_pipeline/compact.rs")).expect("compact.rs");
    assert!(
        compact.contains("complete_with_stream_events"),
        "compact must send through the unified stream provider entry"
    );

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
        let stripped = text.replace("complete_with_stream_events", "");
        if text.contains("thinking_mode:")
            || text.contains("reasoning_effort:")
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
        let stripped = text.replace("complete_with_stream_events", "");
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

/// Ticket 05: search/revert must not use SQL window pointers as authority.
#[test]
fn session_seq_g6_derived_paths_use_surface_not_pointers() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    walk_rs_files(&root.join("src/engines/session_search"), &mut files);
    files.push(root.join("src/tools/session_search.rs"));
    files.sort();

    let needles = ["kept_from_seq", "checkpoint_seq", "compact_checkpoint"];
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
            for needle in needles {
                if identifier_boundary_contains(line, needle) {
                    hits.push(format!("{}:{}: `{needle}`", path.display(), i + 1));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "search still uses window pointers:\n{}",
        hits.join("\n")
    );

    let manager = fs::read_to_string(root.join("src/session/manager.rs")).expect("manager.rs");
    let sqlite =
        fs::read_to_string(root.join("src/session/data/sqlite/session.rs")).expect("session.rs");
    for (label, source, sig) in [
        (
            "entry_revert_to_user_anchor",
            manager.as_str(),
            "pub fn entry_revert_to_user_anchor",
        ),
        (
            "Session::revert_to_user_anchor",
            sqlite.as_str(),
            "pub fn revert_to_user_anchor",
        ),
    ] {
        let start = source.find(sig).unwrap_or_else(|| panic!("{label}"));
        let body = &source[start..];
        let end = body[1..]
            .find("\n    pub fn ")
            .map(|i| i + 1)
            .unwrap_or(body.len());
        let revert = &body[..end];
        for needle in needles {
            assert!(
                !revert.contains(needle),
                "{label} must not use `{needle}` as a window pointer"
            );
        }
    }
}
#[test]
fn session_seq_g4_wire_speaks_seq() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    walk_rs_files(&root.join("src/client_protocol"), &mut files);
    files.push(root.join("src/runtime/observer.rs"));
    files.sort();

    let needles = [
        "buffer_index",
        "bufferIndex",
        "kept_from_seq",
        "checkpoint_seq",
        "compact_checkpoint",
        "committed_end",
    ];
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
            for needle in needles {
                if identifier_boundary_contains(line, needle) {
                    hits.push(format!("{}:{}: `{needle}`", path.display(), i + 1));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "wire/observer still speak buffer_index dialect:\n{}",
        hits.join("\n")
    );
}

#[test]
fn death_list_dialect_tokens_absent_from_src() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(src.is_dir(), "src/ missing at {}", src.display());

    let mut files = Vec::new();
    walk_rs_files(&src, &mut files);
    files.sort();

    let mut violations = Vec::new();
    for path in &files {
        violations.extend(check_file(path));
    }

    assert!(
        violations.is_empty(),
        "death-list dialect residuals still under src/:\n{}",
        violations.join("\n")
    );
}

/// The provider catalog is the only source of LLM provider/model facts.
///
/// Legacy knowledge is confined to the one-shot v6 migration module and to the
/// archived schema comments; everything else must go through the catalog.
#[test]
fn death_list_provider_catalog_is_the_only_llm_source() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    walk_rs_files(&root.join("src"), &mut files);

    // Files allowed to know the removed provider/model registry.
    fn legacy_exempt(rel: &str) -> bool {
        matches!(
            rel,
            "src/config/global_db/legacy.rs"
                | "src/config/global_db/migrate.rs"
                | "src/types/death_list_gate.rs"
        )
    }

    const FORBIDDEN: &[&str] = &[
        "adapter_id",
        "ADAPTER_OPENAI_RESPONSES",
        "ADAPTER_DEEPSEEK_RESPONSES",
        "ADAPTER_MIMO_RESPONSES",
        "ADAPTER_OPENCODE",
        "ADAPTER_ARK_CODING",
        "ADAPTER_COMMANDCODE",
        "list_adapters",
        "AdapterDescriptor",
        "FieldSchema",
        "ProviderDefinition",
        "ProviderConnectionConfig",
        "ProviderAuth",
        "ModelDefinition",
        "ModelAdapterConfig",
        "ModelCapability",
        "remote_model_catalog",
        "has_remote_model_catalog",
        "catalog_supported_ids",
        "closed_api_model_ids",
        "closed_context_windows",
        "closed_default_endpoint",
        "parse_provider_config",
        "parse_model_config",
        "apply_owned_modality_capabilities",
        "provider_from_definition",
        "chat_models_url",
        "chat_post_url",
        "parse_chat_model_catalog",
        "map_thinking_to_wire",
        "llm_ecosystem",
        "catalog_count",
        "DocId::Models",
        "write_models",
    ];

    let mut hits = Vec::new();
    for path in &files {
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        if legacy_exempt(&rel) {
            continue;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            if is_comment_line(line) {
                continue;
            }
            for needle in FORBIDDEN {
                if line.contains(needle) {
                    hits.push(format!("{rel}:{}: {needle} -> {}", index + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "legacy LLM registry residuals under src/:\n{}",
        hits.join("\n")
    );
}

/// Vendor model-catalog fetching is gone: no HTTP call may target one.
#[test]
fn death_list_no_vendor_model_catalog_fetch() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    walk_rs_files(&root.join("src"), &mut files);

    let mut hits = Vec::new();
    for path in &files {
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            if is_comment_line(line) {
                continue;
            }
            let lowered = line.to_ascii_lowercase();
            let fetches = lowered.contains(".get(")
                || lowered.contains("reqwest::client")
                || lowered.contains("send().await");
            if fetches && (lowered.contains("/models") || lowered.contains("models_get_url")) {
                hits.push(format!("{rel}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "vendor model-catalog fetch residuals:\n{}",
        hits.join("\n")
    );
}

/// A codec never branches on provider identity, and every EndpointKind has
/// exactly one factory arm.
#[test]
fn death_list_codecs_are_provider_agnostic() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let codec_dir = root.join("src/llm/codec");
    let mut files = Vec::new();
    walk_rs_files(&codec_dir, &mut files);
    assert!(!files.is_empty(), "codec directory must exist");

    let mut hits = Vec::new();
    for path in &files {
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            if is_comment_line(line) {
                continue;
            }
            // Catalog fixtures in tests legitimately mention provider ids; the
            // production shape we forbid is branching on one.
            let branches = line.contains("provider_id ==")
                || line.contains("provider_id.as_str()")
                || line.contains("match provider_id")
                || line.contains("match model.provider_id");
            if branches {
                hits.push(format!("{rel}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "codec must not branch on provider id:\n{}",
        hits.join("\n")
    );

    let factory = fs::read_to_string(root.join("src/llm/codec/mod.rs")).expect("codec/mod.rs");
    for kind in crate::provider_catalog::EndpointKind::ALL {
        let needle = format!("EndpointKind::{} =>", pascal_case(kind.as_str()));
        assert!(
            factory.contains(&needle),
            "codec factory must select a codec for {kind:?} ({needle})"
        );
    }
    // Only the factory match counts: the diagnostic prefix may switch on the
    // kind too, but the codec selector must be a single exhaustive match.
    let build = factory
        .split("pub(crate) fn error_prefix")
        .next()
        .expect("factory section");
    let arms = build.matches("EndpointKind::").count();
    assert_eq!(
        arms,
        crate::provider_catalog::EndpointKind::ALL.len(),
        "one codec factory arm per EndpointKind, no more"
    );
}

fn pascal_case(snake: &str) -> String {
    snake
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// The runtime store touches only the credential table; the legacy tables are
/// readable exactly once, from the migration module.
#[test]
fn death_list_db_runtime_store_only_uses_provider_credentials() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let runtime_store =
        fs::read_to_string(root.join("src/config/global_db/mod.rs")).expect("global_db/mod.rs");
    for needle in [
        "FROM providers",
        "INTO providers",
        "FROM models",
        "INTO models",
        "DELETE FROM providers",
        "DELETE FROM models",
    ] {
        assert!(
            !runtime_store.contains(needle),
            "runtime store must not touch legacy tables: {needle}"
        );
    }
    assert!(
        runtime_store.contains("provider_credentials"),
        "runtime store must own the credential table"
    );

    let schema_sql =
        fs::read_to_string(root.join("src/config/global_db/schema.sql")).expect("schema.sql");
    assert!(
        schema_sql.contains("provider_credentials"),
        "fresh schema must create the credential table"
    );
    assert!(
        !schema_sql.contains("CREATE TABLE IF NOT EXISTS providers")
            && !schema_sql.contains("CREATE TABLE IF NOT EXISTS models"),
        "fresh installs must not create the legacy LLM tables"
    );

    let legacy =
        fs::read_to_string(root.join("src/config/global_db/legacy.rs")).expect("legacy.rs");
    assert!(
        legacy.contains("FROM providers") && legacy.contains("FROM models"),
        "the one-shot migration is the only reader of the legacy tables"
    );
}
