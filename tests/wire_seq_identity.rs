//! Locks seq identity on the wire projection: cursor, compact, revert, and
//! streaming must not invent, skip, or reuse seqs. Asserts outcomes, not
//! whether the cursor once scanned the full log.

use std::sync::Arc;

use litecode::authority::responses::{ResponseStreamEvent, ResponseTextDeltaEvent};
use litecode::client_protocol::controller::Projection;
use litecode::client_protocol::observer::InternalEvent;
use litecode::client_protocol::protocol::{SessionBindingProjection, methods};
use litecode::config::TurnGuard;
use litecode::runtime::observer::{
    CompactionStage, CompactionTrigger, TurnEndReason, TurnTokenStats,
};
use litecode::session::data::command::{MutationId, SessionMutation};
use litecode::session::event::EventType;
use litecode::session::manager::SessionManager;
use litecode::types::user_text;

fn binding() -> SessionBindingProjection {
    SessionBindingProjection::default()
}

fn sessions() -> Arc<SessionManager> {
    Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        String::new(),
    ))
}

fn setup_with_details(texts: &[&str]) -> (Projection, String, Arc<SessionManager>) {
    let sessions = sessions();
    let sid = sessions
        .open_session_sync("/p", "default", Some("m"))
        .unwrap();
    sessions
        .insert_detail_rows(
            &sid,
            &texts.iter().map(|t| user_text(*t)).collect::<Vec<_>>(),
        )
        .unwrap();
    let proj = Projection::new(sid.clone(), sessions.clone(), 0);
    (proj, sid, sessions)
}

fn apply_compact(sessions: &SessionManager, sid: &str, summary: &str) {
    let expected = sessions.data().revision_blocking(sid).unwrap_or(0);
    sessions
        .mutate_blocking(SessionMutation::Compact {
            session_id: sid.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            summary: user_text(summary),
            token_estimate: 10,
            kept_from: None,
            expected_prefix: None,
        })
        .unwrap();
}

fn insert_detail(sessions: &SessionManager, sid: &str, text: &str) {
    sessions
        .insert_detail_rows(sid, &[user_text(text)])
        .unwrap();
}

fn log_max_cursor(sessions: &SessionManager, sid: &str) -> (i64, u64) {
    let events = sessions.data().events_blocking(sid).unwrap();
    let last = events.iter().map(|e| e.seq as i64).max().unwrap_or(-1);
    let next = if last < 0 {
        0
    } else {
        (last as u64).saturating_add(1)
    };
    (last, next)
}

fn assert_cursors_agree(proj: &Projection, sessions: &SessionManager, sid: &str) {
    let from_events = log_max_cursor(sessions, sid);
    let from_entry = sessions.entry_wire_seq_cursor(sid);
    let snap = proj.snapshot("/p", &binding());
    assert_eq!(from_entry, from_events, "entry cursor must equal MAX(seq)");
    assert_eq!(
        snap.buffer.last_seq, from_events.0,
        "snapshot last_seq must equal MAX(seq)"
    );
    assert_eq!(
        snap.buffer.next_seq, from_events.1,
        "snapshot next_seq must equal MAX(seq)+1"
    );
}

fn buffer_item_frames(out: &[serde_json::Value]) -> Vec<&serde_json::Value> {
    out.iter()
        .filter(|msg| msg["method"] == methods::BUFFER_ITEM)
        .collect()
}

fn assert_buffer_items_below_next(out: &[serde_json::Value], next_seq: u64) {
    for item in buffer_item_frames(out) {
        let seq = item["params"]["seq"].as_u64().expect("buffer/item seq");
        assert!(
            seq < next_seq,
            "buffer/item seq {seq} must be < next_seq {next_seq}"
        );
    }
}

fn succeeded(trigger: CompactionTrigger) -> InternalEvent {
    InternalEvent::CompactionLifecycle {
        trigger,
        stage: CompactionStage::Succeeded,
        operation_id: None,
        fail_kind: None,
        error: None,
    }
}

fn stream_delta(n: u64) -> InternalEvent {
    InternalEvent::StreamEvent(ResponseStreamEvent::ResponseOutputTextDelta(
        ResponseTextDeltaEvent {
            sequence_number: n,
            item_id: "msg_1".into(),
            output_index: 0,
            content_index: 0,
            delta: "x".into(),
            logprobs: None,
        },
    ))
}

#[test]
fn empty_session_cursor_is_minus_one_zero() {
    let sessions = sessions();
    let sid = sessions
        .open_session_sync("/p", "default", Some("m"))
        .unwrap();
    let proj = Projection::new(sid.clone(), sessions.clone(), 0);
    assert_eq!(sessions.entry_wire_seq_cursor(&sid), (-1, 0));
    assert_cursors_agree(&proj, &sessions, &sid);
}

#[test]
fn three_detail_rows_cursor_is_two_three() {
    let (proj, sid, sessions) = setup_with_details(&["a", "b", "c"]);
    assert_eq!(sessions.entry_wire_seq_cursor(&sid), (2, 3));
    assert_cursors_agree(&proj, &sessions, &sid);
}

#[test]
fn compact_succeeded_checkpoint_item_precedes_lifecycle_and_matches_max_seq() {
    let (mut proj, sid, sessions) = setup_with_details(&["a", "b", "c"]);
    apply_compact(&sessions, &sid, "first-cut");
    let _ = proj.take_outgoing();

    proj.on_event(succeeded(CompactionTrigger::Manual), "/p", &binding());
    let out = proj.take_outgoing();

    let items = buffer_item_frames(&out);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["params"]["seq"], 3);
    assert_eq!(items[0]["params"]["kind"], "compacted");

    let life = out
        .iter()
        .find(|msg| msg["method"] == methods::SESSION_COMPACT_LIFECYCLE)
        .expect("lifecycle after checkpoint item");
    assert_eq!(life["params"]["snapshot"]["buffer"]["last_seq"], 3);
    assert_eq!(life["params"]["snapshot"]["buffer"]["next_seq"], 4);

    let item_pos = out
        .iter()
        .position(|msg| msg["method"] == methods::BUFFER_ITEM)
        .unwrap();
    let life_pos = out
        .iter()
        .position(|msg| msg["method"] == methods::SESSION_COMPACT_LIFECYCLE)
        .unwrap();
    assert!(item_pos < life_pos);

    let events = sessions.data().events_blocking(&sid).unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.seq == 0 && e.event_type != EventType::Compacted),
        "compact must keep historical rows"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.event_type == EventType::Compacted)
            .count(),
        1
    );
    assert_cursors_agree(&proj, &sessions, &sid);
    assert_buffer_items_below_next(&out, 4);
}

#[test]
fn second_compact_appends_new_seq_and_keeps_both_checkpoints() {
    let (mut proj, sid, sessions) = setup_with_details(&["a", "b", "c"]);
    apply_compact(&sessions, &sid, "first-cut");
    proj.on_event(succeeded(CompactionTrigger::Auto), "/p", &binding());
    let _ = proj.take_outgoing();

    insert_detail(&sessions, &sid, "d");
    proj.bump_buffer_revision("/p", &binding());
    let _ = proj.take_outgoing();

    apply_compact(&sessions, &sid, "second-cut");
    proj.on_event(succeeded(CompactionTrigger::Auto), "/p", &binding());
    let out = proj.take_outgoing();

    let items = buffer_item_frames(&out);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["params"]["seq"], 5);
    assert_eq!(items[0]["params"]["kind"], "compacted");

    let events = sessions.data().events_blocking(&sid).unwrap();
    let compact_seqs: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == EventType::Compacted)
        .map(|e| e.seq)
        .collect();
    assert_eq!(compact_seqs, vec![3, 5]);
    assert_cursors_agree(&proj, &sessions, &sid);
}

#[test]
fn revert_k1_emits_reverted_and_operation_snapshot_matching_max_seq() {
    let (mut proj, sid, sessions) = setup_with_details(&["a", "b", "c"]);
    proj.revert_to_user_anchor(1, "/p", &binding()).unwrap();
    let out = proj.take_outgoing();

    let reverted = out
        .iter()
        .find(|msg| msg["method"] == "buffer/reverted")
        .expect("buffer/reverted");
    assert_eq!(reverted["params"]["next_seq"], 1);
    assert_eq!(reverted["params"]["last_seq"], 0);

    let op = out
        .iter()
        .find(|msg| msg["method"] == "agent/operation_result")
        .expect("operation_result");
    assert_eq!(op["params"]["ok"], true);
    assert_eq!(op["params"]["snapshot"]["buffer"]["last_seq"], 0);
    assert_eq!(op["params"]["snapshot"]["buffer"]["next_seq"], 1);

    let loaded = proj.materialize_range(0, 1).unwrap();
    assert!(
        loaded.events.iter().all(|e| e.seq < 1),
        "buffer/load must not return seq >= next_seq after revert"
    );
    assert_eq!(loaded.events.len(), 1);
    assert_eq!(loaded.events[0].seq, 0);

    assert_cursors_agree(&proj, &sessions, &sid);
    assert!(
        buffer_item_frames(&out).is_empty(),
        "revert shrinks the log and must not emit new buffer/item rows"
    );
}

#[test]
fn compact_then_revert_before_checkpoint_drops_compact_and_resets_pointers() {
    let (mut proj, sid, sessions) = setup_with_details(&["a", "b", "c"]);
    apply_compact(&sessions, &sid, "first-cut");
    proj.on_event(succeeded(CompactionTrigger::Auto), "/p", &binding());
    let _ = proj.take_outgoing();
    assert_cursors_agree(&proj, &sessions, &sid);

    proj.revert_to_user_anchor(1, "/p", &binding()).unwrap();
    let out = proj.take_outgoing();
    let reverted = out
        .iter()
        .find(|msg| msg["method"] == "buffer/reverted")
        .expect("buffer/reverted");
    assert_eq!(reverted["params"]["next_seq"], 1);

    let events = sessions.data().events_blocking(&sid).unwrap();
    assert!(
        events.iter().all(|e| e.event_type != EventType::Compacted),
        "revert before checkpoint must drop the compacted row"
    );
    let meta = sessions.data().meta_blocking(&sid).unwrap();
    assert_eq!(meta.compacted_seq, None);
    assert_eq!(meta.spine_from, 0);
    assert_cursors_agree(&proj, &sessions, &sid);
}

#[test]
fn compact_then_revert_after_checkpoint_keeps_compacted_seq() {
    let (mut proj, sid, sessions) = setup_with_details(&["a", "b", "c"]);
    apply_compact(&sessions, &sid, "first-cut");
    proj.on_event(succeeded(CompactionTrigger::Auto), "/p", &binding());
    let _ = proj.take_outgoing();

    insert_detail(&sessions, &sid, "d");
    proj.bump_buffer_revision("/p", &binding());
    let _ = proj.take_outgoing();
    assert_eq!(sessions.entry_wire_seq_cursor(&sid), (4, 5));

    // Users a,b,c plus post-compact d → k=3 is the new user (seq 4).
    proj.revert_to_user_anchor(3, "/p", &binding()).unwrap();
    let out = proj.take_outgoing();
    let reverted = out
        .iter()
        .find(|msg| msg["method"] == "buffer/reverted")
        .expect("buffer/reverted");
    assert_eq!(reverted["params"]["next_seq"], 4);
    assert_eq!(reverted["params"]["last_seq"], 3);

    let events = sessions.data().events_blocking(&sid).unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.event_type == EventType::Compacted && e.seq == 3),
        "revert after checkpoint must keep the compacted row"
    );
    let meta = sessions.data().meta_blocking(&sid).unwrap();
    assert_eq!(meta.compacted_seq, Some(3));
    assert_eq!(meta.spine_from, 3);
    assert_cursors_agree(&proj, &sessions, &sid);
}

#[test]
fn revert_then_append_uses_truncated_max_plus_one() {
    let (mut proj, sid, sessions) = setup_with_details(&["a", "b", "c"]);
    proj.revert_to_user_anchor(1, "/p", &binding()).unwrap();
    let _ = proj.take_outgoing();
    assert_eq!(sessions.entry_wire_seq_cursor(&sid), (0, 1));

    insert_detail(&sessions, &sid, "after-revert");
    proj.bump_buffer_revision("/p", &binding());
    let out = proj.take_outgoing();
    let items = buffer_item_frames(&out);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["params"]["seq"], 1);
    assert_cursors_agree(&proj, &sessions, &sid);
    assert_buffer_items_below_next(&out, 2);
}

#[test]
fn stream_events_do_not_emit_snapshot_or_grow_seq() {
    let (mut proj, sid, sessions) = setup_with_details(&["a"]);
    let before = sessions.entry_wire_seq_cursor(&sid);
    proj.on_event(
        InternalEvent::TurnStarted {
            turn_id: "t1".into(),
            input: "hi".into(),
            step_max: 4,
        },
        "/p",
        &binding(),
    );
    let _ = proj.take_outgoing();

    for n in 1..=8 {
        proj.on_event(stream_delta(n), "/p", &binding());
    }
    let out = proj.take_outgoing();
    assert!(
        out.iter()
            .all(|msg| msg["method"] != methods::SESSION_SNAPSHOT),
        "stream path must not emit session/snapshot"
    );
    assert!(
        buffer_item_frames(&out).is_empty(),
        "stream path must not emit buffer/item"
    );
    assert_eq!(sessions.entry_wire_seq_cursor(&sid), before);
    assert_cursors_agree(&proj, &sessions, &sid);
}

#[test]
fn step_committed_emits_only_new_seq_and_snapshot_matches_max() {
    let (mut proj, sid, sessions) = setup_with_details(&["a", "b"]);
    assert_eq!(sessions.entry_wire_seq_cursor(&sid), (1, 2));

    insert_detail(&sessions, &sid, "c");
    proj.on_event(InternalEvent::StepCommitted, "/p", &binding());
    let out = proj.take_outgoing();
    let items = buffer_item_frames(&out);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["params"]["seq"], 2);
    assert!(
        out.iter()
            .all(|msg| msg["method"] != methods::SESSION_SNAPSHOT),
        "StepCommitted must not emit session/snapshot"
    );
    assert_cursors_agree(&proj, &sessions, &sid);
    assert_buffer_items_below_next(&out, 3);
}

#[test]
fn turn_finished_snapshot_next_seq_matches_db_max_plus_one() {
    let (mut proj, sid, sessions) = setup_with_details(&["a", "b", "c"]);
    proj.on_event(
        InternalEvent::TurnStarted {
            turn_id: "t1".into(),
            input: "hi".into(),
            step_max: 4,
        },
        "/p",
        &binding(),
    );
    let _ = proj.take_outgoing();

    proj.on_event(
        InternalEvent::TurnCompleted {
            turn_id: "t1".into(),
            final_text: Some("done".into()),
            reason: TurnEndReason::Completed,
            turn_token_stats: TurnTokenStats::default(),
            committed_next_seq: 99,
        },
        "/p",
        &binding(),
    );
    let out = proj.take_outgoing();
    let finished = out
        .iter()
        .find(|msg| msg["method"] == "agent/turn_finished")
        .expect("turn_finished");
    let (last, next) = log_max_cursor(&sessions, &sid);
    assert_eq!(finished["params"]["snapshot"]["buffer"]["last_seq"], last);
    assert_eq!(finished["params"]["snapshot"]["buffer"]["next_seq"], next);
    assert_eq!(next, 3);
}
