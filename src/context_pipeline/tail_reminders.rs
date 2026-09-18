use crate::session::manager::ChildCounts;
use crate::session::task_state::PlanRef;
use crate::session::task_state::TaskReminders;
use crate::session::task_state::TodoStatus;

/// Build the post-compaction reminder text from session state.
///
/// Called only right after a full context compaction (Plan C: no per-step
/// injection), so the model regains todo/plan awareness after the window reset.
/// Includes the **full todo list** — counts alone are useless to a model that
/// just lost its working memory — plus the active plan path and a
/// continue/finish hint, plus live child-session counts (state only; details
/// stay behind `subagent_list`). Returns `None`
/// when there is nothing to remind (no active todos, no active plan, no
/// children).
pub fn build_compaction_content(
    state: &TaskReminders,
    children: Option<ChildCounts>,
) -> Option<String> {
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
        parts.push(active_plan_reminder(plan));
    }

    if let Some(counts) = children.filter(|counts| counts.total > 0) {
        parts.push(format!(
            "Children: {} ({} running, {} idle)",
            counts.total,
            counts.running,
            counts.total.saturating_sub(counts.running)
        ));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

/// Reminder for an active plan that exists on disk (callers settle stale
/// pointers first), so it only says how to continue or close the plan.
fn active_plan_reminder(plan: &PlanRef) -> String {
    format!(
        "[Active plan] {}\nAn active plan exists. If it is not finished, re-read the plan and continue the work. If the work is done, call plan finish to clear the active plan state.",
        plan.relative_path
    )
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
        let tail = build_compaction_content(&state, None).expect("tail");
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
        let tail = build_compaction_content(&state, None).expect("tail");
        assert!(tail.starts_with("[Active plan] .litecode/plan/calm-river.md"));
        assert!(tail.contains("re-read the plan and continue the work"));
        assert!(tail.contains("plan finish"));
    }

    #[test]
    fn build_compaction_content_none_when_empty() {
        let state = TaskReminders {
            todos: vec![],
            active_plan: None,
        };
        assert!(build_compaction_content(&state, None).is_none());
        assert!(
            build_compaction_content(
                &state,
                Some(ChildCounts {
                    total: 0,
                    running: 0
                })
            )
            .is_none(),
            "zero children must not add a reminder section"
        );
    }

    #[test]
    fn build_compaction_content_counts_children() {
        let state = TaskReminders::default();
        let tail = build_compaction_content(
            &state,
            Some(ChildCounts {
                total: 3,
                running: 1,
            }),
        )
        .expect("tail");
        assert_eq!(tail, "Children: 3 (1 running, 2 idle)");
    }

    #[test]
    fn build_compaction_content_puts_children_after_task_state() {
        use crate::session::task_state::{TodoItem, TodoStatus};

        let state = TaskReminders {
            todos: vec![TodoItem {
                id: "t1".into(),
                content: "active".into(),
                status: TodoStatus::InProgress,
                priority: None,
            }],
            active_plan: Some(PlanRef::new("calm-river")),
        };
        let tail = build_compaction_content(
            &state,
            Some(ChildCounts {
                total: 2,
                running: 0,
            }),
        )
        .expect("tail");
        assert!(
            tail.ends_with("Children: 2 (0 running, 2 idle)"),
            "children section must come last, got {tail:?}"
        );
    }
}
