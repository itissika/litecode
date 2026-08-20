//! Persist working set: log `seq` plus Responses [`Item`] payload.
//!
//! `Item[]` remains the LLM / tool projection. Identity for the write gate is `seq`.

use crate::types::Item;

use super::event::Seq;

/// One model-visible row in the persist working set.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkingRow {
    pub seq: Option<Seq>,
    pub item: Item,
}

impl WorkingRow {
    pub fn pending(item: Item) -> Self {
        Self { seq: None, item }
    }

    pub fn persisted(seq: Seq, item: Item) -> Self {
        Self {
            seq: Some(seq),
            item,
        }
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
        } else {
            rows.push(WorkingRow::pending(item.clone()));
        }
    }
}
