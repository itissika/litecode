//! Live LLM transport reconnect notices.
//!
//! The codec emits these while it is still inside one model call. The runtime
//! installs a task-local sink for that call; tests and headless providers that
//! never enter the scope simply drop the notice.

use std::future::Future;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmReconnectPhase {
    Waiting,
    Connecting,
    Cleared,
    Failed,
}

/// One reconnect transition. `attempt` is 1-based. `delay_ms` is set only while
/// waiting out the backoff before the next try.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmReconnect {
    pub phase: LlmReconnectPhase,
    pub attempt: u32,
    pub max_attempts: u32,
    #[serde(default, skip_serializing_if = "u64_is_zero")]
    pub delay_ms: u64,
}

fn u64_is_zero(value: &u64) -> bool {
    *value == 0
}

impl LlmReconnect {
    /// Snapshot / retry authority. A clear is an absence, not a stored phase.
    pub fn retained(self) -> Option<Self> {
        match self.phase {
            LlmReconnectPhase::Cleared => None,
            _ => Some(self),
        }
    }

    pub fn is_failed(self) -> bool {
        self.phase == LlmReconnectPhase::Failed
    }
}

tokio::task_local! {
    static TRANSPORT_SINK: Arc<dyn Fn(LlmReconnect) + Send + Sync>;
}

pub fn emit(notice: LlmReconnect) {
    let _ = TRANSPORT_SINK.try_with(|sink| sink(notice));
}

pub async fn scope<F>(sink: Arc<dyn Fn(LlmReconnect) + Send + Sync>, fut: F) -> F::Output
where
    F: Future,
{
    TRANSPORT_SINK.scope(sink, fut).await
}

/// Compaction shares transport retries, but a terminal failure is not an agent
/// turn. `Failed` is forwarded as `Cleared` so `agent/retry` cannot wake one.
pub async fn scope_compaction<F>(fut: F) -> F::Output
where
    F: Future,
{
    let outer = TRANSPORT_SINK.try_with(|sink| Arc::clone(sink)).ok();
    let sink: Arc<dyn Fn(LlmReconnect) + Send + Sync> = Arc::new(move |notice| {
        let Some(outer) = &outer else {
            return;
        };
        if notice.is_failed() {
            outer(LlmReconnect {
                phase: LlmReconnectPhase::Cleared,
                attempt: notice.attempt,
                max_attempts: notice.max_attempts,
                delay_ms: 0,
            });
        } else {
            outer(notice);
        }
    });
    scope(sink, fut).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[tokio::test]
    async fn compaction_scope_turns_a_terminal_failure_into_cleared() {
        let notices = Arc::new(Mutex::new(Vec::new()));
        let sink_notices = Arc::clone(&notices);
        let sink: Arc<dyn Fn(LlmReconnect) + Send + Sync> =
            Arc::new(move |notice| sink_notices.lock().unwrap().push(notice));
        scope(sink, async {
            scope_compaction(async {
                emit(LlmReconnect {
                    phase: LlmReconnectPhase::Waiting,
                    attempt: 1,
                    max_attempts: 6,
                    delay_ms: 500,
                });
                emit(LlmReconnect {
                    phase: LlmReconnectPhase::Failed,
                    attempt: 6,
                    max_attempts: 6,
                    delay_ms: 0,
                });
            })
            .await;
        })
        .await;
        let seen = notices.lock().unwrap().clone();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].phase, LlmReconnectPhase::Waiting);
        assert_eq!(seen[1].phase, LlmReconnectPhase::Cleared);
        assert_eq!(seen[1].attempt, 6);
    }
}
