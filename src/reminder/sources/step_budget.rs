use crate::reminder::engine::{SeamCtx, SpineReminderView};
use crate::reminder::kinds::{Reminder, ReminderKind, StepBudgetBody};

const WARN_WITHIN: u64 = 3;

pub(crate) fn step_budget(ctx: &SeamCtx, view: &SpineReminderView) -> Option<Reminder> {
    if ctx.max_steps == 0 || ctx.step == 0 {
        return None;
    }
    let remaining = ctx.max_steps.saturating_sub(ctx.step);
    if remaining > WARN_WITHIN {
        return None;
    }
    if let Some(Reminder::StepBudget(previous)) = view.latest(ReminderKind::StepBudget)
        && previous.turn_id == ctx.turn_id
    {
        return None;
    }
    let text = format!(
        "Step budget: {step} of {max} used in this turn. {remaining} steps remain before the turn stops. Finish the current task or wrap up.",
        step = ctx.step,
        max = ctx.max_steps,
        remaining = remaining
    );
    Some(Reminder::StepBudget(StepBudgetBody {
        step: ctx.step,
        max_steps: ctx.max_steps,
        turn_id: ctx.turn_id.clone(),
        text,
    }))
}
