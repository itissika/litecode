//! Locks the turn/writer working window against reader full-log fold.
//! Asserts payload/seq identity, not just rendered text.

use std::sync::Arc;

use litecode::config::TurnGuard;
use litecode::context_pipeline::{Context, ContextPipeline};
use litecode::session::data::command::{CommitKind, MutationId, SessionMutation};
use litecode::session::event::spine_working_payload;
use litecode::session::manager::SessionManager;
use litecode::session::surface::project_working_payloads;
use litecode::session::{WorkingPayload, WorkingRow, fold_surface};
use litecode::types::{item_text_preview, user_text};

fn sessions() -> Arc<SessionManager> {
    Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        String::new(),
    ))
}

fn open_with_users(texts: &[&str]) -> (Arc<SessionManager>, String) {
    let sessions = sessions();
    let sid = sessions
        .open_session_sync("/p", "default", Some("test/test-primary-model"))
        .unwrap();
    sessions
        .insert_detail_rows(
            &sid,
            &texts.iter().map(|t| user_text(*t)).collect::<Vec<_>>(),
        )
        .unwrap();
    (sessions, sid)
}

fn fold_visible(sessions: &SessionManager, sid: &str) -> Vec<(u64, WorkingPayload)> {
    let events = sessions.data().events_blocking(sid).unwrap();
    let surface = fold_surface(&events).unwrap();
    project_working_payloads(&surface, |seq| {
        events
            .iter()
            .find(|e| e.seq == seq)
            .ok_or_else(|| {
                litecode::types::LitecodeError::InvalidSessionEvent(format!(
                    "surface seq {seq} missing"
                ))
            })
            .and_then(spine_working_payload)
    })
    .unwrap()
}

fn reader_visible(sessions: &SessionManager, sid: &str) -> Vec<(u64, WorkingPayload)> {
    sessions
        .data()
        .working_set_blocking(sid)
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.log_seq.expect("reader working row must have seq"),
                row.payload,
            )
        })
        .collect()
}

fn text(payload: &WorkingPayload) -> String {
    item_text_preview(&payload.model_item())
}

fn assert_reader_matches_fold(sessions: &SessionManager, sid: &str) {
    assert_eq!(
        reader_visible(sessions, sid),
        fold_visible(sessions, sid),
        "reader working_set_blocking must preserve the same payloads as fold_surface"
    );
}

#[test]
fn append_rows_reader_working_set_matches_full_fold() {
    let (sessions, sid) = open_with_users(&["a", "b", "c"]);
    let visible = fold_visible(&sessions, &sid);
    assert_eq!(
        visible
            .iter()
            .map(|(seq, payload)| (*seq, text(payload)))
            .collect::<Vec<_>>(),
        vec![(0, "a".into()), (1, "b".into()), (2, "c".into())]
    );
    assert_reader_matches_fold(&sessions, &sid);
}

#[test]
fn keep_recent_compact_reader_matches_fold_checkpoint_then_kept() {
    let (sessions, sid) = open_with_users(&["a", "b", "c"]);
    let expected = sessions.data().revision_blocking(&sid).unwrap_or(0);
    sessions
        .mutate_blocking(SessionMutation::Compact {
            session_id: sid.clone(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            summary: "sum".into(),
            token_estimate: 10,
            kept_from: Some(1),
            expected_prefix: None,
            usage: None,
        })
        .unwrap();
    let visible = fold_visible(&sessions, &sid);
    let checkpoint = visible
        .first()
        .expect("checkpoint row")
        .1
        .checkpoint()
        .expect("checkpoint payload");
    assert_eq!(checkpoint.summary, "sum");
    assert_eq!(checkpoint.from, 0);
    assert_eq!(checkpoint.to, 1);
    assert_eq!(
        text(&visible[0].1).matches("<compacted-summary>").count(),
        1,
        "checkpoint projection must contain exactly one wrapper"
    );
    assert!(
        visible
            .iter()
            .any(|(seq, payload)| *seq == 1 && text(payload) == "b"),
        "keep-recent must retain seq 1"
    );
    assert_reader_matches_fold(&sessions, &sid);
}

#[test]
fn revert_then_commit_discard_returns_fold_window_and_does_not_append() {
    let (sessions, sid) = open_with_users(&["a", "b", "c"]);
    assert_eq!(sessions.entry_wire_seq_cursor(&sid), (2, 3));
    sessions.entry_revert_to_user_anchor(&sid, 1).unwrap();
    // Active tail drops to 0; the allocator high-water stays at 3 (seqs 1..2 are
    // deleted but must never be reused).
    assert_eq!(sessions.entry_wire_seq_cursor(&sid), (0, 3));

    let stale = vec![
        WorkingRow::pending(user_text("a")),
        WorkingRow::pending(user_text("b")),
        WorkingRow::pending(user_text("c")),
        WorkingRow::pending(user_text("stale")),
    ];
    let (kind, working, _) = sessions
        .commit_turn_delta(&sid, stale, 2, "t1", &[])
        .unwrap();
    assert!(
        matches!(kind, CommitKind::Idempotent),
        "truncated log must discard the delta, got {kind:?}"
    );
    let got: Vec<(u64, String)> = working
        .iter()
        .map(|row| {
            (
                row.log_seq.expect("discarded window is persisted"),
                item_text_preview(&row.payload.model_item()),
            )
        })
        .collect();
    assert_eq!(
        got,
        fold_visible(&sessions, &sid)
            .iter()
            .map(|(seq, payload)| (*seq, text(payload)))
            .collect::<Vec<_>>()
    );
    assert_eq!(got, vec![(0, "a".into())]);
    assert_eq!(sessions.entry_wire_seq_cursor(&sid), (0, 3));

    sessions
        .insert_detail_rows(&sid, &[user_text("after")])
        .unwrap();
    assert_eq!(
        sessions.entry_wire_seq_cursor(&sid),
        (3, 4),
        "next append after discard must use the persisted high-water, not MAX(seq)+1"
    );
    assert_reader_matches_fold(&sessions, &sid);
}

#[test]
fn commit_applied_window_matches_fold() {
    let (sessions, sid) = open_with_users(&["a"]);
    let persisted = sessions.data().working_set_blocking(&sid).unwrap();
    let mut rows = persisted;
    rows.push(WorkingRow::pending(user_text("b")));
    let max_seq = sessions.entry_wire_seq_cursor(&sid).0;
    let (kind, working, _) = sessions
        .commit_turn_delta(&sid, rows, max_seq, "t1", &[])
        .unwrap();
    assert!(!matches!(kind, CommitKind::Idempotent));
    let got: Vec<(u64, String)> = working
        .iter()
        .map(|row| {
            (
                row.log_seq.expect("applied rows are persisted"),
                item_text_preview(&row.payload.model_item()),
            )
        })
        .collect();
    assert_eq!(
        got,
        fold_visible(&sessions, &sid)
            .iter()
            .map(|(seq, payload)| (*seq, text(payload)))
            .collect::<Vec<_>>()
    );
    assert_eq!(got, vec![(0, "a".into()), (1, "b".into())]);
}

#[test]
fn pipeline_begin_turn_pending_tail_has_no_log_seq() {
    let (sessions, sid) = open_with_users(&["a", "b"]);
    let pipeline = ContextPipeline::new(
        0,
        Context {
            cwd: std::env::temp_dir(),
            workspace_paths: litecode::config::WorkspacePaths::for_legacy_root(
                &std::env::temp_dir(),
            ),
            agents_md: None,
            claude_md: None,
        },
        sessions.data_root_path(),
    );
    let mut rows = pipeline.begin_turn(&sessions, &sid).unwrap();
    assert!(rows.iter().all(|row| row.log_seq.is_some()));
    assert_eq!(
        rows.iter()
            .map(|row| (
                row.log_seq.unwrap(),
                item_text_preview(&row.payload.model_item())
            ))
            .collect::<Vec<_>>(),
        fold_visible(&sessions, &sid)
            .iter()
            .map(|(seq, payload)| (*seq, text(payload)))
            .collect::<Vec<_>>()
    );

    rows.push(WorkingRow::pending(user_text("pending")));
    assert!(
        rows.last().is_some_and(|row| row.log_seq.is_none()),
        "unpersisted tail must not carry a seq"
    );
    let persisted: Vec<_> = rows.iter().filter_map(|row| row.log_seq).collect();
    assert_eq!(persisted, vec![0, 1]);
}
fn compact(sessions: &SessionManager, sid: &str, summary: &str, kept_from: Option<u64>) {
    let expected = sessions.data().revision_blocking(sid).unwrap_or(0);
    sessions
        .mutate_blocking(SessionMutation::Compact {
            session_id: sid.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            summary: summary.into(),
            token_estimate: 10,
            kept_from,
            expected_prefix: None,
            usage: None,
        })
        .unwrap();
}

/// First visible seq of the reader, which is the checkpoint whenever the
/// session compacted. Reading from the checkpoint instead of seq 0 is only
/// correct if the rows it produces are the full fold's rows, so every case below
/// compares the two.
fn assert_reader_starts_at_checkpoint(sessions: &SessionManager, sid: &str) {
    let first = reader_visible(sessions, sid)
        .first()
        .map(|(seq, _)| *seq)
        .expect("non-empty working set");
    let events = sessions.data().events_blocking(sid).unwrap();
    let checkpoint = events
        .iter()
        .filter(|event| matches!(event.event_type, litecode::session::EventType::Compacted))
        .map(|event| event.seq)
        .max()
        .expect("session compacted, so a checkpoint row exists");
    assert_eq!(first, checkpoint, "reader must begin at the newest checkpoint");
}

#[test]
fn compact_then_append_reader_matches_fold() {
    let (sessions, sid) = open_with_users(&["a", "b", "c"]);
    compact(&sessions, &sid, "sum", Some(2));
    sessions
        .insert_detail_rows(&sid, &[user_text("d"), user_text("e")])
        .unwrap();
    assert_reader_matches_fold(&sessions, &sid);
    assert_reader_starts_at_checkpoint(&sessions, &sid);
}

#[test]
fn two_compacts_reader_matches_fold() {
    let (sessions, sid) = open_with_users(&["a", "b", "c"]);
    compact(&sessions, &sid, "first", Some(1));
    sessions.insert_detail_rows(&sid, &[user_text("d")]).unwrap();
    // The second keep boundary sits inside the first compaction's kept tail, so
    // the older checkpoint row is inside the read range and must be dropped.
    compact(&sessions, &sid, "second", Some(2));
    sessions.insert_detail_rows(&sid, &[user_text("e")]).unwrap();
    assert_reader_matches_fold(&sessions, &sid);
    assert_reader_starts_at_checkpoint(&sessions, &sid);
    let visible = fold_visible(&sessions, &sid);
    let checkpoint = visible[0].1.checkpoint().expect("checkpoint payload");
    // `from` is the seq of the first shadowed node, which is the *previous*
    // checkpoint (seq 3) — above `to`, because a compact shadows a prefix of the
    // surface and the old checkpoint sits at the front with a high seq.
    assert_eq!((checkpoint.from, checkpoint.to), (3, 2));
}

#[test]
fn empty_keep_compact_reader_matches_fold() {
    let (sessions, sid) = open_with_users(&["a", "b"]);
    compact(&sessions, &sid, "sum", None);
    sessions.insert_detail_rows(&sid, &[user_text("c")]).unwrap();
    assert_reader_matches_fold(&sessions, &sid);
    assert_reader_starts_at_checkpoint(&sessions, &sid);
    let visible = fold_visible(&sessions, &sid);
    assert_eq!(visible.len(), 2, "summary only, then the row appended after it");
    let checkpoint = visible[0].1.checkpoint().expect("checkpoint payload");
    // An empty keep ends at the checkpoint's own seq, so the fold starts exactly
    // at that row.
    assert_eq!((checkpoint.from, checkpoint.to), (0, 2));
}

#[test]
fn revert_then_compact_reader_matches_fold() {
    let (sessions, sid) = open_with_users(&["a", "b", "c"]);
    sessions.entry_revert_to_user_anchor(&sid, 1).unwrap();
    sessions.insert_detail_rows(&sid, &[user_text("after")]).unwrap();
    compact(&sessions, &sid, "sum", Some(3));
    assert_reader_matches_fold(&sessions, &sid);
    assert_reader_starts_at_checkpoint(&sessions, &sid);
}
/// A compact on a session with no surface rows has no predecessor range, so the
/// body's \`from\` and \`to\` are both the checkpoint's own seq — the only place
/// \`from == to\` is legal.
#[test]
fn empty_surface_compact_reader_matches_fold() {
    let (sessions, sid) = open_with_users(&[]);
    compact(&sessions, &sid, "sum", None);
    assert_reader_matches_fold(&sessions, &sid);
    let visible = fold_visible(&sessions, &sid);
    assert_eq!(visible.len(), 1, "only the checkpoint survives");
    let checkpoint = visible[0].1.checkpoint().expect("checkpoint payload");
    assert_eq!(
        (checkpoint.from, checkpoint.to),
        (visible[0].0, visible[0].0),
        "no predecessor range: from == to == the checkpoint's own seq"
    );
    assert_reader_starts_at_checkpoint(&sessions, &sid);
}
