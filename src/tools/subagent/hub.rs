//! Completion routing client for child Session turns.

use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

use crate::session::live::LifecycleEvent;
use crate::session::manager::SessionManager;

use super::jobs::{CompletionInbox, CompletionRef};

type ExitHandler = Arc<dyn Fn(CompletionRef) + Send + Sync>;

pub struct SubagentHub {
    pub completions: Arc<CompletionInbox>,
    sessions: Mutex<Option<Arc<SessionManager>>>,
    exit_handler: Arc<Mutex<Option<ExitHandler>>>,
}

impl Default for SubagentHub {
    fn default() -> Self {
        Self::new()
    }
}

impl SubagentHub {
    pub fn new() -> Self {
        Self {
            completions: Arc::new(CompletionInbox::default()),
            sessions: Mutex::new(None),
            exit_handler: Arc::new(Mutex::new(None)),
        }
    }

    pub fn attach_sessions(&self, sessions: Arc<SessionManager>) {
        let mut slot = self.sessions.lock().expect("hub sessions lock");
        let first = slot.is_none();
        *slot = Some(Arc::clone(&sessions));
        drop(slot);
        if first {
            spawn_completion_router(
                Arc::clone(&self.completions),
                sessions,
                Arc::clone(&self.exit_handler),
            );
        }
    }

    pub fn set_exit_handler(&self, handler: ExitHandler) {
        *self.exit_handler.lock().expect("exit handler lock") = Some(handler);
    }

    pub fn purge_parent(&self, parent_session_id: &str) {
        self.completions.purge_parent(parent_session_id);
    }

    pub fn forget_child(&self, child_id: &str) {
        self.completions.forget_child(child_id);
    }

    pub fn has_pending(&self, parent_session_id: &str) -> bool {
        self.completions.pending(parent_session_id)
    }

    pub fn take_completions(&self, parent_session_id: &str) -> Vec<CompletionRef> {
        self.completions.take_all(parent_session_id)
    }

    pub fn restore_completions(&self, parent_session_id: &str, completions: Vec<CompletionRef>) {
        self.completions
            .restore_front(parent_session_id, completions);
    }
}

fn spawn_completion_router(
    completions: Arc<CompletionInbox>,
    sessions: Arc<SessionManager>,
    exit_handler: Arc<Mutex<Option<ExitHandler>>>,
) {
    let mut rx = sessions.subscribe_lifecycle();
    if let Err(error) = std::thread::Builder::new()
        .name("subagent-completion-router".into())
        .spawn(move || {
            loop {
                match rx.blocking_recv() {
                    Ok(LifecycleEvent::TurnFinished {
                        session_id,
                        progress,
                        ..
                    }) => {
                        let Ok(meta) = sessions.reader().meta_blocking(&session_id) else {
                            continue;
                        };
                        let Some(parent_session_id) = meta.parent_session_id else {
                            continue;
                        };
                        let completion = CompletionRef {
                            parent_session_id,
                            child_session_id: session_id,
                            turn_id: progress.turn_id,
                        };
                        completions.push(completion.clone());
                        if let Some(handler) =
                            exit_handler.lock().expect("exit handler lock").clone()
                        {
                            handler(completion);
                        }
                    }
                    Ok(LifecycleEvent::SessionRemoved { session_id }) => {
                        completions.purge_parent(&session_id);
                        completions.forget_child(&session_id);
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "subagent completion router lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    {
        tracing::error!(%error, "failed to spawn subagent completion router");
    }
}
