//! Idle auto-turn when a session async custom-tool job exits and a UI is attached.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::permission::PermissionSink;
use crate::runtime::{RuntimeHandle, TurnOptions, spawn_turn};
use crate::session::{LifecycleEvent, SessionManager};
use crate::tools::custom_hub::CustomToolHub;
use crate::types::LitecodeError;

pub enum IdleAutoTurn {
    Prepared {
        session_id: String,
        turn_id: String,
        primary_agent: String,
        project: String,
        sink: Arc<dyn PermissionSink>,
    },
    SkippedBusy,
    SkippedNoUi,
    SkippedEmptyMailbox,
    SkippedSessionGone,
}

/// Decide whether an idle live session should start a turn for custom-tool exits.
/// On `Prepared`, the turn is reserved. The mailbox stays pending so the first
/// request seam can write `reminder/custom_tool_settled`.
pub fn try_begin_idle_auto_turn(
    hub: &CustomToolHub,
    runtime: &RuntimeHandle,
    sessions: &SessionManager,
    workspace_root: &Path,
    session_id: &str,
) -> IdleAutoTurn {
    let sid = session_id;
    if sid.is_empty() || sid == "_" {
        return IdleAutoTurn::SkippedNoUi;
    }
    if !hub.mailbox_pending(sid) {
        return IdleAutoTurn::SkippedEmptyMailbox;
    }
    if sessions.subscriber_count_blocking(sid) == 0 {
        return IdleAutoTurn::SkippedNoUi;
    }
    if sessions.is_session_busy_blocking(sid) {
        return IdleAutoTurn::SkippedBusy;
    }

    let default_primary = runtime.desired_primary_agent();
    let primary_agent =
        match sessions.resolve_primary_agent(sid, default_primary, &runtime.resolved) {
            Ok(id) => id,
            Err(_) => return IdleAutoTurn::SkippedSessionGone,
        };
    let project = sessions
        .project(sid)
        .unwrap_or_else(|| workspace_root.display().to_string());
    let step_max = crate::config::bridge::agent_config_for(&runtime.resolved, &primary_agent)
        .map(|a| a.max_steps)
        .unwrap_or(50);
    let turn_id = uuid::Uuid::new_v4().to_string();
    match sessions.reserve_turn(sid, turn_id.clone(), step_max, &primary_agent, &project) {
        Ok(_) => {}
        Err(LitecodeError::AgentAlreadyRunning) => return IdleAutoTurn::SkippedBusy,
        Err(_) => return IdleAutoTurn::SkippedSessionGone,
    }

    if !hub.mailbox_pending(sid) {
        sessions.release_turn_reservation(sid, &turn_id);
        return IdleAutoTurn::SkippedEmptyMailbox;
    }
    let sink = crate::runtime::auto_turn_sink::auto_turn_permission_sink(sessions, sid);
    IdleAutoTurn::Prepared {
        session_id: sid.to_string(),
        turn_id,
        primary_agent,
        project,
        sink,
    }
}

fn spawn_prepared_idle_auto_turn(
    runtime: &RuntimeHandle,
    sessions: &Arc<SessionManager>,
    decision: IdleAutoTurn,
) {
    let IdleAutoTurn::Prepared {
        session_id,
        turn_id,
        primary_agent,
        project,
        sink,
    } = decision
    else {
        return;
    };
    let sessions = Arc::clone(sessions);
    let handle = match spawn_turn(
        runtime,
        session_id.clone(),
        Arc::clone(&sessions),
        crate::runtime::TurnInput::Wake {
            anchor_user_seq: None,
        },
        sink,
        turn_id.clone(),
        TurnOptions::default(),
    ) {
        Ok(h) => h,
        Err(error) => {
            tracing::warn!(
                session_id = %session_id,
                error = %error,
                "custom-tool idle auto-turn spawn failed"
            );
            sessions.release_turn_reservation(&session_id, &turn_id);
            return;
        }
    };
    let session_id_err = session_id.clone();
    let turn_id_err = turn_id.clone();
    let sessions_err = Arc::clone(&sessions);
    let spawn_result = std::thread::Builder::new()
        .name(format!("custom-idle-{session_id}"))
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(error) => {
                    tracing::error!(
                        session_id = %session_id,
                        error = %error,
                        "custom-tool idle auto-turn runtime failed"
                    );
                    sessions.release_turn_reservation(&session_id, &turn_id);
                    return;
                }
            };
            if let Err(error) = rt.block_on(sessions.start_turn(
                &session_id,
                handle,
                &primary_agent,
                &project,
                Arc::clone(&sessions),
            )) {
                tracing::error!(
                    session_id = %session_id,
                    error = %error,
                    "custom-tool idle auto-turn start failed"
                );
                sessions.release_turn_reservation(&session_id, &turn_id);
            }
        });
    if let Err(error) = spawn_result {
        tracing::error!(
            session_id = %session_id_err,
            error = %error,
            "failed to spawn custom-tool idle auto-turn thread"
        );
        sessions_err.release_turn_reservation(&session_id_err, &turn_id_err);
    }
}

fn maybe_spawn_idle_auto_turn(
    hub: &CustomToolHub,
    runtime: &RwLock<RuntimeHandle>,
    sessions: &Arc<SessionManager>,
    workspace_root: &Path,
    session_id: &str,
) {
    let runtime_snap = runtime.read().expect("runtime lock").clone();
    let decision =
        try_begin_idle_auto_turn(hub, &runtime_snap, sessions, workspace_root, session_id);
    spawn_prepared_idle_auto_turn(&runtime_snap, sessions, decision);
}

pub fn install_custom_auto_turn(
    hub: Arc<CustomToolHub>,
    runtime: Arc<RwLock<RuntimeHandle>>,
    sessions: Arc<SessionManager>,
    workspace_root: PathBuf,
) {
    let hub_for_exit = Arc::clone(&hub);
    let runtime_for_exit = Arc::clone(&runtime);
    let sessions_for_exit = Arc::clone(&sessions);
    let root_for_exit = workspace_root.clone();
    hub.set_exit_handler(Arc::new(move |notice| {
        maybe_spawn_idle_auto_turn(
            &hub_for_exit,
            &runtime_for_exit,
            &sessions_for_exit,
            &root_for_exit,
            &notice.session_id,
        );
    }));

    let hub_for_life = Arc::clone(&hub);
    let runtime_for_life = Arc::clone(&runtime);
    let sessions_for_life = Arc::clone(&sessions);
    let mut rx = sessions.subscribe_lifecycle();
    if let Err(error) = std::thread::Builder::new()
        .name("custom-idle-turn-flush".into())
        .spawn(move || {
            loop {
                match rx.blocking_recv() {
                    Ok(LifecycleEvent::TurnFinished { session_id, .. }) => {
                        maybe_spawn_idle_auto_turn(
                            &hub_for_life,
                            &runtime_for_life,
                            &sessions_for_life,
                            &workspace_root,
                            &session_id,
                        );
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    {
        tracing::error!(
            error = %error,
            "failed to spawn custom-tool idle-turn-flush thread"
        );
    }
}

#[cfg(test)]
mod tests {
    
    
    
    
    
    
    
    
    
    

}
