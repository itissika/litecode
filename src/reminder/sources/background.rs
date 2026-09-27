use crate::reminder::engine::{BackgroundFacts, SpineReminderView};
use crate::reminder::kinds::{BackgroundBody, ChildCountsBody, Reminder, ReminderKind};

pub(crate) fn background(view: &SpineReminderView, facts: &BackgroundFacts) -> Option<Reminder> {
    if view.compacted_head.is_none() {
        return None;
    }
    if view.kinds_after_compacted.contains(&ReminderKind::Background) {
        return None;
    }
    let idle = facts.children_idle;
    let running = facts.children_running;
    if facts.running_bash.is_empty() && running == 0 && idle == 0 {
        return None;
    }
    let text = render(facts);
    Some(Reminder::Background(BackgroundBody {
        running_bash: facts.running_bash.clone(),
        children: ChildCountsBody { running, idle },
        text,
    }))
}

fn render(facts: &BackgroundFacts) -> String {
    let mut parts = Vec::new();
    if !facts.running_bash.is_empty() {
        let mut lines = vec![format!("Running bash: {}", facts.running_bash.len())];
        for job in &facts.running_bash {
            lines.push(format!(
                "- {}  {}  ({})",
                job.job_id, job.command, job.output_file
            ));
        }
        parts.push(lines.join("\n"));
    }
    let total = facts.children_running + facts.children_idle;
    if total > 0 {
        parts.push(format!(
            "Children: {total} ({} running, {} idle)",
            facts.children_running, facts.children_idle
        ));
    }
    parts.join("\n\n")
}
