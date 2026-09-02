//! SessionData typed CRUD through the single writer.

mod common;

use common::SessionDataFixture;
use litecode::session::SessionRevision;
use litecode::session::WorkingRow;
use litecode::session::data::command::{CommitKind, MutationId, SessionMutation};
use litecode::types::user_text;

fn open() -> (
    SessionDataFixture,
    std::sync::Arc<litecode::session::SessionData>,
) {
    let fixture = SessionDataFixture::new();
    let data = fixture.data.clone();
    (fixture, data)
}

#[test]
fn mutation_enum_is_closed() {
    fn visit(m: &SessionMutation) {
        match m {
            SessionMutation::Create { .. }
            | SessionMutation::Apply { .. }
            | SessionMutation::InsertDetails { .. }
            | SessionMutation::PersistItem { .. }
            | SessionMutation::AppendJobExit { .. }
            | SessionMutation::SealInProgress { .. }
            | SessionMutation::CommitTurnDelta { .. }
            | SessionMutation::Compact { .. }
            | SessionMutation::SaveTaskState { .. }
            | SessionMutation::SaveContextMeter { .. }
            | SessionMutation::SetAgent { .. }
            | SessionMutation::SetModel { .. }
            | SessionMutation::SetThinkingTier { .. }
            | SessionMutation::SetContextMode { .. }
            | SessionMutation::Delete { .. }
            | SessionMutation::ClearOrphanedModelIds { .. }
            | SessionMutation::RebuildFts { .. } => {}
        }
    }
    let m = SessionMutation::RebuildFts {
        operation_id: MutationId::new(),
    };
    visit(&m);
}

#[test]
fn create_append_meta_delete() {
    let (_dir, data) = open();
    let sid = data.create_session("/proj", "default", Some("m")).unwrap();
    assert_eq!(data.revision_blocking(&sid).unwrap(), 1);
    data.insert_items(&sid, &[user_text("one"), user_text("two")])
        .unwrap();
    assert_eq!(data.transcript_blocking(&sid).unwrap().len(), 2);
    let rev = data.revision_blocking(&sid).unwrap();
    let receipt = data
        .mutate_blocking(SessionMutation::SetAgent {
            session_id: sid.clone(),
            expected_revision: rev,
            operation_id: MutationId::new(),
            agent_id: "reviewer".into(),
        })
        .unwrap();
    assert!(matches!(receipt.outcome, CommitKind::MetaUpdated));
    assert_eq!(data.meta_blocking(&sid).unwrap().agent_id, "reviewer");
    let rev = data.revision_blocking(&sid).unwrap();
    data.mutate_blocking(SessionMutation::Delete {
        session_id: sid.clone(),
        expected_revision: rev,
        operation_id: MutationId::new(),
    })
    .unwrap();
    assert!(data.meta_blocking(&sid).is_err());
}

#[test]
fn parent_delete_cascades_child() {
    let (_dir, data) = open();
    let parent = data.create_session("/proj", "default", None).unwrap();
    let child = data
        .mutate_blocking(SessionMutation::Create {
            operation_id: MutationId::new(),
            project: "/proj".into(),
            agent_id: "reviewer".into(),
            model_id: None,
            parent_session_id: Some(parent.clone()),
            parent_call_id: Some("call_1".into()),
        })
        .unwrap()
        .session_id;
    assert_eq!(
        data.list_child_ids_blocking(&parent).unwrap(),
        vec![child.clone()]
    );
    let rev = data.revision_blocking(&parent).unwrap();
    data.mutate_blocking(SessionMutation::Delete {
        session_id: parent.clone(),
        expected_revision: rev,
        operation_id: MutationId::new(),
    })
    .unwrap();
    assert!(data.meta_blocking(&parent).is_err());
    assert!(data.meta_blocking(&child).is_err());
}

#[test]
fn idempotent_operation_id_returns_same_receipt() {
    let (_dir, data) = open();
    let sid = data.create_session("/p", "default", None).unwrap();
    let op = MutationId("stable-op-1".into());
    let rev = data.revision_blocking(&sid).unwrap();
    let first = data
        .mutate_blocking(SessionMutation::InsertDetails {
            session_id: sid.clone(),
            expected_revision: rev,
            operation_id: op.clone(),
            items: vec![user_text("once")],
            turn_id: String::new(),
        })
        .unwrap();
    let second = data
        .mutate_blocking(SessionMutation::InsertDetails {
            session_id: sid.clone(),
            expected_revision: rev,
            operation_id: op,
            items: vec![user_text("once")],
            turn_id: String::new(),
        })
        .unwrap();
    assert_eq!(first.revision, second.revision);
    assert_eq!(first.operation_id, second.operation_id);
    assert_eq!(data.transcript_blocking(&sid).unwrap().len(), 1);
}

#[test]
fn create_operation_id_is_idempotent() {
    let (_dir, data) = open();
    let op = MutationId("stable-create-op".into());
    let create = || SessionMutation::Create {
        operation_id: op.clone(),
        project: "/p".into(),
        agent_id: "default".into(),
        model_id: None,
        parent_session_id: None,
        parent_call_id: None,
    };
    let first = data.mutate_blocking(create()).unwrap();
    let second = data.mutate_blocking(create()).unwrap();
    assert_eq!(first.session_id, second.session_id);
    assert_eq!(
        data.list_session_ids_blocking().unwrap(),
        vec![first.session_id]
    );
}

#[test]
fn conflict_on_stale_revision() {
    let (_dir, data) = open();
    let sid = data.create_session("/p", "default", None).unwrap();
    data.insert_items(&sid, &[user_text("a")]).unwrap();
    let err = data
        .mutate_blocking(SessionMutation::InsertDetails {
            session_id: sid,
            expected_revision: SessionRevision(1).0,
            operation_id: MutationId::new(),
            items: vec![user_text("b")],
            turn_id: String::new(),
        })
        .unwrap_err();
    match err {
        litecode::types::LitecodeError::SessionConflict { expected, actual } => {
            assert_eq!(expected, 1);
            assert_eq!(actual, 2);
        }
        other => panic!("expected conflict, got {other}"),
    }
}

#[test]
fn commit_turn_delta_receipt_carries_user_preview() {
    let (_dir, data) = open();
    let sid = data.create_session("/p", "default", None).unwrap();
    let rev = data.revision_blocking(&sid).unwrap();
    let receipt = data
        .mutate_blocking(SessionMutation::CommitTurnDelta {
            session_id: sid,
            expected_revision: rev,
            operation_id: MutationId::new(),
            rows: vec![WorkingRow::pending(user_text("hello from user"))],
            expected_max_seq: -1,
            turn_id: "t1".into(),
        })
        .unwrap();
    let (preview, _updated_at) = receipt
        .preview
        .expect("user append must return last_message preview");
    assert_eq!(preview, "hello from user");
}
