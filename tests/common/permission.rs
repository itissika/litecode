use std::sync::Arc;

use litecode::permission::{AskOutcome, PermissionSink, RecordingPermissionSink};
use tokio_util::sync::CancellationToken;

/// Integration-test sink: auto-approves permission prompts (not used in production).
#[derive(Debug, Default, Clone, Copy)]
struct TestAutoApproveSink;

impl PermissionSink for TestAutoApproveSink {
    fn ask_permission(
        &self,
        _tool_name: &str,
        _rule_id: &str,
        _summary: &str,
        _cancel: &CancellationToken,
    ) -> AskOutcome {
        AskOutcome::Allow { always: false }
    }
}

pub fn test_auto_approve_sink() -> Arc<dyn PermissionSink> {
    Arc::new(TestAutoApproveSink)
}

pub fn recording_sink(response: (bool, bool)) -> RecordingPermissionSink {
    RecordingPermissionSink::from_reply(response.0, response.1)
}

#[test]
fn test_auto_approve_sink_approves_ask() {
    let sink = test_auto_approve_sink();
    assert_eq!(
        sink.ask_permission("write", "default", "file", &CancellationToken::new()),
        AskOutcome::Allow { always: false }
    );
}

#[test]
fn recording_sink_records_and_returns_response() {
    let sink = recording_sink((false, false));
    assert_eq!(
        sink.ask_permission("bash", "dangerous", "rm -rf", &CancellationToken::new()),
        AskOutcome::Deny
    );
    let calls = sink.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        ("bash".into(), "dangerous".into(), "rm -rf".into())
    );
}
