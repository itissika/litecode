//! Job registry handle. Watches the same session event stream a client uses.
//!
//! The hub does not start, finish, or join turns. Live `TurnCompleted` is the
//! same fact the UI sees; if that event is missed, it hydrates `turn/end` from
//! the session log. Missing durable truth settles as `unknown`, never as a
//! synthesized failure.

use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

use crate::authority::responses::MessageItem;
use crate::runtime::observer::{InternalEnvelope, InternalEvent};
use crate::session::event::EventType;
use crate::session::live::LifecycleEvent;
use crate::session::manager::SessionManager;
use crate::types::{Item, item_text_preview};

use super::jobs::SubagentJobBoard;

pub struct SubagentHub {
    pub jobs: Arc<SubagentJobBoard>,
    sessions: Mutex<Option<Arc<SessionManager>>>,
}

fn spawn_session_removed_gc(jobs: Arc<SubagentJobBoard>, sessions: Arc<SessionManager>) {
    let mut rx = sessions.subscribe_lifecycle();
    let _ = std::thread::Builder::new()
        .name("subagent-hub-gc".into())
        .spawn(move || loop {
            match rx.blocking_recv() {
                Ok(LifecycleEvent::SessionRemoved { session_id }) => {
                    jobs.purge_parent(&session_id);
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        });
}

impl Default for SubagentHub {
    fn default() -> Self {
        Self::new()
    }
}

impl SubagentHub {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(SubagentJobBoard::new()),
            sessions: Mutex::new(None),
        }
    }

    pub fn attach_sessions(&self, sessions: Arc<SessionManager>) {
        let mut slot = self.sessions.lock().expect("hub sessions lock");
        let first = slot.is_none();
        *slot = Some(Arc::clone(&sessions));
        drop(slot);
        if first {
            spawn_session_removed_gc(Arc::clone(&self.jobs), sessions);
        }
    }

    pub fn set_exit_handler(
        &self,
        handler: Arc<dyn Fn(super::jobs::ExitNotice) + Send + Sync>,
    ) {
        self.jobs.set_exit_handler(handler);
    }

    pub fn set_jobs_changed_handler(&self, handler: Arc<dyn Fn(String) + Send + Sync>) {
        self.jobs.set_jobs_changed_handler(handler);
    }

    pub fn purge_parent(&self, parent_session_id: &str) {
        self.jobs.purge_parent(parent_session_id);
    }

    pub fn forget_child(&self, child_id: &str) {
        self.jobs.forget_child(child_id);
    }

    pub fn wire_snapshot(&self, session_id: &str) -> super::jobs::SubagentJobsSnapshot {
        self.jobs.wire_snapshot(session_id)
    }

    /// Watch a child session event receiver (subscribed before `start_turn`)
    /// for `TurnCompleted`. Never joins the turn thread. `call_id` scopes the
    /// watcher to one registered job so a later send cannot steal the first exit.
    pub fn watch_child_exit(
        &self,
        child_id: &str,
        call_id: &str,
        rx: broadcast::Receiver<InternalEnvelope>,
    ) {
        let jobs = Arc::clone(&self.jobs);
        let sessions = self
            .sessions
            .lock()
            .expect("hub sessions lock")
            .clone();
        let child = child_id.to_string();
        let call = call_id.to_string();
        if let Err(error) = std::thread::Builder::new()
            .name(format!("subagent-watch-{child}"))
            .spawn({
                let jobs = Arc::clone(&jobs);
                let sessions = sessions.clone();
                let child = child.clone();
                let call = call.clone();
                move || watch_loop(jobs, sessions, child, call, rx)
            })
        {
            tracing::error!(
                error = %error,
                child_session_id = %child,
                "named subagent watcher spawn failed; polling durable turn/end"
            );
            spawn_fallback_observer(jobs, sessions, child, call);
        }
    }
}

/// Last-resort closer when the named watcher thread cannot start. Must not
/// leave the job `alive` with nobody watching it.
fn spawn_fallback_observer(
    jobs: Arc<SubagentJobBoard>,
    sessions: Option<Arc<SessionManager>>,
    child_id: String,
    call_id: String,
) {
    if std::thread::Builder::new()
        .spawn({
            let jobs = Arc::clone(&jobs);
            let sessions = sessions.clone();
            let child_id = child_id.clone();
            let call_id = call_id.clone();
            move || poll_idle_then_hydrate(jobs, sessions, child_id, call_id)
        })
        .is_ok()
    {
        return;
    }
    tracing::error!(
        child_session_id = %child_id,
        "unnamed subagent watcher spawn failed"
    );
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn_blocking(move || {
            poll_idle_then_hydrate(jobs, sessions, child_id, call_id);
        });
        return;
    }
    settle_from_durable(&jobs, sessions.as_ref(), &child_id, &call_id);
}

fn watch_loop(
    jobs: Arc<SubagentJobBoard>,
    sessions: Option<Arc<SessionManager>>,
    child_id: String,
    call_id: String,
    mut rx: broadcast::Receiver<InternalEnvelope>,
) {
    loop {
        match rx.try_recv() {
            Ok(envelope) => {
                if settle_if_completed(&jobs, &child_id, &call_id, &envelope) {
                    return;
                }
                continue;
            }
            Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(broadcast::error::TryRecvError::Closed) => {
                settle_from_durable(&jobs, sessions.as_ref(), &child_id, &call_id);
                return;
            }
            Err(broadcast::error::TryRecvError::Empty) => {}
        }
        if !jobs.is_alive(&child_id) {
            return;
        }
        let idle = sessions
            .as_ref()
            .is_some_and(|manager| !manager.is_turn_running_blocking(&child_id));
        if idle {
            loop {
                match rx.try_recv() {
                    Ok(envelope) => {
                        if settle_if_completed(&jobs, &child_id, &call_id, &envelope) {
                            return;
                        }
                    }
                    Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                    Err(broadcast::error::TryRecvError::Empty)
                    | Err(broadcast::error::TryRecvError::Closed) => break,
                }
            }
            settle_from_durable(&jobs, sessions.as_ref(), &child_id, &call_id);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn poll_idle_then_hydrate(
    jobs: Arc<SubagentJobBoard>,
    sessions: Option<Arc<SessionManager>>,
    child_id: String,
    call_id: String,
) {
    loop {
        if !jobs.is_alive(&child_id) {
            return;
        }
        let idle = sessions
            .as_ref()
            .is_some_and(|manager| !manager.is_turn_running_blocking(&child_id));
        if idle {
            settle_from_durable(&jobs, sessions.as_ref(), &child_id, &call_id);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn settle_if_completed(
    jobs: &SubagentJobBoard,
    child_id: &str,
    call_id: &str,
    envelope: &InternalEnvelope,
) -> bool {
    let InternalEvent::TurnCompleted {
        reason, final_text, ..
    } = &envelope.event
    else {
        return false;
    };
    jobs.finish_for_call(
        child_id,
        call_id,
        reason.as_log_reason(),
        final_text.clone().unwrap_or_default(),
    );
    true
}

fn settle_from_durable(
    jobs: &SubagentJobBoard,
    sessions: Option<&Arc<SessionManager>>,
    child_id: &str,
    call_id: &str,
) {
    let Some(sessions) = sessions else {
        jobs.finish_for_call(child_id, call_id, "unknown", String::new());
        return;
    };
    let reason = last_turn_end_reason(sessions, child_id).unwrap_or("unknown");
    let text = last_assistant_text(sessions, child_id);
    jobs.finish_for_call(child_id, call_id, reason, text);
}

fn last_turn_end_reason(sessions: &SessionManager, child_id: &str) -> Option<&'static str> {
    let events = sessions.data().events_blocking(child_id).ok()?;
    let reason = events
        .iter()
        .rev()
        .find(|event| event.event_type == EventType::TurnEnd)?
        .data
        .get("reason")?
        .as_str()?;
    match reason {
        "completed" => Some("completed"),
        "cancelled" => Some("cancelled"),
        "error" => Some("error"),
        "max_steps" => Some("max_steps"),
        _ => None,
    }
}

fn last_assistant_text(sessions: &SessionManager, child_id: &str) -> String {
    let Ok(items) = sessions.data().transcript_blocking(child_id) else {
        return String::new();
    };
    items
        .iter()
        .rev()
        .find(|item| matches!(item, Item::Message(MessageItem::Output(_))))
        .map(item_text_preview)
        .unwrap_or_default()
}
