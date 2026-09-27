//! Engine-owned system reminders.
//!
//! Each reminder owns one SessionLog seq. The body is frozen at write time.
//! AgentView renders that body; it does not recompute live state.

mod engine;
mod file_tracker;
mod kinds;
pub mod migrate;
mod sources;

pub use engine::{
    BackgroundFacts, Facts, SeamCtx, SpineNode, SpineReminderView, TaskFacts, sync, view_from_spine,
};
pub use file_tracker::FileTracker;
pub use kinds::{
    BackgroundBody, BashExitBody, BashExitEntry, ChildCountsBody, EnvBody, FilesChangedBody,
    PlanChangedBody, PlanPointer, Reminder, ReminderKind, RunningBash, SettledChild,
    StepBudgetBody, SubagentSettledBody, TasksBody, TodoSnap, Visibility,
};

use crate::types::{Item, user_text};

pub const SYSTEM_REMINDER_OPEN: &str = "<system-reminder>";
pub const SYSTEM_REMINDER_CLOSE: &str = "</system-reminder>";

/// AgentView projection: one user Item, text frozen inside the reminder tag.
pub fn render_item(reminder: &Reminder) -> Item {
    user_text(render_text(reminder.text()))
}

pub fn render_text(text: &str) -> String {
    format!("{SYSTEM_REMINDER_OPEN}\n{text}\n{SYSTEM_REMINDER_CLOSE}")
}

/// `true` when HumanView must not render this log kind.
///
/// Live kinds come from [`ReminderKind::hidden`]. `reminder/model` is retired:
/// old rows stay in the log and are hidden, and they no longer enter the spine.
pub fn hidden_kind(kind: &str) -> bool {
    kind == "reminder/model" || ReminderKind::parse_wire(kind).is_some_and(ReminderKind::hidden)
}
