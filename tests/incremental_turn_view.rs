//! A running turn extends its view with the rows the log gained, and falls back
//! to the full read whenever the suffix is not a plain append.

mod common;

use common::session_data_fixture::SessionDataFixture;
use litecode::context_pipeline::{Context, ContextPipeline, ProviderPromptBaseline};
use litecode::session::data::command::SessionMutation;
use litecode::types::{assistant_text, user_text};

fn test_context(dir: &std::path::Path) -> Context {
    Context {
        cwd: dir.to_path_buf(),
        workspace_paths: litecode::config::WorkspacePaths::for_legacy_root(dir),
        agents_md: None,
        claude_md: None,
    }
}

fn model() -> std::sync::Arc<litecode::provider_catalog::ResolvedModel> {
    common::default_test_catalog()
        .model(common::TEST_PRIMARY_MODEL_REF)
        .expect("fixture catalog primary model")
        .clone()
}

fn pipeline(fx: &SessionDataFixture) -> ContextPipeline {
    let root = fx.dir.path().to_path_buf();
    ContextPipeline::new(10_000, test_context(&root), root)
}

#[test]
fn a_turn_view_extends_with_only_the_new_rows() {
    let fx = SessionDataFixture::new();
    let sessions = fx.manager();
    let sid = fx.create("/p", "agent", None);
    let first: Vec<_> = (0..3).map(|i| user_text(format!("row {i}"))).collect();
    fx.insert_items(&sid, &first);

    let pipeline = pipeline(&fx);
    let rows = pipeline.begin_turn(&sessions, &sid).expect("begin turn");
    assert_eq!(rows.len(), 3);

    let more: Vec<_> = (3..6).map(|i| user_text(format!("row {i}"))).collect();
    fx.insert_items(&sid, &more);

    let baseline = ProviderPromptBaseline::default();
    pipeline
        .build_view(&sessions, &sid, &baseline, &model())
        .expect("build view");

    assert!(
        pipeline.last_sync_incremental(),
        "a plain append after the turn started must extend the held fold"
    );
    let synced = pipeline.working_set();
    assert_eq!(synced, fx.data.working_set_blocking(&sid).expect("durable"));
    assert_eq!(synced.len(), 6);

    // A long turn keeps extending on every step, not just the first one: the
    // per-step path must never quietly degrade into the full read.
    for round in 6..40 {
        fx.insert_items(&sid, &[user_text(format!("row {round}"))]);
        pipeline
            .build_view(&sessions, &sid, &baseline, &model())
            .expect("build view");
        assert!(
            pipeline.last_sync_incremental(),
            "step {round} fell back to the full read"
        );
    }
    let synced = pipeline.working_set();
    assert_eq!(synced.len(), 40);
    assert_eq!(synced, fx.data.working_set_blocking(&sid).expect("durable"));
}

#[test]
fn a_seal_inside_the_turn_is_refreshed_in_place() {
    let fx = SessionDataFixture::new();
    let sessions = fx.manager();
    let sid = fx.create("/p", "agent", None);
    fx.insert_items(&sid, &[user_text("first")]);

    let pipeline = pipeline(&fx);
    pipeline.begin_turn(&sessions, &sid).expect("begin turn");

    // A stream row opens in flight.
    let revision = fx.data.revision_blocking(&sid).expect("revision");
    fx.mutate(SessionMutation::BeginStreamItem {
        session_id: sid.clone(),
        expected_revision: revision,
        operation_id: SessionDataFixture::operation_id("begin"),
        item: assistant_text("draft"),
        turn_id: "turn".into(),
    });
    let (stream_seq, _) = fx.data.seq_cursor_blocking(&sid).expect("cursor");

    // The cursor moves for another reason, then the row is sealed in place.
    fx.insert_items(&sid, &[user_text("second")]);
    let revision = fx.data.revision_blocking(&sid).expect("revision");
    fx.mutate(SessionMutation::SealStreamItem {
        session_id: sid.clone(),
        expected_revision: revision,
        operation_id: SessionDataFixture::operation_id("seal"),
        seq: stream_seq as u64,
        item: assistant_text("final answer"),
    });

    let baseline = ProviderPromptBaseline::default();
    pipeline
        .build_view(&sessions, &sid, &baseline, &model())
        .expect("build view");

    assert!(
        pipeline.last_sync_incremental(),
        "the suffix is a plain append"
    );
    let synced = pipeline.working_set();
    assert_eq!(synced, fx.data.working_set_blocking(&sid).expect("durable"));
    let sealed = synced
        .iter()
        .find(|row| row.log_seq == Some(stream_seq as u64))
        .expect("sealed row is on the surface");
    assert!(
        format!("{:?}", sealed.payload).contains("final answer"),
        "a seal must be folded in place, not left as the draft"
    );
}

#[test]
fn external_rows_disable_the_incremental_path() {
    let fx = SessionDataFixture::new();
    let sessions = fx.manager();
    let sid = fx.create("/p", "agent", None);
    fx.insert_items(&sid, &[user_text("row 0"), user_text("row 1")]);

    let pipeline = pipeline(&fx);
    let rows = pipeline.begin_turn(&sessions, &sid).expect("begin turn");
    // Rows installed from outside the fold cannot be extended incrementally.
    pipeline.stage_working(rows);

    fx.insert_items(&sid, &[user_text("row 2")]);
    let baseline = ProviderPromptBaseline::default();
    pipeline
        .build_view(&sessions, &sid, &baseline, &model())
        .expect("build view");

    assert!(
        !pipeline.last_sync_incremental(),
        "no held fold means the full read"
    );
    assert_eq!(
        pipeline.working_set(),
        fx.data.working_set_blocking(&sid).expect("durable")
    );
}
#[test]
fn the_projection_refreshes_its_meter_from_the_extended_view() {
    use std::sync::Arc;

    use litecode::client_protocol::controller::Projection;
    use litecode::client_protocol::protocol::SessionBindingProjection;

    let fx = SessionDataFixture::new();
    let sessions = fx.manager();
    let sid = fx.create("/p", "agent", None);
    fx.insert_items(&sid, &[user_text("row 0"), user_text("row 1")]);

    let mut projection =
        Projection::new_with_model(sid.clone(), Arc::new(sessions), 10_000, Some(model()));
    let binding = SessionBindingProjection {
        agent_id: "default".into(),
        model_id: Some(common::TEST_PRIMARY_MODEL_REF.into()),
        api_model_id: "test-primary-model".into(),
        label: "Test".into(),
        context_window: 10_000,
        max_output_tokens: 1_024,
        thinking_tier: "medium".into(),
        context_mode: "standard".into(),
    };

    projection.bump_buffer_revision("/p", &binding);
    assert!(
        !projection.last_estimate_incremental(),
        "the first refresh has no fold yet and must build one"
    );
    let first = projection.context_tokens_estimate;
    assert!(first > 0);

    fx.insert_items(&sid, &[user_text("row 2")]);
    projection.bump_buffer_revision("/p", &binding);
    assert!(
        projection.last_estimate_incremental(),
        "once a fold exists the refresh must extend it"
    );
    assert!(
        projection.context_tokens_estimate > first,
        "the new row has to land in the meter"
    );
}
#[test]
fn a_revert_under_the_fold_forces_a_rebuild() {
    let fx = SessionDataFixture::new();
    let sessions = fx.manager();
    let sid = fx.create("/p", "agent", None);
    let rows: Vec<_> = (0..4).map(|i| user_text(format!("row {i}"))).collect();
    fx.insert_items(&sid, &rows);

    let pipeline = pipeline(&fx);
    assert_eq!(
        pipeline
            .begin_turn(&sessions, &sid)
            .expect("begin turn")
            .len(),
        4
    );

    // Delete the tail, then append before anything observes the shrink. Seqs are
    // never reused, so the new row lands above the old high-water and the seq
    // range alone looks like a plain append.
    sessions
        .entry_revert_to_user_anchor(&sid, 2)
        .expect("revert");
    fx.insert_items(&sid, &[user_text("after revert")]);

    let baseline = ProviderPromptBaseline::default();
    pipeline
        .build_view(&sessions, &sid, &baseline, &model())
        .expect("build view");

    assert!(
        !pipeline.last_sync_incremental(),
        "a revert below the fold must force a rebuild"
    );
    let synced = pipeline.working_set();
    assert_eq!(
        synced,
        fx.data.working_set_blocking(&sid).expect("durable"),
        "the reverted rows must not survive in the prompt"
    );
    assert_eq!(synced.len(), 3);
}

#[test]
fn a_revert_drops_the_projection_fold() {
    use std::sync::Arc;

    use litecode::client_protocol::controller::Projection;
    use litecode::client_protocol::protocol::SessionBindingProjection;

    let fx = SessionDataFixture::new();
    let sessions = Arc::new(fx.manager());
    let sid = fx.create("/p", "agent", None);
    fx.insert_items(
        &sid,
        &[user_text("row 0"), user_text("row 1"), user_text("row 2")],
    );

    let mut projection =
        Projection::new_with_model(sid.clone(), Arc::clone(&sessions), 10_000, Some(model()));
    let binding = SessionBindingProjection {
        agent_id: "default".into(),
        model_id: Some(common::TEST_PRIMARY_MODEL_REF.into()),
        api_model_id: "test-primary-model".into(),
        label: "Test".into(),
        context_window: 10_000,
        max_output_tokens: 1_024,
        thinking_tier: "medium".into(),
        context_mode: "standard".into(),
    };
    projection.bump_buffer_revision("/p", &binding);
    let full = projection.context_tokens_estimate;
    assert!(full > 0);

    sessions
        .entry_revert_to_user_anchor(&sid, 2)
        .expect("revert");
    fx.insert_items(&sid, &[user_text("after revert")]);

    projection.bump_buffer_revision("/p", &binding);
    assert!(
        !projection.last_estimate_incremental(),
        "a revert below the fold must not be extended over"
    );
    assert!(
        projection.context_tokens_estimate < full,
        "the meter must drop once the reverted rows are gone"
    );
}
