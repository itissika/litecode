use std::collections::HashMap;
use std::sync::Arc;

use crate::runtime::context::RuntimeContext;
use crate::types::{FunctionToolCall, ToolCallResult, Transcript};

use super::executor::{outputs_from_tool_results, partition_tool_calls, run_tool};

pub struct ToolPipeline {
    runtime: Arc<RuntimeContext>,
    session_id: String,
}

impl ToolPipeline {
    pub fn new(runtime: Arc<RuntimeContext>) -> Self {
        Self {
            runtime,
            session_id: String::new(),
        }
    }

    pub fn bind_session(&mut self, session_id: impl Into<String>) {
        self.session_id = session_id.into();
    }

    pub(crate) fn set_runtime(&mut self, runtime: Arc<RuntimeContext>) {
        self.runtime = runtime;
    }

    /// Inject session_id into session-scoped tools before execution.
    fn inject_sessions(&self) {
        let sid = &self.session_id;
        for tool in &self.runtime.tools {
            tool.set_active_session(sid.clone());
        }
    }

    /// Execute tools for `invocations` and append **FunctionCallOutput** Items only.
    ///
    /// FunctionCall Items must already be in `transcript` (from model output).
    /// Does not synthesize Reasoning/Message Items from strings.
    pub async fn execute_batch(
        &self,
        invocations: &[FunctionToolCall],
        transcript: &mut Transcript,
    ) -> crate::types::Result<()> {
        self.execute_batch_cancellable(invocations, transcript, || false)
            .await
    }

    pub(crate) async fn execute_batch_cancellable(
        &self,
        invocations: &[FunctionToolCall],
        transcript: &mut Transcript,
        is_cancelled: impl Fn() -> bool,
    ) -> crate::types::Result<()> {
        let cancel = self.runtime.cancel.clone();
        let mut results_by_id: HashMap<String, ToolCallResult> = HashMap::new();
        if is_cancelled() || cancel.is_cancelled() {
            self.append_cancelled_outputs(invocations, results_by_id, transcript);
            return Err(crate::types::LitecodeError::Canceled);
        }

        self.inject_sessions();

        let batches = partition_tool_calls(
            invocations,
            &self.runtime.tools,
            &self.runtime.ctx.cwd,
            |name| self.runtime.permission.path_mode(name).to_tool_path_mode(),
        );
        for batch in batches {
            if is_cancelled() || cancel.is_cancelled() {
                self.append_cancelled_outputs(invocations, results_by_id, transcript);
                return Err(crate::types::LitecodeError::Canceled);
            }
            if batch.is_concurrency_safe {
                let handles: Vec<_> = batch
                    .blocks
                    .iter()
                    .map(|tu| {
                        let tu = tu.clone();
                        let captured_id = tu.call_id.clone();
                        let runtime = Arc::clone(&self.runtime);
                        let session_id = self.session_id.clone();
                        let cancel = cancel.clone();
                        let write_lock = Arc::clone(&runtime.write_lock);
                        let handle = tokio::spawn(async move {
                            let result = run_tool(
                                &tu,
                                &runtime.tools,
                                &runtime.permission,
                                &runtime.ctx,
                                &session_id,
                                &runtime.agent_name,
                                runtime.permission_sink.as_ref(),
                                cancel,
                                &runtime.data_root,
                                runtime.spill_threshold,
                                runtime.turn_anchor_k(),
                                write_lock,
                            )
                            .await;
                            (tu.call_id.clone(), result)
                        });
                        (captured_id, handle)
                    })
                    .collect();

                let mut iter = handles.into_iter();
                while let Some((captured_id, handle)) = iter.next() {
                    if is_cancelled() || cancel.is_cancelled() {
                        // Signal is already set. Join remaining tasks so cancellable
                        // tools can kill their process trees; do not abort-and-forget.
                        Self::join_tool_handle(
                            handle,
                            &captured_id,
                            &mut results_by_id,
                        )
                        .await;
                        for (id, remaining) in iter {
                            Self::join_tool_handle(
                                remaining,
                                &id,
                                &mut results_by_id,
                            )
                            .await;
                        }
                        self.append_cancelled_outputs(invocations, results_by_id, transcript);
                        return Err(crate::types::LitecodeError::Canceled);
                    }
                    match handle.await {
                        Ok((tool_use_id, result)) => {
                            results_by_id.insert(tool_use_id, result);
                            if is_cancelled() || cancel.is_cancelled() {
                                for (id, remaining) in iter {
                                    Self::join_tool_handle(
                                        remaining,
                                        &id,
                                        &mut results_by_id,
                                    )
                                    .await;
                                }
                                self.append_cancelled_outputs(
                                    invocations,
                                    results_by_id,
                                    transcript,
                                );
                                return Err(crate::types::LitecodeError::Canceled);
                            }
                        }
                        Err(join_err) => {
                            tracing::error!(
                                "tokio task panicked during concurrent tool execution: {}",
                                join_err
                            );
                            results_by_id.insert(
                                captured_id.clone(),
                                ToolCallResult::error(format!(
                                    "tool execution task panicked: {}",
                                    join_err
                                )),
                            );
                        }
                    }
                }
            } else {
                for tu in &batch.blocks {
                    if is_cancelled() || cancel.is_cancelled() {
                        self.append_cancelled_outputs(invocations, results_by_id, transcript);
                        return Err(crate::types::LitecodeError::Canceled);
                    }
                    let tool_use_id = tu.call_id.clone();
                    let result = run_tool(
                        tu,
                        &self.runtime.tools,
                        &self.runtime.permission,
                        &self.runtime.ctx,
                        &self.session_id,
                        &self.runtime.agent_name,
                        self.runtime.permission_sink.as_ref(),
                        cancel.clone(),
                        &self.runtime.data_root,
                        self.runtime.spill_threshold,
                        self.runtime.turn_anchor_k(),
                        Arc::clone(&self.runtime.write_lock),
                    )
                    .await;
                    results_by_id.insert(tool_use_id.clone(), result);
                    if is_cancelled() || cancel.is_cancelled() {
                        self.append_cancelled_outputs(invocations, results_by_id, transcript);
                        return Err(crate::types::LitecodeError::Canceled);
                    }
                }
            }
        }

        // Output-only: FunctionCall Items already live in transcript from model output.
        transcript.extend(outputs_from_tool_results(
            invocations,
            results_by_id,
            &self.runtime.data_root,
        ));

        Ok(())
    }

    /// Join a spawned tool task. Cancellation must kill-and-wait inside the
    /// tool; dropping the wrapper with `abort()` is not process recovery.
    async fn join_tool_handle(
        handle: tokio::task::JoinHandle<(String, ToolCallResult)>,
        captured_id: &str,
        results_by_id: &mut HashMap<String, ToolCallResult>,
    ) {
        match handle.await {
            Ok((tool_use_id, result)) => {
                results_by_id.insert(tool_use_id, result);
            }
            Err(join_err) => {
                tracing::error!(
                    "tokio task panicked during concurrent tool execution: {}",
                    join_err
                );
                results_by_id.insert(
                    captured_id.to_string(),
                    ToolCallResult::error(format!("tool execution task panicked: {join_err}")),
                );
            }
        }
    }

    /// Cancellation is context: before a cancelled turn ends, append an output for
    /// every invocation so the persisted transcript stays valid for the next turn
    /// (no dangling FunctionCalls). Completed results are kept; the rest fall back
    /// to the "interrupted by user" text in `outputs_from_tool_results`.
    fn append_cancelled_outputs(
        &self,
        invocations: &[FunctionToolCall],
        results_by_id: HashMap<String, ToolCallResult>,
        transcript: &mut Transcript,
    ) {
        transcript.extend(outputs_from_tool_results(
            invocations,
            results_by_id,
            &self.runtime.data_root,
        ));
    }
}
