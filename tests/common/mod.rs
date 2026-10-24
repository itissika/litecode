pub mod bindings;
pub mod fake_deps;
pub mod permission;
pub mod recording_provider;
pub mod responses_replay;
pub mod runtime;
pub mod scripted_provider;
pub mod seed;
pub mod session_data_fixture;
pub mod subagent_fixture;
pub mod workspace_fixture;

pub fn completed_terminal_event() -> litecode::types::StreamEvents {
    serde_json::from_value(serde_json::json!({
        "type": "response.completed",
        "sequence_number": 1,
        "response": {
            "id": "resp_test_completed",
            "object": "response",
            "created_at": 1,
            "model": "test-model",
            "status": "completed",
            "output": [],
            "parallel_tool_calls": null,
            "tool_choice": null,
            "tools": null
        }
    }))
    .expect("completed terminal event")
}

pub fn emit_completed_terminal(
    on_event: &mut Option<Box<dyn FnMut(litecode::types::StreamEvents) + Send + '_>>,
) {
    if let Some(callback) = on_event.as_mut() {
        callback(completed_terminal_event());
    }
}

pub fn dsh_summary(anchor: &str) -> String {
    format!(
        "## Primary Request and Intent\n- Preserve the user request verbatim: {anchor}\n\n\n## Key Technical Concepts\n- Rust provider streams and compaction checkpoints\n\n## Files and Code\n- src/context_pipeline/compact.rs: mechanical summary acceptance\n\n## Errors and Fixes\n- incomplete provider finish is rejected\n\n## Pending Jobs\n- none\n\n## Current Work\n- Validate the summary before persistence\n\n## Next Step\n- Continue the main turn after the checkpoint\n\n## Critical Context\n- Keep the main request prefix stable"
    )
}

pub fn latest_user_anchor_from_items(items: &[litecode::types::Item]) -> String {
    for item in items.iter().rev() {
        let litecode::types::Item::Message(litecode::authority::responses::MessageItem::Input(
            message,
        )) = item
        else {
            continue;
        };
        if !matches!(
            message.role,
            litecode::authority::responses::InputRole::User
        ) {
            continue;
        }
        let text = message
            .content
            .iter()
            .filter_map(|part| match part {
                litecode::authority::responses::InputContent::InputText(text) => {
                    Some(text.text.as_str())
                }
                _ => None,
            })
            .collect::<String>();
        let text = text.trim();
        if text.is_empty()
            || text.contains("<compacted-summary>")
            || text.starts_with(litecode::config::global_db::CHECKPOINT_PREAMBLE)
        {
            continue;
        }
        return text.chars().take(240).collect();
    }
    "recent user request".into()
}

fn summary_anchor_token(raw: &str) -> &str {
    raw.trim_matches(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\''
                    | '`'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | ','
                    | ';'
                    | ':'
                    | '|'
                    | '<'
                    | '>'
            )
    })
}

fn summary_source_tokens(source_text: &str) -> Vec<&str> {
    source_text
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .flat_map(str::split_whitespace)
        .map(summary_anchor_token)
        .filter(|token| token.len() >= 3)
        .collect()
}

fn first_source_path(source_text: &str) -> Option<&str> {
    summary_source_tokens(source_text)
        .into_iter()
        .find(|token| {
            let lower = token.to_ascii_lowercase();
            (token.contains('/') || token.contains('\\'))
                && (token.contains('.')
                    || lower.starts_with("src")
                    || lower.starts_with("tests")
                    || lower.starts_with("web"))
        })
}

fn first_source_command(source_text: &str) -> Option<&str> {
    summary_source_tokens(source_text)
        .into_iter()
        .find(|token| {
            let lower = token.to_ascii_lowercase();
            [
                "cargo",
                "pnpm",
                "npm",
                "git",
                "rg",
                "python",
                "node",
                "pwsh",
                "powershell",
                "bash",
                "dotnet",
                "bun",
                "yarn",
            ]
            .iter()
            .any(|command| lower == *command || lower.starts_with(&format!("{command}:")))
        })
}

fn first_source_error(source_text: &str) -> Option<&str> {
    summary_source_tokens(source_text)
        .into_iter()
        .find(|token| {
            let lower = token.to_ascii_lowercase();
            lower.contains("error")
                || lower.contains("panic")
                || lower.contains("exception")
                || lower.contains("traceback")
                || lower.contains("segfault")
                || lower.contains("fatal")
                || lower.starts_with("failed")
                || lower.starts_with("failure")
                || lower.starts_with("e0")
                || [
                    "enoent",
                    "eacces",
                    "eperm",
                    "econnrefused",
                    "etimedout",
                    "enospc",
                    "eexist",
                    "errno",
                ]
                .iter()
                .any(|marker| lower.contains(marker))
        })
}

fn dsh_summary_for_source(anchor: &str, source_text: &str) -> String {
    let mut summary = dsh_summary(anchor);
    if let Some(path) = first_source_path(source_text) {
        summary = summary.replace(
            "- src/context_pipeline/compact.rs: mechanical summary acceptance",
            &format!("- {path}: mechanical summary acceptance"),
        );
    }
    if let Some(error) = first_source_error(source_text) {
        summary = summary.replace(
            "- incomplete provider finish is rejected",
            &format!("- {error}: observed provider failure"),
        );
    }
    if let Some(command) = first_source_command(source_text) {
        summary = summary.replace(
            "- Validate the summary before persistence",
            &format!("- Run {command} and validate the summary"),
        );
    }
    summary
}

pub fn valid_summary_for_request(request: &litecode::llm::ModelRequest) -> String {
    let source = &request.input[..request.input.len().saturating_sub(1)];
    let anchor = latest_user_anchor_from_items(source);
    let source_text = source
        .iter()
        .map(litecode::types::item_text_preview)
        .collect::<Vec<_>>()
        .join("\n");
    dsh_summary_for_source(&anchor, &source_text)
}

/// LEGACY REQUEST-SHAPE FIXTURE ONLY.
///
/// When the request is a compaction call and the queued response is one
/// ordinary completed assistant message, replace its text with a known-good
/// DSH-shaped summary. This exists solely so older request-prefix/projection
/// fixtures do not repeat summary boilerplate. Never use it in PR-5
/// acceptance, finish, quality, retry, or lifecycle assertions: those tests
/// must supply the exact summary item (and terminal event) they intend to
/// exercise.
pub fn legacy_accepting_compaction_items(
    request: &litecode::llm::ModelRequest,
    items: Vec<litecode::types::Item>,
) -> Vec<litecode::types::Item> {
    if request.purpose != litecode::llm::RequestPurpose::Compaction || items.len() != 1 {
        return items;
    }
    let litecode::types::Item::Message(litecode::authority::responses::MessageItem::Output(
        message,
    )) = &items[0]
    else {
        return items;
    };
    if message.status != litecode::authority::responses::OutputStatus::Completed {
        return items;
    }
    let original = message
        .content
        .iter()
        .filter_map(|part| match part {
            litecode::authority::responses::OutputMessageContent::OutputText(text) => {
                Some(text.text.as_str())
            }
            litecode::authority::responses::OutputMessageContent::Refusal(_) => None,
        })
        .collect::<String>();
    if original.trim().is_empty() {
        return items;
    }
    vec![litecode::types::assistant_text(valid_summary_for_request(
        request,
    ))]
}

pub use session_data_fixture::SessionDataFixture;

pub use fake_deps::{FakeAgentDeps, assistant_text_item, function_call_item};
pub use permission::{recording_sink, test_auto_approve_sink};
pub use recording_provider::{CapturedLlmRequest, RecordingProvider, capture_request};
pub use responses_replay::{
    fixture_responses_sse, serve_responses_queue, text_only_completed_sse, tool_call_completed_sse,
};
pub use runtime::{
    TestAgentSpec, build_runtime_with_provider, build_runtime_with_provider_and_observer,
    test_agent, test_compaction_request, test_resolved, test_resolved_with_budget,
    test_sessions_manager, test_turn_binding,
};
pub use scripted_provider::{DshSummaryProvider, ScriptedProvider};
pub use seed::{
    TEST_CATALOG_ENDPOINT, TEST_COMPACTION_MODEL_REF, TEST_PRIMARY_MODEL_REF, TEST_PROVIDER_ID,
    TestGlobalDb, TestServeFixture, build_global_with_custom_tool, build_global_with_mcp_server,
    catalog_for_db, default_test_catalog, default_test_global, fresh_test_global_db,
    insert_test_llm_registry, seed_global_db, seed_test_catalog, seed_test_catalog_with,
    test_catalog, test_catalog_toml, test_catalog_toml_windows, test_catalog_windows,
    test_serve_settings, test_serve_settings_with_db,
};
pub use workspace_fixture::{test_db_path, test_workspace};
