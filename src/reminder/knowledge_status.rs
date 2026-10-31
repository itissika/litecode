//! Knowledge-base class attached after a human message.
//!
//! Libraries with no enabled node produce nothing. The same class still on
//! the live surface is not written again.

use std::path::Path;

use super::kinds::{KnowledgeStatusBody, Reminder};

pub fn build(workspace: &Path) -> Option<Reminder> {
    let note = crate::knowledge::status::note(workspace)?;
    Some(Reminder::KnowledgeStatus(KnowledgeStatusBody {
        class: note.class.as_str().to_string(),
        text: note.text,
    }))
}
