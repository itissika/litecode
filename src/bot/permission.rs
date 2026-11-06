//! Product-layer permission policy for bot sessions.
//!
//! A bot is an unattended product identity: an approval card would wait for a
//! human who may not be there. Bot sessions therefore deny Permission and
//! Approval prompts immediately (the loop gets an ordinary denied tool result),
//! while ask_user stays answerable from the foreground for a bounded 60s.
//! This is a sink policy, not a per-tool permission matrix; the core engine and
//! its Allow/Deny/Ask evaluation are unchanged.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::permission::{
    AskKind, AskOption, AskOutcome, AskPrompt, AskQuestion, AskReply, PermissionSink,
};

/// A bot's foreground ask_user window. The turn continues on expiry.
pub const BOT_ASK_USER_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct BotPermissionSink {
    inner: Arc<dyn PermissionSink>,
    ask_user_timeout: Duration,
}

impl BotPermissionSink {
    pub fn new(inner: Arc<dyn PermissionSink>) -> Self {
        Self {
            inner,
            ask_user_timeout: BOT_ASK_USER_TIMEOUT,
        }
    }

    /// Test/embedding seam; production uses [BOT_ASK_USER_TIMEOUT].
    pub fn with_timeout(inner: Arc<dyn PermissionSink>, ask_user_timeout: Duration) -> Self {
        Self {
            inner,
            ask_user_timeout,
        }
    }
}

fn no_answer() -> AskReply {
    let mut reply = AskReply::from_outcome(AskOutcome::Deny);
    reply.timed_out = true;
    reply
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

        let owned = OwnedPrompt::from(prompt);
        let inner = Arc::clone(&self.inner);
        let local_cancel = cancel.child_token();
        let thread_cancel = local_cancel.clone();
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("bot-ask-user".into())
            .spawn(move || {
                let reply = inner.ask(&owned.as_prompt(), &thread_cancel);
                let _ = tx.send(reply);
            });
        if thread.is_err() {
            local_cancel.cancel();
            return no_answer();
        }

        let deadline = Instant::now() + self.ask_user_timeout;
        loop {
            if cancel.is_cancelled() {
                local_cancel.cancel();
                return AskReply::from_outcome(AskOutcome::Aborted);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                local_cancel.cancel();
                return no_answer();
            }
            match rx.recv_timeout(left.min(Duration::from_millis(25))) {
                Ok(reply) => {
                    if cancel.is_cancelled() {
                        local_cancel.cancel();
                        return AskReply::from_outcome(AskOutcome::Aborted);
                    }
                    // A stale/dead WS route answers Aborted without a human.
                    // For a bot that is "no answer", not turn cancellation.
                    if reply.outcome == AskOutcome::Aborted {
                        return no_answer();
                    }
                    return reply;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return no_answer(),
            }
        }
    }
}

/// Owned copy for the timeout worker, which outlives the borrowed prompt.
struct OwnedPrompt {
    tool: String,
    rule_id: String,
    summary: String,
    kind: AskKind,
    free_text: bool,
    options: Vec<AskOption>,
    multi_select: bool,
    questions: Vec<AskQuestion>,
}

impl From<&AskPrompt<'_>> for OwnedPrompt {
    fn from(prompt: &AskPrompt<'_>) -> Self {
        Self {
            tool: prompt.tool.to_string(),
            rule_id: prompt.rule_id.to_string(),
            summary: prompt.summary.to_string(),
            kind: prompt.kind,
            free_text: prompt.free_text,
            options: prompt.options.to_vec(),
            multi_select: prompt.multi_select,
            questions: prompt.questions.to_vec(),
        }
    }
}

impl OwnedPrompt {
    fn as_prompt(&self) -> AskPrompt<'_> {
        AskPrompt {
            tool: &self.tool,
            rule_id: &self.rule_id,
            summary: &self.summary,
            kind: self.kind,
            free_text: self.free_text,
            options: &self.options,
            multi_select: self.multi_select,
            questions: &self.questions,
        }
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
    use crate::permission::{AskAnswer, RecordingPermissionSink};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct NeverAnswer {
        cancelled: Arc<AtomicBool>,
    }

    impl PermissionSink for NeverAnswer {
        fn ask(&self, _prompt: &AskPrompt<'_>, cancel: &CancellationToken) -> AskReply {
            while !cancel.is_cancelled() {
                std::thread::sleep(Duration::from_millis(10));
            }
            self.cancelled.store(true, Ordering::Release);
            AskReply::from_outcome(AskOutcome::Aborted)
        }
    }

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
        let inner = Arc::new(RecordingPermissionSink::new(AskOutcome::Allow { always: false }));
        let sink = BotPermissionSink::new(inner.clone());
        let cancel = CancellationToken::new();
        let permission = AskPrompt::permission("run", "r", "rm");
        assert_eq!(sink.ask(&permission, &cancel).outcome, AskOutcome::Deny);
        let approval = AskPrompt::approval("plan", "create", "plan?");
        assert_eq!(sink.ask(&approval, &cancel).outcome, AskOutcome::Deny);
        assert!(inner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn ask_user_times_out_as_no_answer_and_continues() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let inner = Arc::new(NeverAnswer {
            cancelled: Arc::clone(&cancelled),
        });
        let sink = BotPermissionSink::with_timeout(inner, Duration::from_millis(30));
        let cancel = CancellationToken::new();
        let questions = [question()];
        let prompt = AskPrompt::ask_user("ask_user", "ask", "", &questions);
        let reply = sink.ask(&prompt, &cancel);
        assert!(reply.timed_out);
        assert_eq!(reply.outcome, AskOutcome::Deny);
        for _ in 0..100 {
            if cancelled.load(Ordering::Acquire) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(cancelled.load(Ordering::Acquire));
    }

    #[test]
    fn foreground_answer_wins_before_the_deadline() {
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
        let sink = BotPermissionSink::with_timeout(
            Arc::new(recording),
            BOT_ASK_USER_TIMEOUT,
        );
        let cancel = CancellationToken::new();
        let questions = [question()];
        let prompt = AskPrompt::ask_user("ask_user", "ask", "", &questions);
        let reply = sink.ask(&prompt, &cancel);
        assert!(!reply.timed_out);
        assert!(matches!(reply.outcome, AskOutcome::Allow { .. }));
        assert_eq!(reply.answers["q"].selected, vec!["a"]);
    }
}
