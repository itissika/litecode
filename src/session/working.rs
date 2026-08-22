//! Transitional GateRow implementation.
//!
//! Identity for the write gate is `log_seq`; the legacy `item` cache exists
//! only while runtime producers are migrated to kind-specific bodies.

use crate::types::Item;

use super::event::Seq;
use super::model::SessionKind;

/// One model-visible row in the persist working set.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkingRow {
    /// `None` means this row has not crossed the write gate.
    pub log_seq: Option<Seq>,
    /// Product kind is explicit; never infer it from JSON during persistence.
    pub kind: SessionKind,
    /// Compatibility cache for the item/* producer migration.
    pub item: Item,
}

impl WorkingRow {
    pub fn pending(item: Item) -> Self {
        Self {
            log_seq: None,
            kind: kind_for_item(&item),
            item,
        }
    }

    pub fn persisted(seq: Seq, item: Item) -> Self {
        Self {
            log_seq: Some(seq),
            kind: kind_for_item(&item),
            item,
        }
    }
}

pub type GateRow = WorkingRow;

fn kind_for_item(item: &Item) -> SessionKind {
    match item {
        Item::FunctionCall(_) => SessionKind::ItemToolCall,
        Item::FunctionCallOutput(_) => SessionKind::ItemToolResult,
        Item::Message(crate::authority::responses::MessageItem::Input(input))
            if matches!(input.role, crate::authority::responses::InputRole::User) =>
        {
            SessionKind::ItemUser
        }
        _ => SessionKind::ItemAssistant,
    }
}

/// Project payload items for the agent loop and LLM view.
pub fn project_items(rows: &[WorkingRow]) -> Vec<Item> {
    rows.iter().map(|row| row.item.clone()).collect()
}

/// Keep prefix seqs, refresh payloads, treat extra items as unpersisted.
pub fn align_working(rows: &mut Vec<WorkingRow>, items: &[Item]) {
    rows.truncate(items.len());
    for (i, item) in items.iter().enumerate() {
        if i < rows.len() {
            rows[i].item = item.clone();
            rows[i].kind = kind_for_item(item);
        } else {
            rows.push(WorkingRow::pending(item.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::user_text;

    #[test]
    fn align_working_keeps_kind_in_sync_with_item() {
        let mut rows = vec![WorkingRow::pending(user_text("a"))];
        assert_eq!(rows[0].kind, SessionKind::ItemUser);
        let asst = crate::types::assistant_text("b");
        align_working(&mut rows, &[asst]);
        assert_eq!(rows[0].kind, SessionKind::ItemAssistant);
    }
}
