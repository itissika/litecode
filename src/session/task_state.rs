use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::WorkspacePaths;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRef {
    pub relative_path: String,
    pub slug: String,
}

impl PlanRef {
    pub fn new(slug: &str) -> Self {
        Self {
            relative_path: format!(".litecode/plan/{slug}.md"),
            slug: slug.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    pub status: TodoStatus,
    pub priority: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TodoStatus {
    Pending,
    #[serde(rename = "in_progress")]
    InProgress,
    Completed,
}

pub fn render_todos(todos: &[TodoItem]) -> String {
    if todos.is_empty() {
        return "No todos found".into();
    }

    let mut out = String::new();
    for item in todos {
        let status_icon = match item.status {
            TodoStatus::Pending => "○",
            TodoStatus::InProgress => "◐",
            TodoStatus::Completed => "●",
        };
        let priority_str = item
            .priority
            .as_ref()
            .map(|p| format!("[{}]", p))
            .unwrap_or_default();
        out.push_str(&format!(
            "{} [{}] {}{}\n",
            status_icon,
            item.id,
            item.content,
            if priority_str.is_empty() {
                "".into()
            } else {
                format!(" {}", priority_str)
            },
        ));
    }
    out.trim_end().to_string()
}

#[derive(Debug, Clone, Default)]
pub struct TaskReminders {
    pub todos: Vec<TodoItem>,
    pub active_plan: Option<PlanRef>,
}

impl TaskReminders {
    pub fn has_todo_overlay(&self) -> bool {
        !self.todos.is_empty()
    }

    /// Apply overlay-drop rules (e.g. all completed → empty) before read or persist.
    pub fn normalize(&mut self) {
        self.drop_completed_todos();
    }

    pub fn drop_completed_todos(&mut self) {
        if self.todos.iter().all(|t| t.status == TodoStatus::Completed) {
            self.todos.clear();
        }
    }

    pub fn clear_plan(&mut self) {
        self.active_plan = None;
    }

    pub fn set_active_plan(&mut self, plan: PlanRef) {
        self.active_plan = Some(plan);
    }
}

pub fn plan_dir(paths: &WorkspacePaths) -> PathBuf {
    paths.plan_dir.clone()
}

/// Clear active plan overlay when the plan file is missing on disk.
pub fn prune_stale_active_plan(state: &mut TaskReminders) -> bool {
    let Some(plan) = state.active_plan.as_ref() else {
        return false;
    };
    let paths = crate::config::workspace::active_paths();
    let plan_dir = plan_dir(&paths);
    if !plan_dir.is_dir() {
        state.clear_plan();
        return true;
    }
    let plan_file = plan_dir.join(format!("{}.md", plan.slug));
    if plan_file.is_file() {
        return false;
    }
    state.clear_plan();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_clears_all_completed_todos() {
        let mut state = TaskReminders {
            todos: vec![TodoItem {
                id: "t1".into(),
                content: "done".into(),
                status: TodoStatus::Completed,
                priority: None,
            }],
            active_plan: None,
        };
        state.normalize();
        assert!(state.todos.is_empty());
    }

    #[test]
    fn normalize_keeps_pending_todos() {
        let mut state = TaskReminders {
            todos: vec![
                TodoItem {
                    id: "t1".into(),
                    content: "pending".into(),
                    status: TodoStatus::Pending,
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
        state.normalize();
        assert_eq!(state.todos.len(), 2);
    }

    #[test]
    fn normalize_keeps_in_progress_todos() {
        let mut state = TaskReminders {
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
        state.normalize();
        assert_eq!(state.todos.len(), 2);
    }

    #[test]
    fn plan_ref_path_is_flat_under_plan_dir() {
        let plan = PlanRef::new("my-plan");
        assert_eq!(plan.slug, "my-plan");
        assert_eq!(plan.relative_path, ".litecode/plan/my-plan.md");
    }

    #[test]
    fn prune_stale_active_plan_clears_when_file_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::config::workspace::set_runtime_paths(
            crate::config::WorkspacePaths::for_legacy_root(dir.path()),
        );
        let plan_root = dir.path().join(".litecode/plan");
        std::fs::create_dir_all(&plan_root).unwrap();
        std::fs::write(plan_root.join("gone.md"), "# old").unwrap();
        std::fs::remove_file(plan_root.join("gone.md")).unwrap();

        let mut state = TaskReminders {
            todos: vec![],
            active_plan: Some(PlanRef::new("gone")),
        };
        assert!(prune_stale_active_plan(&mut state));
        assert!(state.active_plan.is_none());
    }

    #[test]
    fn prune_stale_active_plan_clears_when_plan_dir_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::config::workspace::set_runtime_paths(
            crate::config::WorkspacePaths::for_legacy_root(dir.path()),
        );
        let mut state = TaskReminders {
            todos: vec![],
            active_plan: Some(PlanRef::new("gone")),
        };
        assert!(prune_stale_active_plan(&mut state));
        assert!(state.active_plan.is_none());
    }

    #[test]
    fn prune_stale_active_plan_keeps_when_file_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::config::workspace::set_runtime_paths(
            crate::config::WorkspacePaths::for_legacy_root(dir.path()),
        );
        let plan_root = dir.path().join(".litecode/plan");
        std::fs::create_dir_all(&plan_root).unwrap();
        std::fs::write(plan_root.join("keep.md"), "# keep").unwrap();

        let mut state = TaskReminders {
            todos: vec![],
            active_plan: Some(PlanRef::new("keep")),
        };
        assert!(!prune_stale_active_plan(&mut state));
        assert!(state.active_plan.is_some());
    }
}
