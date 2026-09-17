//! Start a turn the same way the human controller does: reserve, then spawn,
//! then `SessionManager::start_turn`. Not a SessionManager API.

use std::sync::Arc;

use crate::permission::deny_permission_sink;
use crate::runtime::{RuntimeHandle, TurnOptions, spawn_turn};
use crate::session::manager::SessionManager;
use crate::types::LitecodeError;

/// Returns the new `turn_id`. On failure the reservation is released (and the
/// spawned turn is cancelled when spawn already succeeded).
pub(crate) async fn start_turn_like_human(
    runtime: &RuntimeHandle,
    sessions: &Arc<SessionManager>,
    session_id: &str,
    input: String,
    agent_id: &str,
    project: &str,
    opts: TurnOptions,
) -> Result<String, LitecodeError> {
    let mut runtime = runtime.clone();
    runtime.apply_non_engine()?;
    runtime.sync_workspace_tool_readiness();

    let turn_id = uuid::Uuid::new_v4().to_string();
    let step_max = opts.max_steps_override.unwrap_or_else(|| {
        crate::config::bridge::agent_config_for(&runtime.resolved, agent_id)
            .map(|agent| agent.max_steps)
            .unwrap_or(50)
    });
    sessions.reserve_turn(session_id, turn_id.clone(), step_max, agent_id, project)?;

    let handle = match spawn_turn(
        &runtime,
        session_id.to_string(),
        Arc::clone(sessions),
        input,
        deny_permission_sink(),
        turn_id.clone(),
        opts,
    ) {
        Ok(handle) => handle,
        Err(error) => {
            sessions.release_turn_reservation(session_id, &turn_id);
            return Err(LitecodeError::ToolExecution(error.to_string()));
        }
    };
    if let Err(error) = sessions
        .start_turn(session_id, handle, agent_id, project, Arc::clone(sessions))
        .await
    {
        sessions.release_turn_reservation(session_id, &turn_id);
        return Err(error);
    }
    Ok(turn_id)
}
