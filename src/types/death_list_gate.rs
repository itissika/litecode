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
    path_slash(path).contains("/llm/adapter/")
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
    let in_adapter = is_adapter_path(path);
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
        panic!("MENTAL-MODEL.md or .scratch spec.md must exist for G1 vocab");
    };
    for needle in ["seq", "surface_op", "source_seqs"] {
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

/// R9 DoD #1: adapter registry is the LLM product surface — no fake seed providers.
#[test]
fn death_list_adapter_registry_invariants() {
    use crate::config::schema::{
        ADAPTER_ARK_CODING, ADAPTER_DEEPSEEK_RESPONSES, ADAPTER_MIMO_RESPONSES,
        ADAPTER_OPENAI_RESPONSES, ADAPTER_OPENCODE,
    };
    use crate::llm::list_adapters;

    let adapters = list_adapters();
    assert_eq!(
        adapters.len(),
        5,
        "expected exactly five registered adapters"
    );
    let ids: Vec<_> = adapters.iter().map(|a| a.id).collect();
    assert!(ids.contains(&ADAPTER_OPENAI_RESPONSES));
    assert!(ids.contains(&ADAPTER_DEEPSEEK_RESPONSES));
    assert!(ids.contains(&ADAPTER_MIMO_RESPONSES));
    assert!(ids.contains(&ADAPTER_OPENCODE));
    assert!(ids.contains(&ADAPTER_ARK_CODING));

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let seed = fs::read_to_string(root.join("src/config/global_db/seed.rs")).expect("seed.rs");
    let schema_sql =
        fs::read_to_string(root.join("src/config/global_db/schema.sql")).expect("schema.sql");
    let schema_rs = fs::read_to_string(root.join("src/config/schema.rs")).expect("schema.rs");

    assert!(
        seed.contains("No fake providers"),
        "seed.rs must not plant fake LLM provider/model rows"
    );
    for line in seed.lines() {
        if is_comment_line(line) {
            continue;
        }
        if line.contains("LlmProtocol") {
            panic!("seed must not reference removed LlmProtocol: {line}");
        }
    }

    assert!(
        !schema_sql.contains("protocol"),
        "schema.sql must not retain providers.protocol column"
    );
    assert!(
        schema_sql.contains("adapter_id"),
        "schema.sql providers must use adapter_id"
    );

    for const_name in [
        "ADAPTER_OPENAI_RESPONSES",
        "ADAPTER_DEEPSEEK_RESPONSES",
        "ADAPTER_MIMO_RESPONSES",
        "ADAPTER_OPENCODE",
        "ADAPTER_ARK_CODING",
    ] {
        assert!(
            schema_rs.contains(const_name),
            "schema.rs must declare adapter id constant {const_name}"
        );
    }
    assert!(
        !schema_rs.contains("pub enum LlmProtocol"),
        "schema.rs must not define removed LlmProtocol enum"
    );
}
