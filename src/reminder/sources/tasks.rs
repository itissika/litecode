use crate::reminder::engine::{SpineReminderView, TaskFacts};
use crate::reminder::kinds::{Reminder, ReminderKind, TasksBody};

pub(crate) fn tasks(view: &SpineReminderView, facts: &TaskFacts) -> Option<Reminder> {
    if view.compacted_head.is_none() {
        return None;
    }
    if view.kinds_after_compacted.contains(&ReminderKind::Tasks) {
        return None;
    }
    if facts.todos.is_empty() && facts.active_plan.is_none() {
        return None;
    }
    let text = render(facts);
    Some(Reminder::Tasks(TasksBody {
        todos: facts.todos.clone(),
        active_plan: facts.active_plan.clone(),
        text,
    }))
}

fn render(facts: &TaskFacts) -> String {
    let mut parts = Vec::new();
    if !facts.todos.is_empty() {
        let mut lines = vec!["Todos:".to_string()];
        for todo in &facts.todos {
            let mark = match todo.status.as_str() {
                "completed" => "[x]",
                "in_progress" => "[~]",
                _ => "[ ]",
            };
            lines.push(format!("{mark} {}", todo.content));
        }
        parts.push(lines.join("\n"));
    }
    if let Some(plan) = &facts.active_plan {
        parts.push(format!(
            "[Active plan] {}\nAn active plan exists. If it is not finished, re-read the plan and continue the work. If the work is done, call plan finish to clear the active plan state.",
            plan.relative_path
        ));
    }
    parts.join("\n\n")
}
