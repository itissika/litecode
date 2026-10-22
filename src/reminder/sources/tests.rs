//! R3 (WP-026): minimal behavior tests per reminder source. Test-only; no product change.
use std::path::PathBuf;

use super::*;
use crate::reminder::engine::{SeamCtx, SpineReminderView};
use crate::reminder::kinds::{
    Reminder, ReminderKind, StepBudgetBody,
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
