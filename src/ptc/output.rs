//! Shared byte budget for console text and the final JSON value.

use super::{LogEntry, LogLevel};

#[derive(Debug)]
pub(crate) struct OutputLedger {
    limit: usize,
    used: usize,
    logs: Vec<LogEntry>,
    tripped: bool,
}

impl OutputLedger {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            used: 0,
            logs: Vec::new(),
            tripped: false,
        }
    }

    pub(crate) fn tripped(&self) -> bool {
        self.tripped
    }

    pub(crate) fn logs(&self) -> &[LogEntry] {
        &self.logs
    }

    /// Append a log. When it does not fit, keep the UTF-8 prefix that does and
    /// trip the hard cap. Returns false once the cap has been hit.
    pub(crate) fn push(&mut self, level: LogLevel, message: String) -> bool {
        if self.tripped {
            return false;
        }
        let room = self.limit.saturating_sub(self.used);
        if message.len() <= room {
            self.used += message.len();
            self.logs.push(LogEntry { level, message });
            return true;
        }
        let prefix = prefix_utf8(&message, room).to_string();
        if !prefix.is_empty() {
            self.used += prefix.len();
            self.logs.push(LogEntry {
                level,
                message: prefix,
            });
        }
        self.tripped = true;
        false
    }

    pub(crate) fn try_value(&mut self, bytes: usize) -> bool {
        if self.used.saturating_add(bytes) > self.limit {
            self.tripped = true;
            false
        } else {
            self.used += bytes;
            true
        }
    }
}

pub(crate) fn prefix_utf8(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_keeps_a_char_boundary() {
        let text = "aé";
        assert_eq!(prefix_utf8(text, 0), "");
        assert_eq!(prefix_utf8(text, 1), "a");
        assert_eq!(prefix_utf8(text, 2), "a");
        assert_eq!(prefix_utf8(text, 3), "aé");
    }

    #[test]
    fn push_keeps_the_fitting_prefix() {
        let mut ledger = OutputLedger::new(12);
        assert!(ledger.push(LogLevel::Log, "abcdefghij".into()));
        assert!(!ledger.push(LogLevel::Log, "XYZ".into()));
        assert!(ledger.tripped());
        assert_eq!(ledger.logs()[0].message, "abcdefghij");
        assert_eq!(ledger.logs()[1].message, "XY");
        assert!(!ledger.try_value(1));
    }
}
