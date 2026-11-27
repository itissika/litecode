//! Shared scaffolding for the two idle auto-turn listeners.
//!
//! `bash_auto_turn` wakes a session when a background job exits;
//! `subagent_auto_turn` wakes it when a child settles. Both ask the same
//! questions in the same order — mailbox empty? viewer attached? session busy?
//! — reserve the turn, hand the spawn the turn's permission sink, and release
//! the reservation on every failure path.
//!
//! What they do *not* share is the mailbox: each listener probes its own hub and
//! neither drains it. Delivery belongs to the request seam
//! (`reminder/bash_exit`, `reminder/subagent_settled`).
//!
//! `pending_auto_turn` deliberately stays separate: it has no viewer gate and
//! wakes with `TurnInput::Wake { anchor_user_seq }`, because a queued user
//! message must be delivered even when nobody is watching.

use std::path::Path;
use std::sync::{Arc, RwLock};

use crate::permission::PermissionSink;
use crate::session::{LifecycleEvent, SessionService};
use crate::types::LitecodeError;

use super::{RuntimeHandle, TurnOptions};

/// The one decision both listeners make, before it is mapped back onto their
/// own public enum.
pub(crate) enum IdleTurn {
    Prepared(PreparedIdleTurn),
    SkippedBusy,
    SkippedNoUi,
    SkippedEmptyMailbox,
    SkippedSessionGone,
}

pub(crate) struct PreparedIdleTurn {
    pub session_id: String,
    pub turn_id: String,
    pub primary_agent: String,
    pub project: String,
    pub sink: Arc<dyn PermissionSink>,
}

/// Decide whether an idle, watched session should start a turn.
///
/// Order matters and is part of the contract: mailbox → viewer gate → busy. A
/// session with nothing to report never reserves a turn, and a busy one leaves
/// its mailbox untouched. On `Prepared` the turn is reserved and the mailbox is
/// still pending, so the first request seam writes the reminder.
pub(crate) fn plan_idle_turn(
    runtime: &RuntimeHandle,
    service: &SessionService,
    workspace_root: &Path,
    session_id: &str,
    mailbox_pending: impl Fn() -> bool,
) -> IdleTurn {
    let sid = session_id;
    if sid.is_empty() || sid == "_" {
        return IdleTurn::SkippedNoUi;
    }
    if !mailbox_pending() {
        return IdleTurn::SkippedEmptyMailbox;
    }
    if service.subscriber_count_blocking(sid) == 0 {
        return IdleTurn::SkippedNoUi;
    }
    if service.is_session_busy_blocking(sid) {
        return IdleTurn::SkippedBusy;
    }

    let default_primary = runtime.desired_primary_agent();
    let primary_agent = match service.resolve_primary_agent_for_session(runtime, sid, default_primary)
    {
        Ok(id) => id,
        Err(_) => return IdleTurn::SkippedSessionGone,
    };
    let project = service
        .project(sid)
        .unwrap_or_else(|| workspace_root.display().to_string());
    let step_max = crate::agent::AGENT_MAX_STEPS;
    let turn_id = uuid::Uuid::new_v4().to_string();
    match service.reserve_auto_turn(sid, turn_id.clone(), step_max, &primary_agent, &project) {
        Ok(_) => {}
        Err(LitecodeError::AgentAlreadyRunning) => return IdleTurn::SkippedBusy,
        Err(_) => return IdleTurn::SkippedSessionGone,
    }

    // A seam may have drained the mailbox between the two probes: give the
    // reservation back instead of waking for nothing.
    if !mailbox_pending() {
        service.release_auto_turn(sid, &turn_id);
        return IdleTurn::SkippedEmptyMailbox;
    }
    let sink = service.auto_turn_permission_sink(sid);
    IdleTurn::Prepared(PreparedIdleTurn {
        session_id: sid.to_string(),
        turn_id,
        primary_agent,
        project,
        sink,
    })
}

/// Spawn the prepared turn on its own thread and release the reservation on
/// every failure path, so a session is never left permanently busy.
pub(crate) fn spawn_prepared(
    runtime: &RuntimeHandle,
    service: &Arc<SessionService>,
    prepared: PreparedIdleTurn,
    label: &'static str,
) {
    let PreparedIdleTurn {
        session_id,
        turn_id,
        primary_agent,
        project,
        sink,
    } = prepared;
    let service = Arc::clone(service);
    let handle = match service.spawn_auto_turn(
        runtime,
        session_id.clone(),
        super::TurnInput::Wake {
            anchor_user_seq: None,
        },
        sink,
        turn_id.clone(),
        TurnOptions::default(),
    ) {
        Ok(handle) => handle,
        Err(error) => {
            tracing::warn!(%session_id, %error, label = label, "idle auto-turn spawn failed");
            service.release_auto_turn(&session_id, &turn_id);
            return;
        }
    };
    let thread_service = Arc::clone(&service);
    let thread_session = session_id.clone();
    let thread_turn = turn_id.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("{label}-idle-{session_id}"))
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(error) => {
                    tracing::warn!(
                        session_id = %thread_session,
                        %error,
                        label = label,
                        "idle auto-turn runtime failed"
                    );
                    thread_service.release_auto_turn(&thread_session, &thread_turn);
                    return;
                }
            };
            if let Err(error) = rt.block_on(thread_service.start_auto_turn_with_handle(
                &thread_session,
                handle,
                &primary_agent,
                &project,
            )) {
                tracing::warn!(
                    session_id = %thread_session,
                    %error,
                    label = label,
                    "idle auto-turn start failed"
                );
                thread_service.release_auto_turn(&thread_session, &thread_turn);
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(
            %session_id,
            %error,
            label = label,
            "idle auto-turn thread failed to spawn"
        );
        service.release_auto_turn(&session_id, &turn_id);
    }
}

/// Run `on_finished` for every finished turn on a dedicated thread.
pub(crate) fn install_turn_finished_flush<F>(
    service: &Arc<SessionService>,
    thread_name: &'static str,
    on_finished: F,
) where
    F: Fn(&str) + Send + 'static,
{
    let mut rx = service.subscribe_lifecycle();
    let spawned = std::thread::Builder::new()
        .name(thread_name.to_string())
        .spawn(move || loop {
            match rx.blocking_recv() {
                Ok(LifecycleEvent::TurnFinished { session_id, .. }) => on_finished(&session_id),
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(
            %error,
            thread_name = thread_name,
            "idle auto-turn flush thread failed to spawn"
        );
    }
}

/// Read the runtime snapshot once and let `plan` decide from it.
pub(crate) fn spawn_from_snapshot<F>(
    runtime: &RwLock<RuntimeHandle>,
    service: &Arc<SessionService>,
    label: &'static str,
    plan: F,
) where
    F: FnOnce(&RuntimeHandle) -> IdleTurn,
{
    let runtime = runtime.read().expect("runtime lock").clone();
    if let IdleTurn::Prepared(prepared) = plan(&runtime) {
        spawn_prepared(&runtime, service, prepared, label);
    }
}
