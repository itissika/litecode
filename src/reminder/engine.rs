//! Request-seam assembly. Sources run in a fixed order and only append.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::kinds::{
    CustomToolSettledEntry, PlanPointer, Reminder, ReminderKind, RunningBash, SettledChild,
    TodoSnap,
};
use super::sources;

/// Facts the seam already sensed. Sources do not reach back into the runtime.
#[derive(Debug, Clone)]
pub struct SeamCtx {
    pub session_id: String,
    pub turn_id: String,
    pub step: u64,
    pub max_steps: u64,
    pub cwd: PathBuf,
}

/// One surface node, in spine order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpineNode {
    Compacted,
    Reminder(Reminder),
    Other,
}

/// Baseline for Diff and Restore. Only the current spine counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpineReminderView {
    pub compacted_head: Option<u64>,
    pub latest_by_kind: HashMap<ReminderKind, Reminder>,
    pub kinds_after_compacted: HashSet<ReminderKind>,
}

impl SpineReminderView {
    pub fn latest(&self, kind: ReminderKind) -> Option<&Reminder> {
        self.latest_by_kind.get(&kind)
    }
}

pub fn view_from_spine(nodes: &[(u64, SpineNode)]) -> SpineReminderView {
    let compacted_head = nodes.first().and_then(|(seq, node)| {
        if matches!(node, SpineNode::Compacted) {
            Some(*seq)
        } else {
            None
        }
    });
    let mut latest_by_kind = HashMap::new();
    let mut kinds_after_compacted = HashSet::new();
    let mut passed_head = compacted_head.is_none();
    for (seq, node) in nodes {
        if Some(*seq) == compacted_head {
            passed_head = true;
            continue;
        }
        if let SpineNode::Reminder(reminder) = node {
            let kind = reminder.kind();
            latest_by_kind.insert(kind, reminder.clone());
            if compacted_head.is_some() && passed_head {
                kinds_after_compacted.insert(kind);
            }
        }
    }
    SpineReminderView {
        compacted_head,
        latest_by_kind,
        kinds_after_compacted,
    }
}

#[derive(Debug, Clone, Default)]
pub struct TaskFacts {
    pub todos: Vec<TodoSnap>,
    pub active_plan: Option<PlanPointer>,
    /// On-disk hash when the active plan file exists.
    pub plan_disk_revision: Option<String>,
    /// Revision the session last authored or acknowledged.
    pub plan_seen_revision: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct BackgroundFacts {
    pub running_bash: Vec<RunningBash>,
    pub children_running: usize,
    pub children_idle: usize,
}

/// Sensed world for one seam. Event sources are already drained by the caller.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub tasks: TaskFacts,
    pub background: BackgroundFacts,
    pub bash_exits: Vec<super::kinds::BashExitEntry>,
    pub bash_running: Vec<RunningBash>,
    pub settled: Vec<SettledChild>,
    /// Full batch text for the subagent reminder, without the reminder wrapper.
    pub settled_detail: String,
    pub custom_tool_settled: Vec<CustomToolSettledEntry>,
    pub changed_paths: Vec<String>,
}

/// Reminders to append, in write order. Empty means the seam changes nothing.
pub fn sync(ctx: &SeamCtx, view: &SpineReminderView, facts: &Facts) -> Vec<Reminder> {
    let mut out = Vec::new();
    if let Some(reminder) = sources::env(ctx, view) {
        out.push(reminder);
    }
    if let Some(reminder) = sources::tasks(view, &facts.tasks) {
        out.push(reminder);
    }
    if let Some(reminder) = sources::background(view, &facts.background) {
        out.push(reminder);
    }
    if let Some(reminder) = sources::plan_changed(view, &facts.tasks) {
        out.push(reminder);
    }
    if let Some(reminder) = sources::files_changed(&facts.changed_paths) {
        out.push(reminder);
    }
    if let Some(reminder) = sources::bash_exit(&facts.bash_exits, &facts.bash_running) {
        out.push(reminder);
    }
    if let Some(reminder) = sources::subagent_settled(&facts.settled, &facts.settled_detail) {
        out.push(reminder);
    }
    if let Some(reminder) = sources::custom_tool_settled(&facts.custom_tool_settled) {
        out.push(reminder);
    }
    if let Some(reminder) = sources::step_budget(ctx, view) {
        out.push(reminder);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reminder::kinds::{PlanPointer, Reminder, TodoSnap};
    use crate::reminder::render_item;
    use crate::types::item_text_preview;

    fn ctx(step: u64) -> SeamCtx {
        SeamCtx {
            session_id: "s".into(),
            turn_id: "t".into(),
            step,
            max_steps: 10,
            cwd: PathBuf::from("/work"),
        }
    }

    #[test]
    fn first_seam_writes_env_only() {
        let reminders = sync(&ctx(1), &SpineReminderView::default(), &Facts::default());
        let kinds: Vec<_> = reminders.iter().map(Reminder::kind).collect();
        assert!(kinds.contains(&ReminderKind::Env));
        assert!(!kinds.contains(&ReminderKind::Tasks));
        assert!(!kinds.contains(&ReminderKind::StepBudget));
        let rendered = item_text_preview(&render_item(&reminders[0]));
        assert!(rendered.starts_with("<system-reminder>\n"));
        assert!(rendered.ends_with("\n</system-reminder>"));
    }

    #[test]
    fn restore_fires_once_after_compaction() {
        let facts = Facts {
            tasks: TaskFacts {
                todos: vec![TodoSnap {
                    id: "t1".into(),
                    content: "active".into(),
                    status: "in_progress".into(),
                    priority: None,
                }],
                active_plan: Some(PlanPointer {
                    relative_path: ".litecode/plan/calm.md".into(),
                    slug: "calm".into(),
                }),
                ..TaskFacts::default()
            },
            ..Facts::default()
        };
        let mut view = SpineReminderView {
            compacted_head: Some(4),
            ..SpineReminderView::default()
        };
        let first = sync(&ctx(1), &view, &facts);
        assert!(first
            .iter()
            .any(|reminder| reminder.kind() == ReminderKind::Tasks));
        view.kinds_after_compacted.insert(ReminderKind::Tasks);
        let second = sync(&ctx(2), &view, &facts);
        assert!(!second
            .iter()
            .any(|reminder| reminder.kind() == ReminderKind::Tasks));
    }

    #[test]
    fn serde_roundtrip_keeps_kind_and_text() {
        let reminders = sync(&ctx(1), &SpineReminderView::default(), &Facts::default());
        for reminder in reminders {
            let json = serde_json::to_string(&reminder).unwrap();
            let back: Reminder = serde_json::from_str(&json).unwrap();
            assert_eq!(back, reminder);
            assert_eq!(back.kind().visibility(), reminder.kind().visibility());
        }
    }
}
