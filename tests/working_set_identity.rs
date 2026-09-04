//! Locks the turn/writer working window against reader full-log fold.
//! Asserts seq identity of the visible set, not whether a hot path scanned the log.

use std::sync::Arc;

use litecode::config::TurnGuard;
use litecode::context_pipeline::{Context, ContextPipeline};
use litecode::session::data::command::{CommitKind, MutationId, SessionMutation};
use litecode::session::event::spine_agent_item;
use litecode::session::manager::SessionManager;
use litecode::session::surface::project_working_pairs;
use litecode::session::{WorkingRow, fold_surface, project_items};
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
        .open_session_sync("/p", "default", Some("m"))
        .unwrap();
    sessions
        .insert_detail_rows(
            &sid,
            &texts.iter().map(|t| user_text(*t)).collect::<Vec<_>>(),
        )
        .unwrap();
    (sessions, sid)
}

fn fold_visible(
    sessions: &SessionManager,
    sid: &str,
) -> Vec<(u64, String)> {
    let events = sessions.data().events_blocking(sid).unwrap();
    let surface = fold_surface(&events).unwrap();
    project_working_pairs(&surface, |seq| {
        events
            .iter()
            .find(|e| e.seq == seq)
            .ok_or_else(|| {
                litecode::types::LitecodeError::InvalidSessionEvent(format!(
                    "surface seq {seq} missing"
                ))
            })
            .and_then(spine_agent_item)
    })
    .unwrap()
    .into_iter()
    .map(|(seq, item)| (seq, item_text_preview(&item)))
    .collect()
}

fn reader_visible(
    sessions: &SessionManager,
    sid: &str,
) -> Vec<(u64, String)> {
    sessions
        .data()
        .working_set_blocking(sid)
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.log_seq.expect("reader working row must have seq"),
                item_text_preview(&row.item),
            )
        })
        .collect()
}

fn assert_reader_matches_fold(sessions: &SessionManager, sid: &str) {
    assert_eq!(
        reader_visible(sessions, sid),
        fold_visible(sessions, sid),
        "reader working_set_blocking must equal fold_surface + project_working_pairs"
    );
}

#[test]
fn append_rows_reader_working_set_matches_full_fold() {
    let (sessions, sid) = open_with_users(&["a", "b", "c"]);
    assert_eq!(
        fold_visible(&sessions, &sid),
        vec![
            (0, "a".into()),
            (1, "b".into()),
            (2, "c".into())
        ]
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
            summary: user_text("sum"),
            token_estimate: 10,
            kept_from: Some(1),
            expected_prefix: None,
        })
        .unwrap();
    let visible = fold_visible(&sessions, &sid);
    assert_eq!(visible[0].1, "sum", "checkpoint must lead the working set");
    assert!(
        visible.iter().any(|(seq, text)| *seq == 1 && text == "b"),
        "keep-recent must retain seq 1: {visible:?}"
    );
    assert_reader_matches_fold(&sessions, &sid);
}

#[test]
fn revert_then_commit_discard_returns_fold_window_and_does_not_append() {
    let (sessions, sid) = open_with_users(&["a", "b", "c"]);
    assert_eq!(sessions.entry_wire_seq_cursor(&sid), (2, 3));
    sessions.entry_revert_to_user_anchor(&sid, 1).unwrap();
    assert_eq!(sessions.entry_wire_seq_cursor(&sid), (0, 1));

    let stale = vec![
        WorkingRow::pending(user_text("a")),
        WorkingRow::pending(user_text("b")),
        WorkingRow::pending(user_text("c")),
        WorkingRow::pending(user_text("stale")),
    ];
    let (kind, working, _) = sessions
        .commit_turn_delta(&sid, stale, 2, "t1")
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
                item_text_preview(&row.item),
            )
        })
        .collect();
    assert_eq!(got, fold_visible(&sessions, &sid));
    assert_eq!(got, vec![(0, "a".into())]);
    assert_eq!(sessions.entry_wire_seq_cursor(&sid), (0, 1));

    sessions
        .insert_detail_rows(&sid, &[user_text("after")])
        .unwrap();
    assert_eq!(
        sessions.entry_wire_seq_cursor(&sid),
        (1, 2),
        "next append after discard must use truncated MAX(seq)+1"
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
        .commit_turn_delta(&sid, rows, max_seq, "t1")
        .unwrap();
    assert!(!matches!(kind, CommitKind::Idempotent));
    let got: Vec<(u64, String)> = working
        .iter()
        .map(|row| {
            (
                row.log_seq.expect("applied rows are persisted"),
                item_text_preview(&row.item),
            )
        })
        .collect();
    assert_eq!(got, fold_visible(&sessions, &sid));
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
            .map(|row| (row.log_seq.unwrap(), item_text_preview(&row.item)))
            .collect::<Vec<_>>(),
        fold_visible(&sessions, &sid)
    );

    let mut items = project_items(&rows);
    items.push(user_text("pending"));
    litecode::session::align_working(&mut rows, &items);
    assert!(
        rows.last().is_some_and(|row| row.log_seq.is_none()),
        "unpersisted tail must not carry a seq"
    );
    let persisted: Vec<_> = rows
        .iter()
        .filter_map(|row| row.log_seq)
        .collect();
    assert_eq!(persisted, vec![0, 1]);
}
