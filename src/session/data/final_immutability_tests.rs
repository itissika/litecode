//! Source-store invariants the projection is allowed to lean on.
//!
//! The projection does not guess "did this row change?". It trusts two things
//! instead, and this file is where they are pinned down:
//!
//! 1. **A finalised row never changes again.** Rows are written `InProgress`
//!    while a turn is streaming and sealed in place exactly once. After that the
//!    body at `(session_id, seq)` is fixed for the life of the row.
//! 2. **A session at rest has no `in_progress` rows.** Whoever is responsible
//!    for that (see the writer), the projection may assume it when reading.
//!
//! If either one breaks, search silently loses documents. They are cheap to
//! assert here and expensive to discover in production.

use std::collections::BTreeMap;
use std::sync::Arc;

use rusqlite::Connection;
use tempfile::TempDir;

use super::command::{MutationId, SessionMutation};
use super::sqlite::session::SessionApply;
use crate::authority::responses::{
    AssistantRole, FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall, MessageItem,
    OutputMessage, OutputMessageContent, OutputStatus, OutputTextContent,
};
use crate::session::{SessionData, WorkspaceWriteLease};
use crate::types::{Item, assistant_text, user_text};

struct Fixture {
    dir: TempDir,
    _lease: WorkspaceWriteLease,
    data: Arc<SessionData>,
    sid: String,
}

fn open() -> Fixture {
    let dir = TempDir::new().expect("tempdir");
    let lease = WorkspaceWriteLease::acquire(dir.path()).expect("lease");
    let data = SessionData::open(&lease, &dir.path().join("sessions.db")).expect("open");
    let sid = data
        .create_session("/p", "default", None)
        .expect("create session");
    Fixture {
        dir,
        _lease: lease,
        data,
        sid,
    }
}

impl Fixture {
    fn mutate(&self, mutation: SessionMutation) -> crate::session::CommitReceipt {
        self.data.mutate_blocking(mutation).expect("mutation")
    }

    /// For the calls whose failure is the point.
    fn mutate_result(
        &self,
        mutation: SessionMutation,
    ) -> crate::types::Result<crate::session::CommitReceipt> {
        self.data.mutate_blocking(mutation)
    }

    fn db(&self) -> Connection {
        Connection::open(self.dir.path().join("sessions.db")).expect("reopen")
    }

    /// The session's current revision, which every mutation must declare.
    fn rev(&self) -> u64 {
        self.db()
            .query_row(
                "SELECT revision FROM sessions WHERE id = ?1",
                rusqlite::params![self.sid],
                |r| r.get::<_, i64>(0),
            )
            .expect("revision") as u64
    }

    /// `seq -> body` for rows that are no longer in flight.
    fn settled(&self) -> BTreeMap<i64, String> {
        let conn = self.db();
        let mut stmt = conn
            .prepare(
                "SELECT seq, ifnull(body, '') FROM transcript_items
                 WHERE session_id = ?1 AND state != 'in_progress'",
            )
            .expect("prepare");
        let it = stmt
            .query_map(rusqlite::params![self.sid], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })
            .expect("query");
        it.collect::<rusqlite::Result<BTreeMap<_, _>>>()
            .expect("collect")
    }

    fn in_progress_seqs(&self) -> Vec<i64> {
        let conn = self.db();
        let mut stmt = conn
            .prepare(
                "SELECT seq FROM transcript_items
                 WHERE session_id = ?1 AND state = 'in_progress' ORDER BY seq",
            )
            .expect("prepare");
        let it = stmt
            .query_map(rusqlite::params![self.sid], |r| r.get::<_, i64>(0))
            .expect("query");
        it.collect::<rusqlite::Result<Vec<_>>>().expect("collect")
    }

    /// Open a row the way the streaming path does: in flight because the caller
    /// says so, not because the payload happens to carry `status`.
    fn streaming(&self, id: &str, text: &str) -> SessionMutation {
        SessionMutation::BeginStreamItem {
            session_id: self.sid.clone(),
            expected_revision: 0,
            operation_id: MutationId::new(),
            item: Item::Message(MessageItem::Output(OutputMessage {
                id: id.into(),
                role: AssistantRole::Assistant,
                content: vec![OutputMessageContent::OutputText(OutputTextContent {
                    text: text.into(),
                    annotations: vec![],
                    logprobs: None,
                })],
                status: OutputStatus::InProgress,
                phase: None,
            })),
            turn_id: "t-stream".into(),
        }
    }

    fn finished(&self, id: &str, text: &str) -> Item {
        Item::Message(MessageItem::Output(OutputMessage {
            id: id.into(),
            role: AssistantRole::Assistant,
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: text.into(),
                annotations: vec![],
                logprobs: None,
            })],
            status: OutputStatus::Completed,
            phase: None,
        }))
    }

    /// More text for a row that is already streaming.
    fn streaming_more(&self, seq: i64, id: &str, text: &str) -> SessionMutation {
        SessionMutation::UpdateStreamItem {
            session_id: self.sid.clone(),
            expected_revision: 0,
            operation_id: MutationId::new(),
            seq: seq as u64,
            item: Item::Message(MessageItem::Output(OutputMessage {
                id: id.into(),
                role: AssistantRole::Assistant,
                content: vec![OutputMessageContent::OutputText(OutputTextContent {
                    text: text.into(),
                    annotations: vec![],
                    logprobs: None,
                })],
                status: OutputStatus::InProgress,
                phase: None,
            })),
        }
    }

    fn seal_all(&self) {
        self.mutate(self.with_rev(SessionMutation::SealInProgress {
            session_id: self.sid.clone(),
            expected_revision: 0,
            operation_id: MutationId::new(),
        }));
    }

    /// Revision drifts on every write; the callers here do not care.
    fn with_rev(&self, mut m: SessionMutation) -> SessionMutation {
        let rev = self.rev();
        match &mut m {
            SessionMutation::PersistItem {
                expected_revision, ..
            }
            | SessionMutation::BeginStreamItem {
                expected_revision, ..
            }
            | SessionMutation::UpdateStreamItem {
                expected_revision, ..
            }
            | SessionMutation::SealStreamItem {
                expected_revision, ..
            }
            | SessionMutation::InsertDetails {
                expected_revision, ..
            }
            | SessionMutation::CommitTurnDelta {
                expected_revision, ..
            }
            | SessionMutation::SealInProgress {
                expected_revision, ..
            }
            | SessionMutation::Compact {
                expected_revision, ..
            }
            | SessionMutation::Apply {
                expected_revision, ..
            } => *expected_revision = rev,
            _ => {}
        }
        m
    }
}

/// The invariant the projection's whole change-detection strategy rests on.
///
/// The ledger is kept for the **whole lifetime** of the session: a seq that has
/// ever been settled keeps its body on record even after a revert deletes the
/// row. That is what catches seq reuse — a later row landing on a deleted seq
/// shows up as a previously-bound seq coming back with a different body.
#[test]
fn a_finalised_row_is_never_rewritten() {
    let f = open();
    let mut ledger = f.settled();

    let check = |f: &Fixture, ledger: &mut BTreeMap<i64, String>, step: &str| {
        let now = f.settled();
        for (seq, body) in now.iter() {
            if let Some(before) = ledger.get(seq) {
                assert_eq!(
                    before, body,
                    "step `{step}`: seq {seq} was settled once with {before:?} and came back as \
                     {body:?} (a final row rewritten, or a deleted seq reused)"
                );
            }
        }
        // Deleted seqs are deliberately *not* forgotten: keeping them is what
        // makes a reused seq observable after the row that first claimed it is
        // already gone.
        ledger.extend(now);
    };

    f.mutate(f.with_rev(SessionMutation::InsertDetails {
        session_id: f.sid.clone(),
        expected_revision: 0,
        operation_id: MutationId::new(),
        items: vec![user_text("first turn")],
        turn_id: "t1".into(),
    }));
    check(&f, &mut ledger, "insert details");

    // Stream an assistant message in pieces, then finalise it: this is the
    // InProgress -> Final transition the projection must never observe.
    f.mutate(f.with_rev(f.streaming("asst_1", "Hel")));
    assert_eq!(f.in_progress_seqs().len(), 1, "the row is in flight");
    let stream_seq = f.in_progress_seqs()[0];
    f.mutate(f.with_rev(f.streaming_more(stream_seq, "asst_1", "Hello wor")));
    check(&f, &mut ledger, "stream again");

    // Commit carries the seq allocated by BeginStreamItem. Session lifecycle
    // transitions are addressed by seq; provider ids are only call-scoped
    // association keys owned by StreamProjection.
    let mut rows = f.data.working_set_blocking(&f.sid).expect("working set");
    let streamed = rows
        .iter_mut()
        .find(|row| row.log_seq == Some(stream_seq as u64))
        .expect("streamed row");
    streamed.item = f.finished("asst_1", "Hello world");
    f.mutate(f.with_rev(SessionMutation::CommitTurnDelta {
        session_id: f.sid.clone(),
        expected_revision: 0,
        operation_id: MutationId::new(),
        rows,
        expected_max_seq: -1,
        turn_id: "t1".into(),
    }));
    check(&f, &mut ledger, "commit turn delta");
    assert!(f.in_progress_seqs().is_empty(), "the row was finalised");

    // A cancelled turn seals whatever is still in flight.
    f.mutate(f.with_rev(f.streaming("asst_2", "partial")));
    check(&f, &mut ledger, "stream a second row");
    f.mutate(f.with_rev(SessionMutation::SealInProgress {
        session_id: f.sid.clone(),
        expected_revision: 0,
        operation_id: MutationId::new(),
    }));
    check(&f, &mut ledger, "seal in progress");

    // Re-sealing is idempotent and must not rewrite the settled bodies.
    let receipt = f.mutate(f.with_rev(SessionMutation::SealInProgress {
        session_id: f.sid.clone(),
        expected_revision: 0,
        operation_id: MutationId::new(),
    }));
    check(&f, &mut ledger, "re-seal");
    assert_eq!(
        receipt.outcome,
        crate::session::data::command::CommitKind::Sealed { seqs: vec![] },
        "sealing again touches nothing"
    );

    // Append a second user turn so its seq is a real revert anchor.
    f.mutate(f.with_rev(SessionMutation::InsertDetails {
        session_id: f.sid.clone(),
        expected_revision: 0,
        operation_id: MutationId::new(),
        items: vec![user_text("second turn"), assistant_text("tail")],
        turn_id: "t2".into(),
    }));
    check(&f, &mut ledger, "append after seal");

    let settled_before = f.settled().len();
    f.mutate(f.with_rev(SessionMutation::Apply {
        session_id: f.sid.clone(),
        expected_revision: 0,
        operation_id: MutationId::new(),
        op: SessionApply::Truncate { anchor_seq: 3 },
    }));
    check(&f, &mut ledger, "revert");
    assert!(
        f.settled().len() < settled_before,
        "the revert really removed rows"
    );

    // An append after the revert must take a fresh seq from the high-water, not
    // resurrect one the revert deleted. The lifetime ledger makes a reuse fail
    // here instead of passing silently.
    f.mutate(f.with_rev(SessionMutation::InsertDetails {
        session_id: f.sid.clone(),
        expected_revision: 0,
        operation_id: MutationId::new(),
        items: vec![assistant_text("after revert")],
        turn_id: "t3".into(),
    }));
    check(&f, &mut ledger, "append after revert");
}

/// The other half: nothing may be left in flight once the writes stop.
#[test]
fn sealing_leaves_no_row_in_flight() {
    let f = open();
    for i in 0..3 {
        f.mutate(f.with_rev(f.streaming(&format!("asst_{i}"), "half")));
    }
    assert_eq!(f.in_progress_seqs().len(), 3);

    f.seal_all();

    assert!(
        f.in_progress_seqs().is_empty(),
        "a session at rest must have no in_progress rows"
    );
    let conn = f.db();
    let states: Vec<String> = conn
        .prepare("SELECT DISTINCT state FROM transcript_items WHERE session_id = ?1")
        .unwrap()
        .query_map(rusqlite::params![f.sid], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        states,
        vec!["final".to_string()],
        "only final states remain"
    );
}

/// The rule is enforced where the write happens, not merely hoped for at the
/// call sites.
///
/// Sealing is the only write that changes the content at an existing `seq`, and
/// it is allowed exactly once, while the row is still in flight. Every other
/// "rewrite" of a settled row has to become a new row, because the derived index
/// no longer carries any mechanism for noticing a change after projection — a
/// silent rewrite would leave search serving text that no longer exists.
#[test]
fn sealing_a_settled_row_is_refused() {
    let f = open();
    f.mutate(f.with_rev(f.streaming("asst_a", "the settled text")));
    f.seal_all();

    let before = f.settled();
    let seq: i64 = *before.keys().next().expect("one settled row");

    let rewrite = f.mutate_result(SessionMutation::Apply {
        session_id: f.sid.clone(),
        expected_revision: f.rev(),
        operation_id: MutationId::new(),
        op: SessionApply::Seal {
            seq: seq as u64,
            item: assistant_text("something else entirely"),
        },
    });
    assert!(
        rewrite.is_err(),
        "sealing a settled row must fail loudly rather than corrupt the index"
    );
    assert_eq!(
        f.settled(),
        before,
        "and it must not have written anything on the way out"
    );
}

/// The second guarantee, and the one a crash can break.
///
/// A cancelled or failed turn seals its own rows (`runtime/mod.rs`). A process
/// that *died* cannot. So the repair happens on the next startup, before a
/// single write is served — which is also the one moment it is exactly correct:
/// nothing can be in flight yet, so every `in_progress` row in the database is
/// an orphan. No age heuristic, no guessing.
///
/// The projection leans on this and deliberately does not compensate for it.
/// Searching only settles history, so an orphan is unsearchable until it is
/// sealed; if nothing ever sealed it, it would be unsearchable forever.
#[test]
fn a_restart_seals_rows_the_previous_run_left_in_flight() {
    let dir = TempDir::new().expect("tempdir");
    let db = dir.path().join("sessions.db");
    let lease = WorkspaceWriteLease::acquire(dir.path()).expect("lease");

    let sid = {
        let data = SessionData::open(&lease, &db).expect("open");
        let sid = data.create_session("/p", "default", None).expect("create");
        // A turn starts streaming and the process dies mid-sentence: nothing
        // cancels it, nothing seals it, and the row keeps its `in_progress`.
        data.mutate_blocking(SessionMutation::BeginStreamItem {
            session_id: sid.clone(),
            expected_revision: 1,
            operation_id: MutationId::new(),
            item: crate::types::Item::Message(MessageItem::Output(OutputMessage {
                id: "asst_live".into(),
                role: AssistantRole::Assistant,
                content: vec![OutputMessageContent::OutputText(OutputTextContent {
                    text: "half a thought".into(),
                    annotations: vec![],
                    logprobs: None,
                })],
                status: OutputStatus::InProgress,
                phase: None,
            })),
            turn_id: "t-crash".into(),
        })
        .expect("persist");
        sid
    };

    // Nothing sealed it: the writer was dropped the way a crash drops one.
    assert_eq!(
        in_flight(&db, &sid),
        vec![0],
        "the row is still in flight after the drop"
    );
    assert!(
        settled_bodies(&db, &sid).is_empty(),
        "an in-flight row is not searchable history yet"
    );

    // Reopening is the restart. `SessionData::open` returns only once the
    // startup repair has run, so there is nothing to poll for.
    let data = SessionData::open(&lease, &db).expect("reopen");

    assert!(
        in_flight(&db, &sid).is_empty(),
        "the restart sealed every in-flight row"
    );
    let settled = settled_bodies(&db, &sid);
    assert_eq!(
        settled.keys().copied().collect::<Vec<_>>(),
        vec![0],
        "the row is settled now"
    );
    let body = &settled[&0];
    let item: Item = serde_json::from_str(body).expect("body parses as an item");
    match item {
        Item::Message(MessageItem::Output(m)) => {
            assert_eq!(
                m.status,
                OutputStatus::Incomplete,
                "sealing says the content is short, not that it is complete"
            );
            match &m.content[0] {
                OutputMessageContent::OutputText(t) => {
                    assert_eq!(t.text, "half a thought", "what did arrive is kept");
                }
                other => panic!("unexpected content {other:?}"),
            }
        }
        other => panic!("unexpected item {other:?}"),
    }

    // And the repair really does reach search: sealing makes the row searchable,
    // which is a set difference the reconcile finds on its own.
    let keys = data
        .reader()
        .searchable_keys_blocking(Some(&sid))
        .expect("keys");
    assert_eq!(
        keys,
        vec![(sid.clone(), 0)],
        "the once-invisible row is now part of searchable history"
    );

    // A second restart over a clean store changes nothing and says nothing.
    drop(data);
    let data = SessionData::open(&lease, &db).expect("reopen again");
    assert!(in_flight(&db, &sid).is_empty());
    assert_eq!(settled_bodies(&db, &sid).len(), 1, "no duplicate rows");
    drop(data);
}

fn function_call(call_id: &str, name: &str) -> Item {
    Item::FunctionCall(FunctionToolCall {
        arguments: "{}".into(),
        call_id: call_id.into(),
        namespace: None,
        name: name.into(),
        id: None,
        status: Some(OutputStatus::Completed),
    })
}

fn insert_details(data: &SessionData, sid: &str, turn_id: &str, items: Vec<Item>) {
    data.mutate_blocking(SessionMutation::InsertDetails {
        session_id: sid.to_string(),
        expected_revision: data.revision_blocking(sid).expect("revision"),
        operation_id: MutationId::new(),
        items,
        turn_id: turn_id.into(),
    })
    .expect("insert");
}

/// A crash during a tool leaves a final call and no output. Nothing is running
/// at the next open, and the calls are still the tail, so startup appends the
/// interrupt result the agent loop can no longer write. The calls stay as they
/// were. A second open finds them answered and appends nothing.
#[test]
fn a_restart_seals_unanswered_calls_left_at_the_tail() {
    let dir = TempDir::new().expect("tempdir");
    let db = dir.path().join("sessions.db");
    let lease = WorkspaceWriteLease::acquire(dir.path()).expect("lease");

    let (sid, call_bodies) = {
        let data = SessionData::open(&lease, &db).expect("open");
        let sid = data.create_session("/p", "default", None).expect("create");
        insert_details(
            &data,
            &sid,
            "t-wait",
            vec![
                user_text("go"),
                function_call("call_a", "subagent_wait"),
                function_call("call_b", "wait_shell"),
            ],
        );
        let bodies = settled_bodies(&db, &sid);
        drop(data);
        (sid, bodies)
    };

    let data = SessionData::open(&lease, &db).expect("reopen");
    let settled = settled_bodies(&db, &sid);
    assert_eq!(
        settled.keys().copied().collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4],
        "the two tail calls each gain one output"
    );
    assert_eq!(
        settled[&1], call_bodies[&1],
        "the first call is not rewritten"
    );
    assert_eq!(
        settled[&2], call_bodies[&2],
        "the second call is not rewritten"
    );
    assert_interrupted_output(&settled[&3], "call_a", "subagent_wait");
    assert_interrupted_output(&settled[&4], "call_b", "wait_shell");

    let conn = Connection::open(&db).expect("db");
    let turn_ids: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT turn_id FROM transcript_items
                 WHERE session_id = ?1 AND seq IN (3, 4) ORDER BY seq",
            )
            .expect("prepare");
        stmt.query_map(rusqlite::params![sid], |row| row.get(0))
            .expect("query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect")
    };
    assert_eq!(
        turn_ids,
        ["t-wait", "t-wait"],
        "the result stays on the call's turn"
    );
    drop(conn);
    drop(data);

    let _data = SessionData::open(&lease, &db).expect("reopen again");
    assert_eq!(
        settled_bodies(&db, &sid).len(),
        5,
        "a second restart does not append another result"
    );
}

/// Later history means the result would land after a subsequent turn. Leave the
/// call unanswered on disk; the model view pads it in place.
#[test]
fn a_restart_leaves_an_unanswered_call_that_later_history_follows() {
    let dir = TempDir::new().expect("tempdir");
    let db = dir.path().join("sessions.db");
    let lease = WorkspaceWriteLease::acquire(dir.path()).expect("lease");

    let sid = {
        let data = SessionData::open(&lease, &db).expect("open");
        let sid = data.create_session("/p", "default", None).expect("create");
        insert_details(
            &data,
            &sid,
            "t-old",
            vec![function_call("call_old", "read"), user_text("continue")],
        );
        drop(data);
        sid
    };

    let _data = SessionData::open(&lease, &db).expect("reopen");
    let settled = settled_bodies(&db, &sid);
    assert_eq!(
        settled.keys().copied().collect::<Vec<_>>(),
        vec![0, 1],
        "startup must not append a result after later history"
    );
    for body in settled.values() {
        let item: Item = serde_json::from_str(body).expect("item");
        assert!(
            !matches!(item, Item::FunctionCallOutput(_)),
            "the hanging call stays unanswered on disk"
        );
    }
}

/// A call still in flight is sealed incomplete first, then closed with the same
/// interrupt result, because the tail is that call once the seal finishes.
#[test]
fn a_restart_seals_an_in_flight_call_and_appends_its_interrupt_result() {
    let dir = TempDir::new().expect("tempdir");
    let db = dir.path().join("sessions.db");
    let lease = WorkspaceWriteLease::acquire(dir.path()).expect("lease");

    let sid = {
        let data = SessionData::open(&lease, &db).expect("open");
        let sid = data.create_session("/p", "default", None).expect("create");
        data.mutate_blocking(SessionMutation::BeginStreamItem {
            session_id: sid.clone(),
            expected_revision: data.revision_blocking(&sid).expect("revision"),
            operation_id: MutationId::new(),
            item: function_call("call_live", "bash"),
            turn_id: "t-crash".into(),
        })
        .expect("begin");
        drop(data);
        sid
    };

    assert_eq!(in_flight(&db, &sid), vec![0]);

    let _data = SessionData::open(&lease, &db).expect("reopen");
    assert!(in_flight(&db, &sid).is_empty(), "the call row is sealed");
    let settled = settled_bodies(&db, &sid);
    assert_eq!(settled.keys().copied().collect::<Vec<_>>(), vec![0, 1]);
    let call: Item = serde_json::from_str(&settled[&0]).expect("call");
    match call {
        Item::FunctionCall(call) => {
            assert_eq!(call.call_id, "call_live");
            assert_eq!(call.status, Some(OutputStatus::Incomplete));
        }
        other => panic!("expected the sealed call, got {other:?}"),
    }
    assert_interrupted_output(&settled[&1], "call_live", "bash");
}

fn assert_interrupted_output(body: &str, call_id: &str, name: &str) {
    let item: Item = serde_json::from_str(body).expect("output item");
    match item {
        Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: got_id,
            output: FunctionCallOutput::Text(text),
            ..
        }) => {
            assert_eq!(got_id, call_id);
            assert_eq!(
                text,
                format!(
                    "tool '{name}' was interrupted: no result was recorded \
                     (session recovered before completion)"
                )
            );
        }
        other => panic!("expected an interrupt result, got {other:?}"),
    }
}

/// A revert must not let a later append reuse a deleted seq, across a reopen.
#[test]
fn reopen_after_revert_does_not_reuse_a_deleted_seq() {
    let dir = TempDir::new().expect("tempdir");
    let db = dir.path().join("sessions.db");
    let lease = WorkspaceWriteLease::acquire(dir.path()).expect("lease");

    let sid = {
        let data = SessionData::open(&lease, &db).expect("open");
        let sid = data.create_session("/p", "default", None).expect("create");
        data.mutate_blocking(SessionMutation::InsertDetails {
            session_id: sid.clone(),
            expected_revision: 1,
            operation_id: MutationId::new(),
            items: vec![user_text("u0"), user_text("u1"), user_text("u2")],
            turn_id: "t1".into(),
        })
        .expect("insert");
        data.mutate_blocking(SessionMutation::Apply {
            session_id: sid.clone(),
            expected_revision: 2,
            operation_id: MutationId::new(),
            op: SessionApply::Truncate { anchor_seq: 1 },
        })
        .expect("revert");
        sid
    };

    // Reopening rehydrates the persisted high-water from `sessions.next_seq`.
    let data = SessionData::open(&lease, &db).expect("reopen");
    data.mutate_blocking(SessionMutation::InsertDetails {
        session_id: sid.clone(),
        expected_revision: 3,
        operation_id: MutationId::new(),
        items: vec![user_text("after reopen")],
        turn_id: "t2".into(),
    })
    .expect("append");
    drop(data);

    let conn = Connection::open(&db).expect("db");
    let seqs: Vec<i64> = {
        let mut stmt = conn
            .prepare("SELECT seq FROM transcript_items WHERE session_id = ?1 ORDER BY seq")
            .expect("prepare");
        stmt.query_map(rusqlite::params![sid], |r| r.get(0))
            .expect("query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect")
    };
    assert_eq!(
        seqs,
        vec![0, 3],
        "the appended row takes the persisted high-water (3), never the deleted seq 1"
    );
}

fn in_flight(db: &std::path::Path, sid: &str) -> Vec<i64> {
    let conn = Connection::open(db).expect("open db");
    let mut stmt = conn
        .prepare(
            "SELECT seq FROM transcript_items
             WHERE session_id = ?1 AND state = 'in_progress' ORDER BY seq",
        )
        .expect("prepare");
    let it = stmt
        .query_map(rusqlite::params![sid], |r| r.get::<_, i64>(0))
        .expect("query");
    it.collect::<rusqlite::Result<Vec<_>>>().expect("collect")
}

fn settled_bodies(db: &std::path::Path, sid: &str) -> BTreeMap<i64, String> {
    let conn = Connection::open(db).expect("open db");
    let mut stmt = conn
        .prepare(
            "SELECT seq, ifnull(body, '') FROM transcript_items
             WHERE session_id = ?1 AND state != 'in_progress'",
        )
        .expect("prepare");
    let it = stmt
        .query_map(rusqlite::params![sid], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })
        .expect("query");
    it.collect::<rusqlite::Result<BTreeMap<_, _>>>()
        .expect("collect")
}
