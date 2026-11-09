//! Product-layer permission policy for bot sessions.
//!
//! A bot is an unattended product identity: an approval card would wait for a
//! human who may not be there. Bot sessions therefore deny Permission and
//! Approval prompts immediately (the loop gets an ordinary denied tool result).
//! ask_user stays answerable from the foreground and waits like any other
//! question. This is a sink policy, not a
//! per-tool permission matrix; the core engine and its Allow/Deny/Ask
//! evaluation are unchanged.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::permission::{AskKind, AskOutcome, AskPrompt, AskReply, PermissionSink};

#[derive(Clone)]
pub struct BotPermissionSink {
    inner: Arc<dyn PermissionSink>,
}

impl BotPermissionSink {
    pub fn new(inner: Arc<dyn PermissionSink>) -> Self {
        Self { inner }
    }
}

impl PermissionSink for BotPermissionSink {
    fn ask(&self, prompt: &AskPrompt<'_>, cancel: &CancellationToken) -> AskReply {
        if cancel.is_cancelled() {
            return AskReply::from_outcome(AskOutcome::Aborted);
        }
        if prompt.kind != AskKind::AskUser {
            // Permission/Approval cards are not shown to an unattended bot.
            return AskReply::from_outcome(AskOutcome::Deny);
        }
        self.inner.ask(prompt, cancel)
    }
}

/// Wrap a bot session's sink. The caller installs this inside the phase sink so
/// a timed-out ask still closes the turn phase as ExecutingTools, not Cancelling.
pub fn bot_permission_sink(inner: Arc<dyn PermissionSink>) -> Arc<dyn PermissionSink> {
    Arc::new(BotPermissionSink::new(inner))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::{AskAnswer, AskOption, AskQuestion, RecordingPermissionSink};
    use std::collections::HashMap;

    fn question() -> AskQuestion {
        AskQuestion {
            id: "q".into(),
            prompt: "pick".into(),
            options: vec![AskOption {
                id: "a".into(),
                label: "A".into(),
            }],
            multi_select: false,
            free_text: true,
        }
    }

    #[test]
    fn permission_and_approval_are_denied_without_reaching_the_ui() {
        let inner = Arc::new(RecordingPermissionSink::new(AskOutcome::Allow {
            always: false,
        }));
        let sink = BotPermissionSink::new(inner.clone());
        let cancel = CancellationToken::new();
        let permission = AskPrompt::permission("run", "r", "rm");
        assert_eq!(sink.ask(&permission, &cancel).outcome, AskOutcome::Deny);
        let approval = AskPrompt::approval("plan", "create", "plan?");
        assert_eq!(sink.ask(&approval, &cancel).outcome, AskOutcome::Deny);
        assert!(inner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn ask_user_is_delegated_to_the_foreground_sink() {
        let mut answers = HashMap::new();
        answers.insert(
            "q".into(),
            AskAnswer {
                selected: vec!["a".into()],
                free_text: None,
            },
        );
        let mut recording = RecordingPermissionSink::new(AskOutcome::Allow { always: false });
        recording.answers = answers;
        let inner = Arc::new(recording);
        let sink = BotPermissionSink::new(Arc::clone(&inner) as Arc<dyn PermissionSink>);
        let cancel = CancellationToken::new();
        let questions = [question()];
        let prompt = AskPrompt::ask_user("ask_user", "ask", "", &questions);
        let reply = sink.ask(&prompt, &cancel);
        assert!(matches!(reply.outcome, AskOutcome::Allow { .. }));
        assert_eq!(reply.answers["q"].selected, vec!["a"]);
        assert_eq!(inner.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn stale_frontend_answer_is_not_rewritten_by_the_bot_sink() {
        let inner = Arc::new(RecordingPermissionSink::new(AskOutcome::Aborted));
        let sink = BotPermissionSink::new(Arc::clone(&inner) as Arc<dyn PermissionSink>);
        let cancel = CancellationToken::new();
        let questions = [question()];
        let prompt = AskPrompt::ask_user("ask_user", "ask", "", &questions);
        let reply = sink.ask(&prompt, &cancel);
        assert_eq!(reply.outcome, AskOutcome::Aborted);
        assert_eq!(inner.calls.lock().unwrap().len(), 1);
    }
}
