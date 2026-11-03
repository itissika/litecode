//! Start a child turn through the same SessionService business operation as
//! the human controller. Runtime preparation stays in the subagent orchestration;
//! reservation, spawn, and registration are one service call.

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
    match service
        .start_turn(
            &runtime,
            session_id,
            input,
            false,
            false,
            deny_permission_sink(),
            &turn_id,
            agent_id,
            project,
            step_max,
            opts,
        )
        .await
    {
        Ok(()) => Ok(turn_id),
        Err(crate::session::service::TurnStartError::AgentAlreadyRunning) => {
            Err(LitecodeError::AgentAlreadyRunning)
        }
        Err(crate::session::service::TurnStartError::Runtime(error)) => {
            Err(LitecodeError::ToolExecution(error.to_string()))
        }
    }
}
