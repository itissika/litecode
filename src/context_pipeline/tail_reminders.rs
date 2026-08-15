use crate::session::task_state::TaskReminders;
use crate::session::task_state::TodoStatus;

/// Build the post-compaction reminder text from the current task state.
///
/// Called only right after a full context compaction (Plan C: no per-step
/// injection), so the model regains todo/plan awareness after the window reset.
/// Includes the **full todo list** — counts alone are useless to a model that
/// just lost its working memory — plus the active plan path. Returns `None`
/// when there is nothing to remind (no active todos and no active plan).
pub fn build_compaction_content(state: &TaskReminders) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    if state.has_todo_overlay() {
        let mut lines: Vec<String> = vec!["Todos:".to_string()];
        for t in &state.todos {
            let mark = match t.status {
                TodoStatus::Completed => "[x]",
                TodoStatus::InProgress => "[~]",
                TodoStatus::Pending => "[ ]",
            };
            lines.push(format!("{mark} {}", t.content));
        }
        parts.push(lines.join("\n"));
    }

    if let Some(plan) = &state.active_plan {
        parts.push(format!("[Active plan] {}", plan.relative_path));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

/// Append a reminder as a `user_text` Item into a transcript.
///
/// Unused on the compact path: the reminder rides on the checkpoint Item
/// (label first, then this block, then summary prose). Kept as a helper for tests.
#[cfg(test)]
pub fn append_to_llm_view(llm_items: &mut crate::types::Transcript, tail: &str) {
    llm_items.push(crate::types::user_text(format!(
        "<system-reminder>\n{tail}\n</system-reminder>"
    )));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::task_state::{PlanRef, TaskReminders};

    #[test]
    fn build_compaction_content_lists_full_todos() {
        use crate::session::task_state::{TodoItem, TodoStatus};

        let state = TaskReminders {
            todos: vec![
                TodoItem {
                    id: "t1".into(),
                    content: "active".into(),
                    status: TodoStatus::InProgress,
                    priority: None,
                },
                TodoItem {
                    id: "t2".into(),
                    content: "done".into(),
                    status: TodoStatus::Completed,
                    priority: None,
                },
            ],
            active_plan: None,
        };
        let tail = build_compaction_content(&state).expect("tail");
        assert!(tail.contains("[~] active"));
        assert!(tail.contains("[x] done"));
        assert!(
            !tail.contains("completed"),
            "compaction reminder must carry the full list, not a count summary"
        );
    }

    #[test]
    fn build_compaction_content_includes_flat_plan_path() {
        let state = TaskReminders {
            todos: vec![],
            active_plan: Some(PlanRef::new("calm-river")),
        };
        let tail = build_compaction_content(&state).expect("tail");
        assert_eq!(tail, "[Active plan] .litecode/plan/calm-river.md");
    }

    #[test]
    fn build_compaction_content_none_when_empty() {
        let state = TaskReminders {
            todos: vec![],
            active_plan: None,
        };
        assert!(build_compaction_content(&state).is_none());
    }
}
