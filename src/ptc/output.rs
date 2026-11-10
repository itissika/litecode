//! One ordered stream for everything a program produced, under one byte cap.
//!
//! Console text and the program's return value are entries of the same stream,
//! in the order they happened; the return value is simply the last entry. The
//! ledger keeps one byte budget for all of it, so there is no "logs half" and
//! "result half" to reason about.

use super::boundary::DroppedUndefined;
use super::json::Json;
use super::{Entry, EntryBody, LogLevel};

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

    pub(crate) fn set_dropped_undefined(&mut self, dropped: DroppedUndefined) {
        self.dropped_undefined = dropped;
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

    /// Append one console line. When it does not fit, keep the UTF-8 prefix that
    /// does and trip the hard cap. Returns false once the cap has been hit.
    pub(crate) fn push_text(&mut self, level: LogLevel, message: String) -> bool {
        if self.tripped {
            return false;
        }
        let room = self.limit.saturating_sub(self.used);
        if message.len() <= room {
            self.used += message.len();
            let order = self.next_order();
            self.entries.push(Entry {
                order,
                body: EntryBody::Text {
                    level,
                    text: message,
                },
            });
            return true;
        }
        let prefix = crate::types::utf8::prefix_utf8(&message, room).to_string();
        if !prefix.is_empty() {
            self.used += prefix.len();
            let order = self.next_order();
            self.entries.push(Entry {
                order,
                body: EntryBody::Text {
                    level,
                    text: prefix,
                },
            });
        }
        self.tripped = true;
        false
    }

    /// Append the program's return value. It is the last entry of the stream.
    pub(crate) fn push_value(&mut self, value: Json) -> bool {
        let bytes = match super::json_bytes(value.as_value()) {
            Ok(bytes) => bytes.len(),
            Err(_) => return false,
        };
        if self.used.saturating_add(bytes) > self.limit {
            self.tripped = true;
            return false;
        }
        self.used += bytes;
        let order = self.next_order();
        self.entries.push(Entry {
            order,
            body: EntryBody::Value(value),
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_value_share_one_stream_in_order() {
        let mut ledger = OutputLedger::new(1024);
        assert!(ledger.push_text(LogLevel::Log, "first".into()));
        assert!(ledger.push_value(Json::new(serde_json::json!({ "ok": true }))));
        assert!(ledger.push_text(LogLevel::Warn, "last".into()));
        let orders: Vec<u64> = ledger.entries().iter().map(|entry| entry.order).collect();
        assert_eq!(orders, vec![1, 2, 3]);
    }

    #[test]
    fn push_keeps_the_fitting_prefix() {
        let mut ledger = OutputLedger::new(12);
        assert!(ledger.push_text(LogLevel::Log, "abcdefghij".into()));
        assert!(!ledger.push_text(LogLevel::Log, "XYZ".into()));
        assert!(ledger.tripped());
        match &ledger.entries()[0].body {
            EntryBody::Text { text, .. } => assert_eq!(text, "abcdefghij"),
            other => panic!("expected text, got {other:?}"),
        }
        match &ledger.entries()[1].body {
            EntryBody::Text { text, .. } => assert_eq!(text, "XY"),
            other => panic!("expected text, got {other:?}"),
        }
        assert!(!ledger.push_value(Json::new(serde_json::json!(1))));
    }
}
