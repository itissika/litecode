use std::sync::{Arc, LazyLock, Mutex};

use tokio_util::sync::CancellationToken;

use super::PermissionSink;
use super::grants::{AskAnswer, AskOutcome, AskPrompt, AskReply};
use std::collections::HashMap;

/// Headless runtime sink: denies if Ask is ever reached (subagent uses static config only).
#[derive(Debug, Default, Clone, Copy)]
pub struct DenyPermissionSink;

impl PermissionSink for DenyPermissionSink {
    fn ask(&self, _prompt: &AskPrompt<'_>, _cancel: &CancellationToken) -> AskReply {
        AskReply::from_outcome(AskOutcome::Deny)
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
    pub free_text: Option<String>,
    pub selected: Vec<String>,
    pub answers: HashMap<String, AskAnswer>,
}

impl RecordingPermissionSink {
    pub fn new(response: AskOutcome) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            response,
            free_text: None,
            selected: Vec::new(),
            answers: HashMap::new(),
        }
    }

    pub fn from_reply(approved: bool, always: bool) -> Self {
        Self::new(AskOutcome::from_reply(approved, always))
    }

    pub fn with_free_text(mut self, text: impl Into<String>) -> Self {
        self.free_text = Some(text.into());
        self
    }

    pub fn with_selected(mut self, selected: Vec<String>) -> Self {
        self.selected = selected;
        self
    }

    pub fn with_answers(mut self, answers: HashMap<String, AskAnswer>) -> Self {
        self.answers = answers;
        self
    }
}

impl PermissionSink for RecordingPermissionSink {
    fn ask(&self, prompt: &AskPrompt<'_>, _cancel: &CancellationToken) -> AskReply {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push((
                prompt.tool.to_string(),
                prompt.rule_id.to_string(),
                prompt.summary.to_string(),
            ));
        }
        let mut answers = self.answers.clone();
        if answers.is_empty() && !self.selected.is_empty() {
            answers.insert(
                "q0".into(),
                AskAnswer {
                    selected: self.selected.clone(),
                    free_text: self.free_text.clone(),
                },
            );
        }
        AskReply {
            outcome: self.response,
            free_text: self.free_text.clone(),
            selected: self.selected.clone(),
            answers,
        }
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
    fn ask(&self, prompt: &AskPrompt<'_>, cancel: &CancellationToken) -> AskReply {
        let cancel = if cancel.is_cancelled() || self.cancel.is_cancelled() {
            &self.cancel
        } else {
            cancel
        };
        if self.cancel.is_cancelled() || cancel.is_cancelled() {
            return AskReply::from_outcome(AskOutcome::Aborted);
        }
        let reply = self.inner.ask(prompt, &self.cancel);
        if self.cancel.is_cancelled() || reply.outcome == AskOutcome::Aborted {
            AskReply::from_outcome(AskOutcome::Aborted)
        } else {
            reply
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::{AskOutcome, AskPrompt};

    #[test]
    fn cancelling_sink_returns_aborted_not_deny() {
        let inner = Arc::new(DenyPermissionSink);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let sink = CancellingPermissionSink::new(inner, cancel.clone());
        let reply = sink.ask(
            &AskPrompt::permission("bash", "default", "ls"),
            &cancel,
        );
        assert_eq!(reply.outcome, AskOutcome::Aborted);
    }
}
