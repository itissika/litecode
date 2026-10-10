use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::action::PermissionAction;

/// Result of a user-facing permission prompt.
///
/// Deny continues the agent loop with a permission-denied tool result.
/// Aborted interrupts the wait (turn cancel) and must not be mapped to Deny.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AskOutcome {
    Allow {
        always: bool,
    },
    #[default]
    Deny,
    Aborted,
}

impl AskOutcome {
    pub fn from_reply(approved: bool, always: bool) -> Self {
        if approved {
            Self::Allow { always }
        } else {
            Self::Deny
        }
    }
}

/// Kind of human Ask surfaced through the permission bridge.
///
/// `Permission` — Allow once / Always allow / Deny (tool grant).
/// `Approval` — Approve / Reject + optional free-text (plan create).
/// `AskUser` — pick from `options` (+ optional free-text); not a permission grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskKind {
    #[default]
    Permission,
    Approval,
    AskUser,
}

/// One choice on an `ask_user` card (`id` is what the agent receives).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskOption {
    pub id: String,
    pub label: String,
}

/// One question in an `ask_user` batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskQuestion {
    pub id: String,
    pub prompt: String,
    pub options: Vec<AskOption>,
    #[serde(default)]
    pub multi_select: bool,
    /// When true, show an optional free-text field for this question.
    #[serde(default)]
    pub free_text: bool,
}

/// Per-question answer on an `ask_user` submit (`answers` map value).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AskAnswer {
    #[serde(default)]
    pub selected: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free_text: Option<String>,
}

fn trim_opt(s: Option<String>) -> Option<String> {
    s.and_then(|s| {
        let t = s.trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    })
}

/// Parameters for a single human Ask.
#[derive(Debug, Clone, Copy)]
pub struct AskPrompt<'a> {
    pub tool: &'a str,
    pub rule_id: &'a str,
    /// Optional card title; for legacy single-question ask_user may equal the prompt.
    pub summary: &'a str,
    pub kind: AskKind,
    /// Approval: show free-text. Legacy ask_user (no `questions`): top-level free_text flag.
    pub free_text: bool,
    /// Legacy flat options (compat → single question id `q0` when `questions` empty).
    pub options: &'a [AskOption],
    pub multi_select: bool,
    /// Batch questions for `ask_user` (≥1). Empty for permission/approval.
    pub questions: &'a [AskQuestion],
}

impl<'a> AskPrompt<'a> {
    pub fn permission(tool: &'a str, rule_id: &'a str, summary: &'a str) -> Self {
        Self {
            tool,
            rule_id,
            summary,
            kind: AskKind::Permission,
            free_text: false,
            options: &[],
            multi_select: false,
            questions: &[],
        }
    }

    pub fn approval(tool: &'a str, rule_id: &'a str, summary: &'a str) -> Self {
        Self {
            tool,
            rule_id,
            summary,
            kind: AskKind::Approval,
            free_text: true,
            options: &[],
            multi_select: false,
            questions: &[],
        }
    }

    pub fn ask_user(
        tool: &'a str,
        rule_id: &'a str,
        summary: &'a str,
        questions: &'a [AskQuestion],
    ) -> Self {
        Self {
            tool,
            rule_id,
            summary,
            kind: AskKind::AskUser,
            free_text: false,
            options: &[],
            multi_select: false,
            questions,
        }
    }

    /// Wire/FE view: prefer `questions`; else lift legacy summary/options into `q0`.
    pub fn effective_questions(&self) -> Vec<AskQuestion> {
        if !self.questions.is_empty() {
            return self.questions.to_vec();
        }
        if self.kind == AskKind::AskUser && !self.options.is_empty() {
            return vec![AskQuestion {
                id: "q0".into(),
                prompt: self.summary.to_string(),
                options: self.options.to_vec(),
                multi_select: self.multi_select,
                free_text: self.free_text,
            }];
        }
        Vec::new()
    }
}

/// Reply from a human Ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskReply {
    pub outcome: AskOutcome,
    /// Approval (and legacy) top-level free text.
    pub free_text: Option<String>,
    /// Legacy flat selected option ids (prefer `answers` for ask_user).
    pub selected: Vec<String>,
    /// ask_user submit: map question_id → answer.
    pub answers: HashMap<String, AskAnswer>,
}

impl AskReply {
    pub fn from_outcome(outcome: AskOutcome) -> Self {
        Self {
            outcome,
            free_text: None,
            selected: Vec::new(),
            answers: HashMap::new(),
        }
    }

    pub fn from_grant(
        approved: bool,
        always: bool,
        free_text: Option<String>,
        selected: Vec<String>,
        answers: HashMap<String, AskAnswer>,
    ) -> Self {
        let free_text = trim_opt(free_text);
        let mut answers = answers;
        for ans in answers.values_mut() {
            ans.free_text = trim_opt(ans.free_text.take());
        }
        // Legacy: flat selected only → synthesize q0 answer.
        if answers.is_empty() && !selected.is_empty() {
            answers.insert(
                "q0".into(),
                AskAnswer {
                    selected: selected.clone(),
                    free_text: free_text.clone(),
                },
            );
        }
        Self {
            outcome: AskOutcome::from_reply(approved, always),
            free_text,
            selected,
            answers,
        }
    }
}

/// User-facing permission prompt sink (CLI, WebSocket bridge, tests).
pub trait PermissionSink: Send + Sync {
    fn ask_permission(
        &self,
        tool_name: &str,
        rule_id: &str,
        summary: &str,
        cancel: &CancellationToken,
    ) -> AskOutcome {
        self.ask(&AskPrompt::permission(tool_name, rule_id, summary), cancel)
            .outcome
    }

    /// Richer Ask that can carry prompt kind + optional free-text opinion.
    /// Default maps to [`ask_permission`]-style permission with no free text
    /// when an impl only overrides `ask`. Prefer overriding `ask`.
    fn ask(&self, prompt: &AskPrompt<'_>, cancel: &CancellationToken) -> AskReply;
}

/// Wait for a oneshot reply without panicking on a tokio worker thread.
pub fn blocking_wait_oneshot<T: Send + 'static>(
    rx: tokio::sync::oneshot::Receiver<T>,
) -> Option<T> {
    blocking_wait_oneshot_cancellable(rx, None)
}

/// Wait for a oneshot reply, returning None when the turn is cancelled.
pub fn blocking_wait_oneshot_cancellable<T: Send + 'static>(
    mut rx: tokio::sync::oneshot::Receiver<T>,
    cancel: Option<CancellationToken>,
) -> Option<T> {
    if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return None;
    }
    loop {
        if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return None;
        }
        match rx.try_recv() {
            Ok(value) => return Some(value),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => return None,
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// How long an `always` grant stays valid before it must be re-approved.
const GRANT_TTL: Duration = Duration::from_secs(60 * 60);
/// Upper bound on per-agent runtime grants to prevent unbounded growth.
const MAX_GRANTS_PER_AGENT: usize = 100;

#[derive(Debug, Clone)]
struct RuntimeGrant {
    tool: String,
    rule_id: String,
    action: PermissionAction,
    expires_at: std::time::Instant,
}

static RUNTIME_GRANTS: std::sync::OnceLock<Mutex<HashMap<String, Vec<RuntimeGrant>>>> =
    std::sync::OnceLock::new();

fn runtime_grants() -> &'static Mutex<HashMap<String, Vec<RuntimeGrant>>> {
    RUNTIME_GRANTS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn grant_runtime(agent: &str, tool_name: &str, rule_id: &str, action: PermissionAction) {
    if let Ok(mut grants) = runtime_grants().lock() {
        let agent_grants = grants.entry(agent.to_string()).or_default();
        // Enforce an upper bound so `always` grants cannot grow unboundedly.
        if agent_grants.len() >= MAX_GRANTS_PER_AGENT {
            return;
        }
        agent_grants.push(RuntimeGrant {
            tool: tool_name.to_string(),
            rule_id: rule_id.to_string(),
            action,
            expires_at: std::time::Instant::now() + GRANT_TTL,
        });
    }
}

/// Lookup a stored grant. Callers must only apply this when evaluate returned `Ask`.
pub fn check_runtime_grant(
    agent: &str,
    tool_name: &str,
    rule_id: &str,
) -> Option<PermissionAction> {
    if let Ok(grants) = runtime_grants().lock()
        && let Some(agent_grants) = grants.get(agent)
    {
        for grant in agent_grants.iter().rev() {
            if grant.expires_at <= std::time::Instant::now() {
                continue; // expired
            }
            if grant.tool == tool_name && grant.rule_id == rule_id {
                return Some(grant.action);
            }
        }
    }
    None
}

/// Reset session-level runtime grants (integration tests).
pub fn clear_runtime_grants() {
    if let Ok(mut grants) = runtime_grants().lock() {
        grants.clear();
    }
}

/// Reset runtime grants for a single agent (integration tests).
pub fn clear_runtime_grants_for(agent: &str) {
    if let Ok(mut grants) = runtime_grants().lock() {
        grants.remove(agent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_has_upper_bound_per_agent() {
        clear_runtime_grants();
        for i in 0..(MAX_GRANTS_PER_AGENT + 50) {
            grant_runtime(
                "agent",
                &format!("tool_{i}"),
                &format!("rule_{i}"),
                PermissionAction::Allow,
            );
        }
        let n = {
            let grants = runtime_grants().lock().unwrap();
            grants.get("agent").map(Vec::len).unwrap_or(0)
        };
        assert_eq!(
            n, MAX_GRANTS_PER_AGENT,
            "per-agent grants must be capped at {MAX_GRANTS_PER_AGENT}"
        );
        clear_runtime_grants();
    }

    #[test]
    fn expired_grant_is_skipped() {
        // Distinct agent so it does not race the cap test in parallel.
        let agent = "expired_agent";
        clear_runtime_grants_for(agent);
        // A grant that already expired must not satisfy a lookup.
        if let Ok(mut grants) = runtime_grants().lock() {
            grants.entry(agent.into()).or_default().push(RuntimeGrant {
                tool: "write".into(),
                rule_id: "r".into(),
                action: PermissionAction::Allow,
                expires_at: std::time::Instant::now() - Duration::from_secs(1),
            });
        }
        assert_eq!(check_runtime_grant(agent, "write", "r"), None);
        clear_runtime_grants_for(agent);
    }

    #[test]
    fn cancellable_wait_returns_none_when_cancelled() {
        let (tx, rx) = tokio::sync::oneshot::channel::<u8>();
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(blocking_wait_oneshot_cancellable(rx, Some(cancel)).is_none());
        drop(tx);
    }
}
