//! R3 (WP-026): minimal behavior tests per reminder source. Test-only; no product change.
use std::path::PathBuf;

use super::*;
use crate::reminder::engine::{SeamCtx, SpineReminderView, TaskFacts};
use crate::reminder::kinds::{
    PlanPointer, Reminder, ReminderKind, TodoSnap,
};

fn ctx(step: u64, max_steps: u64, turn: &str) -> SeamCtx {
    SeamCtx {
        session_id: "s".into(),
        turn_id: turn.into(),
        step,
        max_steps,
        cwd: PathBuf::from("/w"),
    }
}
fn compacted() -> SpineReminderView {
    SpineReminderView {
        compacted_head: Some(1),
        ..Default::default()
    }
}
fn plan() -> PlanPointer {
    PlanPointer {
        relative_path: "plans/a.md".into(),
        slug: "a".into(),
    }
}
fn todo() -> TodoSnap {
    TodoSnap {
        id: "1".into(),
        content: "do it".into(),
        status: "in_progress".into(),
        priority: None,
    }
}

#[test]
fn tasks_restore_only_after_compaction_and_once() {
    let facts = TaskFacts {
        todos: vec![todo()],
        active_plan: Some(plan()),
        ..Default::default()
    };
    assert!(
        tasks(&SpineReminderView::default(), &facts).is_none(),
        "no compacted head => no restore"
    );
    let r = tasks(&compacted(), &facts).expect("restore after compaction");
    let Reminder::Tasks(b) = r else { panic!() };
    assert!(b.text.contains("[~] do it") && b.text.contains("plans/a.md"));
    let mut v = compacted();
    v.kinds_after_compacted.insert(ReminderKind::Tasks);
    assert!(tasks(&v, &facts).is_none(), "already restored after head");
    assert!(
        tasks(&compacted(), &TaskFacts::default()).is_none(),
        "nothing to restore"
    );
}

#[test]
fn env_emits_once_until_snapshot_changes() {
    let c = ctx(1, 0, "t");
    let first = env(&c, &SpineReminderView::default()).expect("first seam emits env");
    let mut v = SpineReminderView::default();
    v.latest_by_kind.insert(ReminderKind::Env, first);
    assert!(env(&c, &v).is_none(), "unchanged snapshot");
    let other = SeamCtx {
        cwd: PathBuf::from("/elsewhere"),
        ..c
    };
    assert!(env(&other, &v).is_some(), "cwd change re-emits");
}
