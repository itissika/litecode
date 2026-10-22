//! Shared byte budget for console text and the final JSON value.

use super::{LogEntry, LogLevel};

#[derive(Debug)]
pub(crate) struct OutputLedger {
    limit: usize,
    used: usize,
    logs: Vec<LogEntry>,
    tripped: bool,
    dropped_undefined: super::boundary::DroppedUndefined,
}

impl OutputLedger {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            used: 0,
            logs: Vec::new(),
            tripped: false,
            dropped_undefined: Default::default(),
        }
    }

    pub(crate) fn tripped(&self) -> bool {
        self.tripped
    }

    pub(crate) fn set_dropped_undefined(&mut self, dropped: super::boundary::DroppedUndefined) {
        self.dropped_undefined = dropped;
    }

    pub(crate) fn dropped_undefined(&self) -> &super::boundary::DroppedUndefined {
        &self.dropped_undefined
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
        let prefix = crate::types::utf8::prefix_utf8(&message, room).to_string();
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

#[cfg(test)]
mod tests {
    

}
