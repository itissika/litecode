//! R3 (WP-026): minimal behavior tests per reminder source. Test-only; no product change.
use std::path::PathBuf;

use super::*;
use crate::reminder::engine::{BackgroundFacts, SeamCtx, SpineReminderView, TaskFacts};
use crate::reminder::kinds::{
    BashExitEntry, CustomToolSettledEntry, PlanChangedBody, PlanPointer, Reminder, ReminderKind,
    RunningBash, SettledChild, StepBudgetBody, TodoSnap,
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
fn job() -> RunningBash {
    RunningBash {
        job_id: "j1".into(),
        command: "sleep 9".into(),
        output_file: "/o".into(),
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
fn background_restore_gates_on_compaction_and_content() {
    let facts = BackgroundFacts {
        running_bash: vec![job()],
        children_running: 1,
        children_idle: 2,
    };
    assert!(background(&SpineReminderView::default(), &facts).is_none());
    let Some(Reminder::Background(b)) = background(&compacted(), &facts) else {
        panic!()
    };
    assert!(
        b.text.contains("Running bash: 1") && b.text.contains("Children: 3 (1 running, 2 idle)")
    );
    let mut v = compacted();
    v.kinds_after_compacted.insert(ReminderKind::Background);
    assert!(background(&v, &facts).is_none());
    assert!(background(&compacted(), &BackgroundFacts::default()).is_none());
}

#[test]
fn files_changed_caps_at_twenty_with_more_count() {
    assert!(files_changed(&[]).is_none());
    let paths: Vec<String> = (0..23).map(|i| format!("f{i}")).collect();
    let Some(Reminder::FilesChanged(b)) = files_changed(&paths) else {
        panic!()
    };
    assert_eq!(b.paths.len(), 20);
    assert_eq!(b.more, 3);
    assert!(b.text.ends_with("and 3 more"));
    let Some(Reminder::FilesChanged(b)) = files_changed(&paths[..20]) else {
        panic!()
    };
    assert_eq!(b.more, 0);
    assert!(!b.text.contains("more"), "exactly at cap: no overflow line");
}

#[test]
fn bash_exit_distinguishes_kill_and_lists_running() {
    assert!(bash_exit(&[], &[job()]).is_none());
    let exits = vec![
        BashExitEntry {
            job_id: "a".into(),
            command: "x".into(),
            exit_code: 0,
            killed: false,
            output_file: "/a".into(),
        },
        BashExitEntry {
            job_id: "b".into(),
            command: "y".into(),
            exit_code: 137,
            killed: true,
            output_file: "/b".into(),
        },
    ];
    let Some(Reminder::BashExit(b)) = bash_exit(&exits, &[]) else {
        panic!()
    };
    assert!(b.text.contains("Background bash a exited with code 0."));
    assert!(
        b.text
            .contains("The user stopped background bash b (Kill).")
    );
    assert!(b.text.ends_with("running: 0"));
}

#[test]
fn subagent_settled_detail_overrides_listing() {
    let child = SettledChild {
        child_session_id: "c1".into(),
        turn_id: "t".into(),
        agent: "".into(),
        reason: "done".into(),
    };
    assert!(subagent_settled(&[], "x").is_none());
    let Some(Reminder::SubagentSettled(b)) = subagent_settled(&[child.clone()], "  ") else {
        panic!()
    };
    assert!(b.text.contains("child_session_id: c1") && !b.text.contains("agent:"));
    let Some(Reminder::SubagentSettled(b)) = subagent_settled(&[child], "custom batch") else {
        panic!()
    };
    assert!(b.text.ends_with("custom batch") && !b.text.contains("child_session_id"));
    let named = SettledChild {
        agent: "explore".into(),
        ..child_named()
    };
    let Some(Reminder::SubagentSettled(b)) = subagent_settled(&[named], "") else {
        panic!()
    };
    assert!(b.text.contains("agent: explore"));
}

fn child_named() -> SettledChild {
    SettledChild {
        child_session_id: "c2".into(),
        turn_id: "t".into(),
        agent: String::new(),
        reason: "done".into(),
    }
}

#[test]
fn custom_tool_settled_lists_entries_and_skips_empty_detail() {
    assert!(custom_tool_settled(&[]).is_none());
    let e = CustomToolSettledEntry {
        job_id: "j".into(),
        call_id: "c".into(),
        tool_name: "t".into(),
        status: "ok".into(),
        detail: "".into(),
    };
    let Some(Reminder::CustomToolSettled(b)) = custom_tool_settled(&[e]) else {
        panic!()
    };
    assert!(
        b.text.contains("settled: 1")
            && b.text.contains("status: ok")
            && !b.text.contains("detail:")
    );
}

#[test]
fn plan_changed_diff_against_seen_and_spine() {
    let mut f = TaskFacts {
        active_plan: Some(plan()),
        plan_disk_revision: Some("r2".into()),
        ..Default::default()
    };
    f.plan_seen_revision = Some("r2".into());
    assert!(
        plan_changed(&SpineReminderView::default(), &f).is_none(),
        "seen == disk"
    );
    f.plan_seen_revision = Some("r1".into());
    assert!(plan_changed(&SpineReminderView::default(), &f).is_some());
    let mut v = SpineReminderView::default();
    v.latest_by_kind.insert(
        ReminderKind::PlanChanged,
        Reminder::PlanChanged(PlanChangedBody {
            relative_path: "plans/a.md".into(),
            revision: "r2".into(),
            text: String::new(),
        }),
    );
    assert!(
        plan_changed(&v, &f).is_none(),
        "already announced this revision"
    );
    f.plan_disk_revision = Some(String::new());
    assert!(
        plan_changed(&SpineReminderView::default(), &f).is_none(),
        "empty revision"
    );
}

#[test]
fn step_budget_warns_within_three_once_per_turn() {
    let v = SpineReminderView::default();
    assert!(step_budget(&ctx(5, 0, "t"), &v).is_none(), "unlimited");
    assert!(step_budget(&ctx(6, 10, "t"), &v).is_none(), "4 remain");
    let Some(Reminder::StepBudget(b)) = step_budget(&ctx(7, 10, "t"), &v) else {
        panic!()
    };
    assert!(b.text.contains("3 steps remain"));
    let mut seen = SpineReminderView::default();
    seen.latest_by_kind.insert(
        ReminderKind::StepBudget,
        Reminder::StepBudget(StepBudgetBody {
            step: 7,
            max_steps: 10,
            turn_id: "t".into(),
            text: String::new(),
        }),
    );
    assert!(step_budget(&ctx(8, 10, "t"), &seen).is_none(), "same turn");
    assert!(
        step_budget(&ctx(8, 10, "t2"), &seen).is_some(),
        "new turn re-warns"
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
