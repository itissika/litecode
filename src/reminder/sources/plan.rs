use crate::reminder::engine::{SpineReminderView, TaskFacts};
use crate::reminder::kinds::{PlanChangedBody, Reminder, ReminderKind};

pub(crate) fn plan_changed(view: &SpineReminderView, facts: &TaskFacts) -> Option<Reminder> {
    let plan = facts.active_plan.as_ref()?;
    let revision = facts.plan_disk_revision.as_deref()?.to_string();
    if revision.is_empty() {
        return None;
    }
    if facts.plan_seen_revision.as_deref() == Some(revision.as_str()) {
        return None;
    }
    if let Some(Reminder::PlanChanged(previous)) = view.latest(ReminderKind::PlanChanged)
        && previous.relative_path == plan.relative_path
        && previous.revision == revision
    {
        return None;
    }
    let text = format!(
        "[Plan updated] {} changed since you last read it. Read that file before continuing.",
        plan.relative_path
    );
    Some(Reminder::PlanChanged(PlanChangedBody {
        relative_path: plan.relative_path.clone(),
        revision,
        text,
    }))
}
