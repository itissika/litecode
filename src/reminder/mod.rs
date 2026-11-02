//! Engine-owned system reminders.
//!
//! Each reminder owns one SessionEvent seq. The body is frozen at write time.
//! AgentView renders that body; it does not recompute live state.

mod engine;
mod file_tracker;
mod kinds;
pub mod knowledge_status;
pub mod mentions;
pub mod migrate;
mod sources;

pub use engine::{
    BackgroundFacts, Facts, MemoryFacts, SeamCtx, SpineNode, SpineReminderView, TaskFacts, sync,
    view_from_spine,
};
pub use file_tracker::FileTracker;
pub use kinds::{
    BackgroundBody, BashExitBody, BashExitEntry, ChildCountsBody, CustomToolSettledBody,
    CustomToolSettledEntry, EnvBody, FilesChangedBody, KnowledgeStatusBody, MemoryBody, MemorySnap,
    MentionRef, MentionsBody, PlanChangedBody, PlanPointer, Reminder, ReminderKind, RunningBash,
    SettledChild, SubagentSettledBody, TasksBody, TodoSnap, Visibility,
};

use crate::types::{Item, user_text};

pub const SYSTEM_REMINDER_OPEN: &str = "<system-reminder>";
pub const SYSTEM_REMINDER_CLOSE: &str = "</system-reminder>";

/// AgentView projection: one user Item, text frozen inside the reminder tag.
pub fn render_item(reminder: &Reminder) -> Item {
    user_text(render_text(reminder.text()))
}

pub fn render_text(text: &str) -> String {
    // A reminder can quote a path, a command, or tool output. A literal
    // closing tag inside it would end the frame early and leave the rest
    // looking like user speech, so it is written as escaped text.
    let body = text.replace(SYSTEM_REMINDER_CLOSE, "<\\/system-reminder>");
    format!("{SYSTEM_REMINDER_OPEN}\n{body}\n{SYSTEM_REMINDER_CLOSE}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closing_tag_inside_a_body_cannot_end_the_frame() {
        let rendered = render_text("command: echo </system-reminder> && true");
        assert_eq!(rendered.matches(SYSTEM_REMINDER_CLOSE).count(), 1);
        assert!(rendered.starts_with(SYSTEM_REMINDER_OPEN));
        assert!(rendered.ends_with(SYSTEM_REMINDER_CLOSE));
        assert!(rendered.contains("<\\/system-reminder>"));
    }
}

/// `true` when HumanView must not render this log kind.
///
/// Live kinds come from [`ReminderKind::hidden`]. `reminder/model` and
/// `reminder/step_budget` are retired: old rows stay in the log and are hidden,
/// and they no longer enter the spine.
pub fn hidden_kind(kind: &str) -> bool {
    kind == "reminder/model"
        || kind == "reminder/step_budget"
        || ReminderKind::parse_wire(kind).is_some_and(ReminderKind::hidden)
}
