//! Knowledge-base class attached after a human message.
//!
//! Empty libraries produce nothing. The same class still on the live surface
//! is not written again.

use std::path::Path;

use super::kinds::{KnowledgeStatusBody, Reminder};

pub fn build(workspace: &Path) -> Option<Reminder> {
    let note = crate::knowledge::status::note(workspace)?;
    Some(Reminder::KnowledgeStatus(KnowledgeStatusBody {
        class: note.class.as_str().to_string(),
        text: note.text,
    }))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use crate::config::TurnGuard;
    use crate::reminder::{ReminderKind, hidden_kind};
    use crate::session::manager::SessionManager;

    #[test]
    fn the_status_row_is_hidden_and_repeats_only_when_the_class_changes() {
        assert!(hidden_kind("reminder/knowledge_status"));
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("ws");
        fs::create_dir_all(workspace.join(".litecode").join("knowledge")).unwrap();
        let db = dir.path().join("sessions.db");
        let mgr = SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            db.to_str().unwrap().to_string(),
        );
        let sid = mgr
            .open_session_sync(workspace.to_str().unwrap(), "default", None)
            .unwrap();

        mgr.append_user_message_with_mentions(&sid, "hello", &workspace)
            .unwrap();
        let kinds = reminder_kinds(&mgr, &sid);
        assert!(
            !kinds.contains(&ReminderKind::KnowledgeStatus),
            "an empty library writes nothing"
        );

        fs::write(
            workspace.join(".litecode/knowledge/seq.md"),
            "```node\nnode : seq\nstatus : enabled\nsummary : seq\n```\n",
        )
        .unwrap();
        mgr.append_user_message_with_mentions(&sid, "again", &workspace)
            .unwrap();
        mgr.append_user_message_with_mentions(&sid, "same", &workspace)
            .unwrap();
        let notes: Vec<_> = reminder_kinds(&mgr, &sid)
            .into_iter()
            .filter(|kind| *kind == ReminderKind::KnowledgeStatus)
            .collect();
        assert_eq!(notes.len(), 1);

        fs::write(
            workspace.join(".litecode/knowledge/draft.md"),
            "```node\nnode : draft\nstatus : pending\nsummary : draft\n```\n",
        )
        .unwrap();
        mgr.append_user_message_with_mentions(&sid, "changed", &workspace)
            .unwrap();
        let notes: Vec<_> = reminder_kinds(&mgr, &sid)
            .into_iter()
            .filter(|kind| *kind == ReminderKind::KnowledgeStatus)
            .collect();
        assert_eq!(notes.len(), 2);
    }

    fn reminder_kinds(mgr: &SessionManager, sid: &str) -> Vec<ReminderKind> {
        mgr.data()
            .events_blocking(sid)
            .unwrap()
            .into_iter()
            .filter_map(|event| match event.event_type {
                crate::session::EventType::Reminder(kind) => Some(kind),
                _ => None,
            })
            .collect()
    }
}
