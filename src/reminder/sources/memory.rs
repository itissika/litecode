use crate::reminder::engine::{MemoryFacts, SpineReminderView};
use crate::reminder::kinds::{MemoryBody, Reminder, ReminderKind};

/// Restore the current memory snapshot exactly once after a compaction.
pub(crate) fn memory(view: &SpineReminderView, facts: &MemoryFacts) -> Option<Reminder> {
    if view.compacted_head.is_none() {
        return None;
    }
    if view.kinds_after_compacted.contains(&ReminderKind::Memory) {
        return None;
    }
    if facts.entries.is_empty() {
        return None;
    }
    Some(Reminder::Memory(MemoryBody {
        entries: facts.entries.clone(),
        text: render(facts),
    }))
}

fn render(facts: &MemoryFacts) -> String {
    let mut out = String::from(
        "Session memory (survives compaction; update with memory.upsert, remove stale with memory.delete):",
    );
    for entry in &facts.entries {
        out.push_str(&format!("\n- {} ({})", entry.text, entry.id));
    }
    out
}
