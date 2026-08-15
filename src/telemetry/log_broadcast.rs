use std::fmt::Write;

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

use crate::client_protocol::protocol::LogLine;

use super::publish_log;

pub fn log_broadcast_layer() -> LogBroadcastLayer {
    LogBroadcastLayer
}

pub struct LogBroadcastLayer;

impl<S> Layer<S> for LogBroadcastLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if super::log_sender().receiver_count() == 0 {
            return;
        }

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let metadata = event.metadata();
        publish_log(LogLine {
            ts_ms: chrono::Utc::now().timestamp_millis(),
            level: level_label(metadata.level()).to_string(),
            target: metadata.target().to_string(),
            message: visitor.finish(),
        });
    }
}

fn level_label(level: &Level) -> &'static str {
    match *level {
        Level::TRACE => "TRACE",
        Level::DEBUG => "DEBUG",
        Level::INFO => "INFO",
        Level::WARN => "WARN",
        Level::ERROR => "ERROR",
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
    fields: String,
}

impl MessageVisitor {
    fn finish(self) -> String {
        match self.message {
            Some(message) if self.fields.is_empty() => message,
            Some(message) => format!("{message} {}", self.fields.trim()),
            None if !self.fields.is_empty() => self.fields.trim().to_string(),
            None => String::new(),
        }
    }
}

impl Visit for MessageVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            let _ = write!(self.fields, "{}={} ", field.name(), value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}").trim_matches('"').to_string());
        } else {
            let _ = write!(self.fields, "{}={value:?} ", field.name());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_protocol::protocol::LogLine;

    #[test]
    fn publish_log_delivers_to_subscriber() {
        let mut rx = super::super::subscribe_logs();
        while rx.try_recv().is_ok() {}

        super::super::publish_log(LogLine {
            ts_ms: 1,
            level: "INFO".into(),
            target: "test".into(),
            message: "hello broadcast".into(),
        });

        let line = rx.try_recv().expect("expected log line");
        assert_eq!(line.message, "hello broadcast");
    }

    #[test]
    fn message_visitor_finish_paths() {
        let only_message = MessageVisitor {
            message: Some("hello".into()),
            fields: String::new(),
        };
        assert_eq!(only_message.finish(), "hello");

        let message_and_fields = MessageVisitor {
            message: Some("hello".into()),
            fields: "tool=bash ".into(),
        };
        assert_eq!(message_and_fields.finish(), "hello tool=bash");

        let fields_only = MessageVisitor {
            message: None,
            fields: "tool=bash ".into(),
        };
        assert_eq!(fields_only.finish(), "tool=bash");
    }
}
