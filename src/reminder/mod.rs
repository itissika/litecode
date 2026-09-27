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
    BackgroundFacts, Facts, SeamCtx, SpineNode, SpineReminderView, TaskFacts, sync,
    view_from_spine,
};
pub use file_tracker::FileTracker;
pub use kinds::{
    BackgroundBody, BashExitBody, BashExitEntry, ChildCountsBody, EnvBody, FilesChangedBody,
    ModelBody, PlanChangedBody, PlanPointer, Reminder, ReminderKind, RunningBash, SettledChild,
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

/// `true` when this log kind must not be rendered in HumanView.
pub fn hidden_kind(kind: &str) -> bool {
    ReminderKind::parse_wire(kind).is_some_and(ReminderKind::hidden)
}
