use std::sync::{Arc, LazyLock, Mutex};

use tokio_util::sync::CancellationToken;

use super::PermissionSink;
use super::grants::AskOutcome;

/// Headless runtime sink: denies if Ask is ever reached (subagent uses static config only).
#[derive(Debug, Default, Clone, Copy)]
pub struct DenyPermissionSink;

impl PermissionSink for DenyPermissionSink {
    fn ask_permission(
        &self,
        _tool_name: &str,
        _rule_id: &str,
        _summary: &str,
        _cancel: &CancellationToken,
    ) -> AskOutcome {
        AskOutcome::Deny
    }
}

pub fn deny_permission_sink() -> Arc<dyn PermissionSink> {
    static SINK: LazyLock<Arc<dyn PermissionSink>> = LazyLock::new(|| Arc::new(DenyPermissionSink));
    Arc::clone(&SINK)
}

/// Test sink: records prompts and returns a configured response.
#[derive(Debug, Default)]
pub struct RecordingPermissionSink {
    pub calls: Arc<Mutex<Vec<(String, String, String)>>>,
    pub response: AskOutcome,
}

impl RecordingPermissionSink {
    pub fn new(response: AskOutcome) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            response,
        }
    }

    pub fn from_reply(approved: bool, always: bool) -> Self {
        Self::new(AskOutcome::from_reply(approved, always))
    }
}

impl PermissionSink for RecordingPermissionSink {
    fn ask_permission(
        &self,
        tool_name: &str,
        rule_id: &str,
        summary: &str,
        _cancel: &CancellationToken,
    ) -> AskOutcome {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push((
                tool_name.to_string(),
                rule_id.to_string(),
                summary.to_string(),
            ));
        }
        self.response
    }
}

/// Wraps a permission sink so turn cancellation short-circuits blocking waits.
pub struct CancellingPermissionSink {
    inner: Arc<dyn PermissionSink>,
    cancel: CancellationToken,
}

impl CancellingPermissionSink {
    pub fn new(inner: Arc<dyn PermissionSink>, cancel: CancellationToken) -> Self {
        Self { inner, cancel }
    }
}

impl PermissionSink for CancellingPermissionSink {
    fn ask_permission(
        &self,
        tool_name: &str,
        rule_id: &str,
        summary: &str,
        cancel: &CancellationToken,
    ) -> AskOutcome {
        let cancel = if cancel.is_cancelled() || self.cancel.is_cancelled() {
            &self.cancel
        } else {
            cancel
        };
        if self.cancel.is_cancelled() || cancel.is_cancelled() {
            return AskOutcome::Aborted;
        }
        let result = self
            .inner
            .ask_permission(tool_name, rule_id, summary, &self.cancel);
        if self.cancel.is_cancelled() || result == AskOutcome::Aborted {
            AskOutcome::Aborted
        } else {
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::AskOutcome;

    #[test]
    fn cancelling_sink_returns_aborted_not_deny() {
        let inner = Arc::new(DenyPermissionSink);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let sink = CancellingPermissionSink::new(inner, cancel.clone());
        assert_eq!(
            sink.ask_permission("bash", "default", "ls", &cancel),
            AskOutcome::Aborted
        );
    }
}
