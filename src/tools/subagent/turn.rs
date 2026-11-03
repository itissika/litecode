//! Start a turn the same way the human controller does: reserve, then spawn,
//! then `SessionService::start_turn`. Kept as subagent orchestration, not a
//! SessionManager API.

use crate::permission::deny_permission_sink;
use crate::runtime::{RuntimeHandle, TurnOptions};
use crate::session::SessionService;
use crate::types::LitecodeError;

/// Returns the new `turn_id`. On failure the reservation is released (and the
/// spawned turn is cancelled when spawn already succeeded).
pub(crate) async fn start_turn_like_human(
    runtime: &RuntimeHandle,
    service: &SessionService,
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
    let step_max = crate::config::bridge::agent_config_for(&runtime.resolved, agent_id)
        .map(|agent| agent.max_steps)
        .unwrap_or(50);
    service.reserve_turn(session_id, turn_id.clone(), step_max, agent_id, project)?;

    let handle = match service.spawn_turn(
        &runtime,
        session_id.to_string(),
        input,
        deny_permission_sink(),
        turn_id.clone(),
        opts,
    ) {
        Ok(handle) => handle,
        Err(error) => {
            service.release_turn_reservation(session_id, &turn_id);
            return Err(LitecodeError::ToolExecution(error.to_string()));
        }
    };
    if let Err(error) = service
        .start_turn(session_id, handle, agent_id, project)
        .await
    {
        service.release_turn_reservation(session_id, &turn_id);
        return Err(error);
    }
    Ok(turn_id)
}
