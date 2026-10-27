//! Prepared-compaction commit boundaries: a background summary re-anchors on
//! the current surface and commits the frozen plan; there is no second 0.8
//! gate at commit time.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use common::SessionDataFixture;
use litecode::context_pipeline::{
    BudgetPolicy, COMPACT_MAX_OUTPUT_TOKENS, CompactPolicy, ProviderPromptBaseline,
};
use litecode::runtime::observer::CompactionTrigger;
use litecode::session::working::WorkingPayload;
use litecode::types::{Item, item_text_preview, user_text};
use tokio_util::sync::CancellationToken;

#[tokio::test(flavor = "current_thread")]
async fn prepared_commit_reanchors_on_rows_written_after_prepare() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    let seed: Vec<Item> = (0..16)
        .map(|i| user_text(format!("prepared-seed-{i} {}", "word ".repeat(200))))
        .collect();
    sessions.insert_detail_rows(&sid, &seed).unwrap();

    let provider = Arc::new(common::DshSummaryProvider::new());
    let request = common::test_compaction_request(
        provider.clone(),
        "main system / instructions",
        vec![],
        &sid,
    );
    let budget = BudgetPolicy::new(128_000).with_keep_recent_tokens(1);
    let cancel = CancellationToken::new();

    let rows = sessions.data().working_set_blocking(&sid).unwrap();
    let prepared = CompactPolicy::prepare_compaction(
        &budget,
        &sessions,
        &sid,
        &request,
        COMPACT_MAX_OUTPUT_TOKENS,
        &rows,
        &cancel,
        &sessions.data_root_path(),
    )
    .await
    .expect("prepare succeeds")
    .expect("a cut exists");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);

    // A row written after the background snapshot must survive verbatim. The
    // prepared plan may only re-anchor its cut, never overwrite this tail.
    sessions
        .insert_detail_rows(&sid, &[user_text("post-prepare-row-kept-verbatim")])
        .unwrap();
    let mut current = sessions.data().working_set_blocking(&sid).unwrap();
    let before_len = current.len();
    let baseline = ProviderPromptBaseline::default();
    let outcome = CompactPolicy::compact_transcript(
        &budget,
        &sessions,
        &sid,
        &request,
        COMPACT_MAX_OUTPUT_TOKENS,
        &baseline,
        &mut current,
        &cancel,
        CompactionTrigger::Auto,
        Some("prepared-reanchor"),
        &sessions.data_root_path(),
        Some(prepared),
        1,
    )
    .await
    .expect("prepared commit succeeds");
    assert!(outcome.is_some(), "a checkpoint must be written");
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        1,
        "commit must not summarize again"
    );

    let after = sessions.data().working_set_blocking(&sid).unwrap();
    assert!(
        after
            .iter()
            .any(|row| matches!(row.payload, WorkingPayload::Checkpoint(_))),
        "checkpoint row expected: {after:?}"
    );
    assert!(after.len() < before_len, "the prefix must be replaced");
    let post_row = after
        .iter()
        .find(|row| {
            row.item().is_some_and(|item| {
                item_text_preview(item).contains("post-prepare-row-kept-verbatim")
            })
        })
        .expect("post-prepare row survives the commit");
    assert!(matches!(post_row.payload, WorkingPayload::Item { .. }));
}

#[tokio::test(flavor = "current_thread")]
async fn prepared_commit_keeps_frozen_plan_above_compact_at() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    let seed: Vec<Item> = (0..24)
        .map(|i| user_text(format!("gate-seed-{i} {}", "word ".repeat(6_000))))
        .collect();
    sessions.insert_detail_rows(&sid, &seed).unwrap();

    let provider = Arc::new(common::DshSummaryProvider::new());
    let mut request = common::test_compaction_request(
        provider.clone(),
        "main system / instructions",
        vec![],
        &sid,
    );
    common::set_compaction_request_capacity(&mut request, 32_000, 4096);
    // Final occupancy is above the 25.6k prepare line but still below the 28k
    // prompt ceiling. The old synchronous path commits here; the frozen plan
    // must do the same.
    let budget = BudgetPolicy::new(32_000).with_keep_recent_tokens(21_000);
    let cancel = CancellationToken::new();

    let rows = sessions.data().working_set_blocking(&sid).unwrap();
    let prepared = CompactPolicy::prepare_compaction(
        &budget,
        &sessions,
        &sid,
        &request,
        COMPACT_MAX_OUTPUT_TOKENS,
        &rows,
        &cancel,
        &sessions.data_root_path(),
    )
    .await
    .expect("prepare succeeds")
    .expect("a cut exists");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);

    let mut current = rows.clone();
    let baseline = ProviderPromptBaseline::default();
    let outcome = CompactPolicy::compact_transcript(
        &budget,
        &sessions,
        &sid,
        &request,
        COMPACT_MAX_OUTPUT_TOKENS,
        &baseline,
        &mut current,
        &cancel,
        CompactionTrigger::Auto,
        Some("prepared-gate"),
        &sessions.data_root_path(),
        Some(prepared),
        1,
    )
    .await
    .expect("frozen commit returns without error");
    assert!(
        outcome.is_some(),
        "a prepared commit above compact_at but below the prompt limit must commit"
    );
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        1,
        "no second summary"
    );
    let after = sessions.data().working_set_blocking(&sid).unwrap();
    assert!(
        after
            .iter()
            .any(|row| matches!(row.payload, WorkingPayload::Checkpoint(_))),
        "the frozen prepared plan must write its checkpoint"
    );
}
