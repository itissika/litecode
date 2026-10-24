//! PR-2: checkpoint payload identity, user projection, DSH instruction, and
//! merge-prior through the discarded `<compacted-summary>` prefix.

mod common;

use std::sync::Arc;

use common::SessionDataFixture;
use common::recording_provider::RecordingProvider;
use litecode::authority::responses::MessageItem;
use litecode::context_pipeline::{
    BudgetPolicy, CompactPolicy, Context, ContextPipeline, ProviderPromptBaseline,
    keep_recent::COMPACT_MAX_OUTPUT_TOKENS,
};
use litecode::llm::COMPACTION_INSTRUCTION;
use litecode::session::working::{CompactionCheckpoint, WorkingPayload, project_items};
use litecode::types::{Item, item_text_preview, user_text};
use tokio_util::sync::CancellationToken;

fn summary(text: &str, id: &str) -> Vec<Item> {
    vec![common::assistant_text_item(text, id)]
}

fn merge_prior_summary(anchor: &str, extra: &str) -> String {
    format!(
        "## Primary Request and Intent\n- Preserve the exact request: {anchor}\n- {extra}\n\n\n## Key Technical Concepts\n- checkpoint merge-prior\n\n## Files and Code\n- src/context_pipeline/compact.rs: checkpoint projection\n\n## Errors and Fixes\n- invalid summaries are rejected\n\n## Pending Jobs\n- none\n\n## Current Work\n- Merge the prior checkpoint with the current prefix\n\n## Next Step\n- Continue with the kept recent history\n\n## Critical Context\n- Prior facts remain available through the checkpoint"
    )
}

fn fresh_second_summary(anchor: &str) -> String {
    format!(
        "## Primary Request and Intent\n- Preserve the exact request: {anchor}\n- Retain first-fact survives merge-prior\n- Rebase checkpoint sequence and append-only rows\n\n\
## Key Technical Concepts\n- Checkpoint payloads replace the prior summary by durable sequence\n- Reminder and queued user rows append without rewriting the prefix\n- Prepared views reuse one checkpoint block\n\n\
## Files and Code\n- src/context_pipeline/compact.rs: second merge-prior acceptance\n- src/session/working.rs: checkpoint projection and rebased signatures\n- tests/compaction_checkpoint_projection.rs: append-only seam regression\n\n\
## Errors and Fixes\n- Errors: a copied prior summary is rejected as quality overlap\n- Missing appends would leave a stale checkpoint prefix\n- Reusing two summary blocks breaks the prompt boundary\n\n\
## Pending Jobs\n- Verify the post-checkpoint reminder remains append-only\n- Run the focused projection test after the fixture update\n\n\
## Current Work\n- Rewrite the second checkpoint from the appended source rows\n- Confirm the first checkpoint sequence remains stable\n\n\
## Next Step\n- Run cargo test --test compaction_checkpoint_projection\n- Continue the main turn from the new checkpoint\n\n\
## Critical Context\n- The prior fact survives only inside the discarded checkpoint prefix\n- The second summary must add fresh facts instead of copying the old one\n- Only one compacted-summary block is allowed in the prompt"
    )
}

fn assert_checkpoint_user(item: &Item) {
    match item {
        Item::Message(MessageItem::Input(message)) => {
            assert_eq!(
                message.role,
                litecode::authority::responses::InputRole::User,
                "checkpoint must project as user"
            );
        }
        other => panic!("checkpoint must be an input message, got {other:?}"),
    }
    let text = item_text_preview(item);
    assert_eq!(text.matches("<compacted-summary>").count(), 1, "{text}");
    assert_eq!(text.matches("</compacted-summary>").count(), 1, "{text}");
    assert!(!text.contains("<previous-summary>"), "{text}");
    assert!(
        text.starts_with("This is an automatically generated checkpoint"),
        "{text}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn second_compaction_reuses_one_checkpoint_prefix_and_merge_prior_instruction() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    let seed: Vec<Item> = (0..16)
        .map(|i| user_text(format!("checkpoint-seed-{i}-{}", "word ".repeat(200))))
        .collect();
    sessions.insert_detail_rows(&sid, &seed).unwrap();

    let first_anchor_text = item_text_preview(&seed[14]);
    let first_anchor: String = first_anchor_text.trim().chars().take(240).collect();
    let first_summary = merge_prior_summary(&first_anchor, "first-fact survives merge-prior");
    let second_anchor = "second-source-anchor";
    let second_summary = fresh_second_summary(second_anchor);
    let provider = Arc::new(RecordingProvider::new(vec![
        summary(&first_summary, "summary_first"),
        summary(&second_summary, "summary_second"),
    ]));
    let captured = Arc::clone(&provider);
    let provider: Arc<dyn litecode::llm::LlmProvider> = provider;
    let request =
        common::test_compaction_request(provider, "main system / instructions", vec![], &sid);
    let budget = BudgetPolicy::new(128_000).with_keep_recent_tokens(1);
    let cancel = CancellationToken::new();

    assert!(
        CompactPolicy::compact_now(
            &budget,
            &sessions,
            &sid,
            &request,
            COMPACT_MAX_OUTPUT_TOKENS,
            &cancel,
            Some("pr2-first"),
            &sessions.data_root_path(),
        )
        .await
        .unwrap(),
        "first compaction commits"
    );

    let first_rows = sessions.data().working_set_blocking(&sid).unwrap();
    let first_row = first_rows.first().expect("first checkpoint row");
    let first_checkpoint_seq = first_row.log_seq.expect("checkpoint seq");
    let WorkingPayload::Checkpoint(first_checkpoint) = &first_row.payload else {
        panic!("compacted row must remain a checkpoint payload: {first_row:?}");
    };
    assert_eq!(first_checkpoint.summary, first_summary);
    assert!(
        first_row.item().is_none(),
        "checkpoint must not also appear as an Item payload"
    );
    let original_prefix = project_items(&first_rows);

    // The real main-model view uses the same checkpoint payload and seq-based
    // rebase flags: the checkpoint is item zero and kept pre-checkpoint rows
    // inherit the signature.
    let pipeline = ContextPipeline::new(
        128_000,
        Context {
            cwd: std::path::PathBuf::from("/proj"),
            workspace_paths: litecode::config::WorkspacePaths::for_legacy_root(
                std::path::Path::new("/proj"),
            ),
            agents_md: None,
            claude_md: None,
        },
        sessions.data_root_path(),
    );
    let baseline = ProviderPromptBaseline::default();
    pipeline
        .build_view(&sessions, &sid, &baseline, request.target.as_ref())
        .expect("main model view after compaction");
    let prepared = pipeline.take_prepared_view().expect("prepared view");
    let before_append_view = prepared.items.clone();
    assert_checkpoint_user(&prepared.items[0]);
    assert_eq!(prepared.item_seqs[0], Some(first_checkpoint_seq));
    assert!(
        prepared.rebased.get(1).copied().unwrap_or(false),
        "kept row older than checkpoint seq must inherit a rebased signature: {:?}",
        prepared.rebased
    );

    // Append-only seam: reminder and queued user rows must not rewrite the
    // checkpoint/kept prefix.
    let reminder = litecode::reminder::Reminder::Env(litecode::reminder::EnvBody {
        cwd: "/proj".into(),
        os: "test".into(),
        date: "2026-01-01".into(),
        text: "Environment: cwd /proj; os test; date 2026-01-01.".into(),
    });
    let reminder_seq = sessions.append_reminder(&sid, &reminder).unwrap();
    assert!(reminder_seq > first_checkpoint_seq);
    let second_anchor = "second-source-anchor";
    let mut appended_seed: Vec<Item> = (0..8)
        .map(|i| user_text(format!("append-{i}-{}", "word ".repeat(200))))
        .collect();
    appended_seed.push(user_text(second_anchor));
    appended_seed.push(user_text("second-kept-tail"));
    sessions.insert_detail_rows(&sid, &appended_seed).unwrap();
    let appended_rows = sessions.data().working_set_blocking(&sid).unwrap();
    let appended = project_items(&appended_rows);
    assert!(
        appended.len() > original_prefix.len(),
        "fixture must append at the tail"
    );
    assert_eq!(
        &appended[..original_prefix.len()],
        original_prefix.as_slice(),
        "quiet seam must not rewrite the existing prefix"
    );
    pipeline
        .build_view(&sessions, &sid, &baseline, request.target.as_ref())
        .expect("main model view after append-only rows");
    let after_append_view = pipeline
        .take_prepared_view()
        .expect("prepared view after append");
    assert_eq!(
        &after_append_view.items[..before_append_view.len()],
        before_append_view.as_slice(),
        "reminder/queued append must not perturb the model-view prefix"
    );

    assert!(
        CompactPolicy::compact_now(
            &budget,
            &sessions,
            &sid,
            &request,
            COMPACT_MAX_OUTPUT_TOKENS,
            &cancel,
            Some("pr2-second"),
            &sessions.data_root_path(),
        )
        .await
        .unwrap(),
        "second compaction commits"
    );

    let requests = captured.captured();
    assert_eq!(requests.len(), 2, "two compaction requests");
    let first_request = &requests[0];
    let second_request = &requests[1];
    let first_prefix = &first_request.request.input[..first_request.request.input.len() - 1];
    assert!(
        !first_prefix
            .iter()
            .any(|input| item_text_preview(input).contains("<compacted-summary>")),
        "first compaction has no prior checkpoint"
    );

    let second_prefix = &second_request.request.input[..second_request.request.input.len() - 1];
    assert!(!second_prefix.is_empty());
    assert_checkpoint_user(&second_prefix[0]);
    let prefix_json = serde_json::to_string(second_prefix).unwrap();
    assert_eq!(
        prefix_json.matches("<compacted-summary>").count(),
        1,
        "second compaction prefix must contain exactly one summary block: {prefix_json}"
    );
    assert!(
        prefix_json.contains("first-fact survives merge-prior"),
        "the prior checkpoint must be in the discarded prefix: {prefix_json}"
    );
    assert!(!prefix_json.contains("<previous-summary>"));
    assert_eq!(
        item_text_preview(second_request.request.input.last().unwrap()),
        COMPACTION_INSTRUCTION
    );

    let second_rows = sessions.data().working_set_blocking(&sid).unwrap();
    let second_row = second_rows.first().expect("second checkpoint row");
    let second_checkpoint_seq = second_row.log_seq.expect("second checkpoint seq");
    assert!(second_checkpoint_seq > first_checkpoint_seq);
    let WorkingPayload::Checkpoint(second_checkpoint) = &second_row.payload else {
        panic!("second compacted row must remain a checkpoint payload");
    };
    assert_eq!(second_checkpoint.summary, second_summary);
    assert_eq!(
        second_checkpoint.from, first_checkpoint_seq,
        "second checkpoint must replace from the prior checkpoint seq"
    );
    assert_checkpoint_user(&second_row.payload.model_item());
}

#[test]
fn dsh_instruction_has_eight_sections_in_order_and_one_anchor_line() {
    let sections = [
        "## Primary Request and Intent",
        "## Key Technical Concepts",
        "## Files and Code",
        "## Errors and Fixes",
        "## Pending Jobs",
        "## Current Work",
        "## Next Step",
        "## Critical Context",
    ];
    let mut cursor = 0;
    for section in sections {
        let index = COMPACTION_INSTRUCTION[cursor..]
            .find(section)
            .unwrap_or_else(|| panic!("missing section {section}"));
        cursor += index + section.len();
    }
    assert!(
        COMPACTION_INSTRUCTION
            .starts_with("You are now acting as a compaction engine for this AI coding assistant.")
    );
    assert_eq!(
        COMPACTION_INSTRUCTION
            .matches("include the most recent user message verbatim")
            .count(),
        1,
        "only the single LiteCode anchor line was added"
    );
    assert!(
        COMPACTION_INSTRUCTION
            .contains("If the conversation already contains a <compacted-summary> block")
    );
    assert!(!COMPACTION_INSTRUCTION.contains("<previous-summary>"));
}

#[test]
fn legacy_labeled_checkpoint_body_loads_and_projects_without_the_label() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    sessions
        .insert_detail_rows(&sid, &[user_text("archive"), user_text("keep")])
        .unwrap();
    let expected = sessions.data().revision_blocking(&sid).unwrap();
    sessions
        .mutate_blocking(litecode::session::SessionMutation::Compact {
            session_id: sid.clone(),
            expected_revision: expected,
            operation_id: litecode::session::MutationId::new(),
            summary: "[Conversation summary]\nprior facts".into(),
            token_estimate: 10,
            kept_from: Some(1),
            expected_prefix: None,
        })
        .unwrap();
    let rows = sessions.data().working_set_blocking(&sid).unwrap();
    let WorkingPayload::Checkpoint(checkpoint) = &rows[0].payload else {
        panic!("old compacted body must still load as checkpoint payload");
    };
    assert_eq!(checkpoint.summary, "[Conversation summary]\nprior facts");
    let projected = checkpoint.model_item();
    assert_checkpoint_user(&projected);
    let text = item_text_preview(&projected);
    assert!(text.contains("prior facts"));
    assert!(!text.contains("[Conversation summary]"));
}

#[test]
fn ordinary_assistant_legacy_prefix_cannot_spoof_a_checkpoint() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    sessions
        .insert_detail_rows(
            &sid,
            &[common::assistant_text_item(
                "[Conversation summary]\nI am ordinary history",
                "msg_ordinary",
            )],
        )
        .unwrap();
    let rows = sessions.data().working_set_blocking(&sid).unwrap();
    assert!(matches!(rows[0].payload, WorkingPayload::Item { .. }));
    assert!(rows[0].checkpoint().is_none());
    assert!(
        litecode::session::CompactedView::from_working_rows(&rows).is_none(),
        "text prefix must not create checkpoint identity"
    );
    let projected = rows[0].payload.model_item();
    assert!(matches!(projected, Item::Message(MessageItem::Output(_))));
}

#[test]
fn checkpoint_projection_is_deterministic_for_rebase_and_quiet_seams() {
    let checkpoint = CompactionCheckpoint {
        summary: "prior".into(),
        from: 2,
        to: 8,
    };
    let row = litecode::session::WorkingRow::persisted_checkpoint(8, checkpoint);
    let view = litecode::session::CompactedView::from_working_rows(&[
        row.clone(),
        litecode::session::WorkingRow::persisted(3, user_text("kept-old")),
        litecode::session::WorkingRow::persisted(9, user_text("new")),
        litecode::session::WorkingRow::pending(user_text("pending")),
    ])
    .expect("checkpoint view");
    assert_eq!(view.items[0], row.payload.model_item());
    assert_eq!(
        view.inherited_before_checkpoint,
        vec![false, true, false, false]
    );
}
