//! One ordered stream of `inspect(...)` values, under one byte cap.

use super::Entry;
use super::boundary::DroppedUndefined;
use super::json::Json;

#[derive(Debug)]
pub(crate) struct OutputLedger {
    limit: usize,
    used: usize,
    entries: Vec<Entry>,
    order: u64,
    tripped: bool,
    dropped_undefined: DroppedUndefined,
}

impl OutputLedger {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            used: 0,
            entries: Vec::new(),
            order: 0,
            tripped: false,
            dropped_undefined: Default::default(),
        }
    }

    pub(crate) fn tripped(&self) -> bool {
        self.tripped
    }

    pub(crate) fn merge_dropped_undefined(&mut self, dropped: DroppedUndefined) {
        self.dropped_undefined.merge(dropped);
    }

    pub(crate) fn dropped_undefined(&self) -> &DroppedUndefined {
        &self.dropped_undefined
    }

    pub(crate) fn entries(&self) -> &[Entry] {
        &self.entries
    }

    fn next_order(&mut self) -> u64 {
        self.order += 1;
        self.order
    }

    /// Append one `inspect(...values)` call. Returns false once the hard byte
    /// cap is exceeded.
    pub(crate) fn push_values(&mut self, values: Vec<Json>) -> bool {
        if self.tripped || values.is_empty() {
            return !self.tripped;
        }
        let mut bytes = 0usize;
        for value in &values {
            let Ok(encoded) = super::json_bytes(value.as_value()) else {
                return false;
            };
            bytes = bytes.saturating_add(encoded.len());
        }
        if self.used.saturating_add(bytes) > self.limit {
            self.tripped = true;
            return false;
        }
        self.used += bytes;
        let order = self.next_order();
        self.entries.push(Entry { order, values });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspect_entries_stay_ordered_under_one_cap() {
        let mut ledger = OutputLedger::new(1024);
        assert!(ledger.push_values(vec![Json::new(serde_json::json!("first"))]));
        assert!(ledger.push_values(vec![
            Json::new(serde_json::json!("label")),
            Json::new(serde_json::json!({ "ok": true })),
        ]));
        let orders: Vec<u64> = ledger.entries().iter().map(|entry| entry.order).collect();
        assert_eq!(orders, vec![1, 2]);
        assert_eq!(ledger.entries()[1].values.len(), 2);
    }

    #[test]
    fn push_values_trips_cap_and_keeps_earlier_entries() {
        let mut ledger = OutputLedger::new(10);
        // `"abcd"` encodes to 6 bytes
        assert!(ledger.push_values(vec![Json::new(serde_json::json!("abcd"))]));
        // `"efghi"` encodes to 7 bytes -> 13 > 10
        assert!(!ledger.push_values(vec![Json::new(serde_json::json!("efghi"))]));
        assert!(ledger.tripped());
        assert_eq!(ledger.entries().len(), 1);
        assert_eq!(
            ledger.entries()[0].values[0].as_value(),
            &serde_json::json!("abcd")
        );
        // Once tripped, even a tiny value is rejected.
        assert!(!ledger.push_values(vec![Json::new(serde_json::json!(1))]));
    }
}
