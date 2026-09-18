use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRef {
    pub relative_path: String,
    pub slug: String,
    /// Content hash the session last authored or acknowledged.
    pub revision: Option<String>,
}

impl PlanRef {
    pub fn new(slug: &str) -> Self {
        Self {
            relative_path: format!(".litecode/plan/{slug}.md"),
            slug: slug.to_string(),
            revision: None,
        }
    }

    pub fn with_revision(slug: &str, revision: Option<String>) -> Self {
        Self {
            revision,
            ..Self::new(slug)
        }
    }
}

/// Stable content hash for plan revision tracking.
pub fn plan_content_revision(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("{:x}", hasher.finalize())
}

/// Content hash of an existing plan file.
pub fn plan_file_revision(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|bytes| plan_content_revision(&bytes))
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

/// Clear active plan overlay when the plan file is missing on disk.
///
/// Callers pass the workspace-owned plan directory explicitly: this runs on
/// shared worker threads where the thread-local `active_paths()` fallback would
/// point at the process launch directory.
pub fn prune_stale_active_plan(state: &mut TaskReminders, plan_dir: &Path) -> bool {
    let Some(plan) = state.active_plan.as_ref() else {
        return false;
    };
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
        let plan_root = dir.path().join(".litecode/plan");
        std::fs::create_dir_all(&plan_root).unwrap();
        std::fs::write(plan_root.join("gone.md"), "# old").unwrap();
        std::fs::remove_file(plan_root.join("gone.md")).unwrap();

        let mut state = TaskReminders {
            todos: vec![],
            active_plan: Some(PlanRef::new("gone")),
        };
        assert!(prune_stale_active_plan(&mut state, &plan_root));
        assert!(state.active_plan.is_none());
    }

    #[test]
    fn prune_stale_active_plan_clears_when_plan_dir_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plan_root = dir.path().join(".litecode/plan");
        let mut state = TaskReminders {
            todos: vec![],
            active_plan: Some(PlanRef::new("gone")),
        };
        assert!(prune_stale_active_plan(&mut state, &plan_root));
        assert!(state.active_plan.is_none());
    }

    #[test]
    fn prune_stale_active_plan_keeps_when_file_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plan_root = dir.path().join(".litecode/plan");
        std::fs::create_dir_all(&plan_root).unwrap();
        std::fs::write(plan_root.join("keep.md"), "# keep").unwrap();

        let mut state = TaskReminders {
            todos: vec![],
            active_plan: Some(PlanRef::new("keep")),
        };
        assert!(!prune_stale_active_plan(&mut state, &plan_root));
        assert!(state.active_plan.is_some());
    }

    #[test]
    fn plan_content_revision_is_stable_and_content_sensitive() {
        let first = plan_content_revision(b"# Plan");
        assert_eq!(first, plan_content_revision(b"# Plan"));
        assert_ne!(first, plan_content_revision(b"# Plan v2"));
    }
}
