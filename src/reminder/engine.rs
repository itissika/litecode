//! Request-seam assembly. Sources run in a fixed order and only append.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::kinds::{
    MemorySnap, PlanPointer, Reminder, ReminderKind, RestoreBody, RunningBash, SettledChild,
    TodoSnap,
};
use super::sources;

/// Facts the seam already sensed. Sources do not reach back into the runtime.
#[derive(Debug, Clone)]
pub struct SeamCtx {
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
            // A restore bundle stands for its parts: each component keeps its own
            // kind on the spine, so Diff (env) and restore-once (tasks, memory,
            // background) still resolve.
            for component in reminder.components() {
                let kind = component.kind();
                latest_by_kind.insert(kind, component.clone());
                if compacted_head.is_some() && passed_head {
                    kinds_after_compacted.insert(kind);
                }
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
}

#[derive(Debug, Clone, Default)]
pub struct MemoryFacts {
    pub entries: Vec<MemorySnap>,
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
    pub memory: MemoryFacts,
    pub background: BackgroundFacts,
    pub bash_exits: Vec<super::kinds::BashExitEntry>,
    pub bash_running: Vec<RunningBash>,
    pub settled: Vec<SettledChild>,
    /// Full batch text for the subagent reminder, without the reminder wrapper.
    pub settled_detail: String,
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
    if let Some(reminder) = sources::memory(view, &facts.memory) {
        out.push(reminder);
    }
    if let Some(reminder) = sources::background(view, &facts.background) {
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
    if view.compacted_head.is_some() {
        fold_restore(&mut out);
    }
    out
}

/// One user item for the post-compaction re-injection.
///
/// After a compacted head, env/tasks/memory/background fire in the same seam and
/// would land as consecutive user items. Folding them into one row keeps the log
/// shape (one row, one frozen body, one seq) while the model sees a single item.
/// A lone reminder is left as-is. Only these four Hidden kinds fold; the Visible
/// bash/subagent cut marks and every other reminder keep their own row.
fn fold_restore(out: &mut Vec<Reminder>) {
    let run = out
        .iter()
        .take_while(|reminder| is_restore_kind(reminder.kind()))
        .count();
    if run < 2 {
        return;
    }
    let parts: Vec<Reminder> = out.drain(..run).collect();
    let text = parts
        .iter()
        .map(Reminder::text)
        .collect::<Vec<_>>()
        .join("\n\n");
    out.insert(0, Reminder::Restore(RestoreBody { parts, text }));
}

fn is_restore_kind(kind: ReminderKind) -> bool {
    matches!(
        kind,
        ReminderKind::Env | ReminderKind::Tasks | ReminderKind::Memory | ReminderKind::Background
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reminder::kinds::{
        EnvBody, MemorySnap, PlanPointer, RestoreBody, RunningBash, TasksBody, TodoSnap,
    };

    fn ctx() -> SeamCtx {
        SeamCtx {
            cwd: PathBuf::from("/work"),
        }
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
        let first = sync(&ctx(), &view, &facts);
        assert!(has_component(&first, ReminderKind::Tasks));
        // The plan-changed source is gone: an active plan never writes a row.
        assert!(
            first
                .iter()
                .all(|reminder| reminder.kind() != ReminderKind::PlanChanged),
            "{first:?}"
        );
        view.kinds_after_compacted.insert(ReminderKind::Tasks);
        let second = sync(&ctx(), &view, &facts);
        assert!(!has_component(&second, ReminderKind::Tasks));
    }

    fn has_component(rows: &[Reminder], kind: ReminderKind) -> bool {
        rows.iter()
            .any(|reminder| reminder.components().iter().any(|c| c.kind() == kind))
    }

    #[test]
    fn a_post_compaction_restore_folds_into_one_row() {
        let facts = Facts {
            tasks: TaskFacts {
                todos: vec![TodoSnap {
                    id: "t1".into(),
                    content: "active".into(),
                    status: "in_progress".into(),
                    priority: None,
                }],
                ..TaskFacts::default()
            },
            memory: MemoryFacts {
                entries: vec![MemorySnap {
                    id: "m1".into(),
                    text: "remember this".into(),
                }],
            },
            background: BackgroundFacts {
                running_bash: vec![RunningBash {
                    job_id: "bg1".into(),
                    command: "sleep 1".into(),
                    output_file: ".litecode/run/bg1.stdout".into(),
                }],
                children_running: 0,
                children_idle: 0,
            },
            ..Facts::default()
        };
        let view = SpineReminderView {
            compacted_head: Some(4),
            ..SpineReminderView::default()
        };
        let out = sync(&ctx(), &view, &facts);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].kind(), ReminderKind::Restore);
        let kinds: Vec<ReminderKind> = out[0].components().iter().map(Reminder::kind).collect();
        assert_eq!(
            kinds,
            vec![
                ReminderKind::Env,
                ReminderKind::Tasks,
                ReminderKind::Memory,
                ReminderKind::Background,
            ]
        );
        assert!(out[0].text().contains("Environment:"), "{}", out[0].text());
        assert!(out[0].text().contains("remember this"), "{}", out[0].text());
        assert!(out[0].text().contains("bg1"), "{}", out[0].text());
    }

    #[test]
    fn a_lone_restore_reminder_does_not_fold() {
        let facts = Facts {
            tasks: TaskFacts {
                todos: vec![TodoSnap {
                    id: "t1".into(),
                    content: "active".into(),
                    status: "in_progress".into(),
                    priority: None,
                }],
                ..TaskFacts::default()
            },
            ..Facts::default()
        };
        // The env snapshot already sits on the spine, so only tasks fires here.
        let mut view = SpineReminderView {
            compacted_head: Some(4),
            ..SpineReminderView::default()
        };
        view.latest_by_kind.insert(
            ReminderKind::Env,
            Reminder::Env(EnvBody {
                cwd: ctx().cwd.display().to_string().replace('\\', "/"),
                os: std::env::consts::OS.to_string(),
                date: chrono::Local::now().format("%Y-%m-%d").to_string(),
                text: "prev".into(),
            }),
        );
        let out = sync(&ctx(), &view, &facts);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].kind(), ReminderKind::Tasks);
    }

    #[test]
    fn a_restore_bundle_keeps_every_component_kind_on_the_spine() {
        let bundle = Reminder::Restore(RestoreBody {
            parts: vec![
                Reminder::Env(EnvBody {
                    cwd: "/w".into(),
                    os: "linux".into(),
                    date: "2026-11-26".into(),
                    text: "env".into(),
                }),
                Reminder::Tasks(TasksBody {
                    todos: Vec::new(),
                    active_plan: None,
                    text: "tasks".into(),
                }),
            ],
            text: "env\n\ntasks".into(),
        });
        let view = view_from_spine(&[(1, SpineNode::Compacted), (2, SpineNode::Reminder(bundle))]);
        assert!(view.latest(ReminderKind::Env).is_some());
        assert!(view.kinds_after_compacted.contains(&ReminderKind::Tasks));
        assert!(!view.kinds_after_compacted.contains(&ReminderKind::Restore));
        assert!(view.latest(ReminderKind::Restore).is_none());
    }

    #[test]
    fn a_restore_bundle_roundtrips_json_and_stays_hidden() {
        let bundle = Reminder::Restore(RestoreBody {
            parts: vec![
                Reminder::Env(EnvBody {
                    cwd: "/w".into(),
                    os: "linux".into(),
                    date: "2026-11-26".into(),
                    text: "Environment: cwd /w; os linux; date 2026-11-26.".into(),
                }),
                Reminder::Tasks(TasksBody {
                    todos: vec![TodoSnap {
                        id: "t1".into(),
                        content: "active".into(),
                        status: "in_progress".into(),
                        priority: None,
                    }],
                    active_plan: None,
                    text: "Todos:\n[~] active".into(),
                }),
            ],
            text: "Environment: cwd /w; os linux; date 2026-11-26.\n\nTodos:\n[~] active".into(),
        });
        let raw = serde_json::to_string(&bundle).expect("serialize restore");
        let parsed: Reminder = serde_json::from_str(&raw).expect("deserialize restore");
        assert_eq!(parsed, bundle);
        assert_eq!(
            ReminderKind::parse_wire(parsed.kind().wire()),
            Some(ReminderKind::Restore)
        );
        assert!(crate::reminder::hidden_kind(parsed.kind().wire()));
    }
}
