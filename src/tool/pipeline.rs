use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::runtime::context::RuntimeContext;
use crate::types::{FunctionToolCall, ToolCallResult, ToolResultData, Transcript};

use super::executor::{
    PlanTouch, ToolCallOrigin, ToolCaller, execute_tool_call, outputs_and_data_from_tool_results,
    partition_tool_calls,
};
use crate::runtime::diagnostics::{AuditEvent, emit_tool_audit};

pub struct ToolPipeline {
    runtime: Arc<RuntimeContext>,
    session_id: String,
    /// Told which files a finished tool declared, so the reminder engine can
    /// tell this session's writes from later external edits.
    file_note: Option<Arc<dyn Fn(&[PathBuf]) + Send + Sync>>,
    plan_touch: Option<PlanTouch>,
}

impl ToolPipeline {
    pub fn new(runtime: Arc<RuntimeContext>) -> Self {
        Self {
            runtime,
            session_id: String::new(),
            file_note: None,
            plan_touch: None,
        }
    }

    pub fn set_file_note(&mut self, hook: Arc<dyn Fn(&[PathBuf]) + Send + Sync>) {
        self.file_note = Some(hook);
    }

    pub fn set_plan_touch(&mut self, hook: PlanTouch) {
        self.plan_touch = Some(hook);
    }

    pub fn bind_session(&mut self, session_id: impl Into<String>) {
        self.session_id = session_id.into();
    }

    pub(crate) fn set_runtime(&mut self, runtime: Arc<RuntimeContext>) {
        self.runtime = runtime;
    }

    fn call_origin(&self) -> ToolCallOrigin {
        let code_names = self
            .runtime
            .tools
            .iter()
            .find_map(|tool| tool.code_bindings())
            .unwrap_or_else(|| Arc::from([]));
        ToolCallOrigin {
            caller: ToolCaller::Direct,
            code_names,
            file_note: self.file_note.clone(),
            permission_sink: Some(Arc::clone(&self.runtime.permission_sink)),
            plan_touch: self.plan_touch.clone(),
            deadline: None,
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
        self.execute_batch_cancellable(invocations, transcript, || false, &mut Vec::new())
            .await
    }

    pub(crate) async fn execute_batch_cancellable(
        &self,
        invocations: &[FunctionToolCall],
        transcript: &mut Transcript,
        is_cancelled: impl Fn() -> bool,
        result_data: &mut Vec<ToolResultData>,
    ) -> crate::types::Result<()> {
        let cancel = self.runtime.cancel.clone();
        let mut results_by_id: HashMap<String, ToolCallResult> = HashMap::new();
        if is_cancelled() || cancel.is_cancelled() {
            self.append_cancelled_outputs(invocations, results_by_id, transcript, result_data);
            return Err(crate::types::LitecodeError::Canceled);
        }

        let origin = self.call_origin();
        let batches = partition_tool_calls(
            invocations,
            &self.runtime.tools,
            &self.runtime.ctx.cwd,
            |name| self.runtime.permission.path_mode(name).to_tool_path_mode(),
        );
        for batch in batches {
            if is_cancelled() || cancel.is_cancelled() {
                self.append_cancelled_outputs(invocations, results_by_id, transcript, result_data);
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
                        let origin = origin.clone();
                        let handle = tokio::spawn(async move {
                            let result = execute_tool_call(
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
                                runtime.turn_anchor_seq(),
                                write_lock,
                                runtime.session.clone(),
                                origin,
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
                        self.join_tool_handle(
                            handle,
                            &captured_id,
                            &mut results_by_id,
                            invocations,
                        )
                        .await;
                        for (id, remaining) in iter {
                            self.join_tool_handle(remaining, &id, &mut results_by_id, invocations)
                                .await;
                        }
                        self.append_cancelled_outputs(
                            invocations,
                            results_by_id,
                            transcript,
                            result_data,
                        );
                        return Err(crate::types::LitecodeError::Canceled);
                    }
                    match handle.await {
                        Ok((tool_use_id, result)) => {
                            results_by_id.insert(tool_use_id, result);
                            if is_cancelled() || cancel.is_cancelled() {
                                for (id, remaining) in iter {
                                    self.join_tool_handle(
                                        remaining,
                                        &id,
                                        &mut results_by_id,
                                        invocations,
                                    )
                                    .await;
                                }
                                self.append_cancelled_outputs(
                                    invocations,
                                    results_by_id,
                                    transcript,
                                    result_data,
                                );
                                return Err(crate::types::LitecodeError::Canceled);
                            }
                        }
                        Err(join_err) => {
                            tracing::error!(
                                "tokio task panicked during concurrent tool execution: {}",
                                join_err
                            );
                            // WP-188 observe-only
                            let tool_name = invocations
                                .iter()
                                .find(|c| c.call_id == captured_id)
                                .map(|c| c.name.as_str())
                                .unwrap_or("unknown");
                            let mut event =
                                AuditEvent::new("panic", &self.session_id, tool_name, &captured_id);
                            event.ok = Some(false);
                            event.error_kind = Some("panic".into());
                            let msg = format!("tool execution task panicked: {join_err}");
                            event.error = Some(msg.chars().take(200).collect());
                            emit_tool_audit(&self.runtime.data_root, &event);
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
                        self.append_cancelled_outputs(
                            invocations,
                            results_by_id,
                            transcript,
                            result_data,
                        );
                        return Err(crate::types::LitecodeError::Canceled);
                    }
                    let tool_use_id = tu.call_id.clone();
                    let result = execute_tool_call(
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
                        self.runtime.turn_anchor_seq(),
                        Arc::clone(&self.runtime.write_lock),
                        self.runtime.session.clone(),
                        origin.clone(),
                    )
                    .await;
                    results_by_id.insert(tool_use_id.clone(), result);
                    if is_cancelled() || cancel.is_cancelled() {
                        self.append_cancelled_outputs(
                            invocations,
                            results_by_id,
                            transcript,
                            result_data,
                        );
                        return Err(crate::types::LitecodeError::Canceled);
                    }
                }
            }
        }

        // Output-only: FunctionCall Items already live in transcript from model output.
        let (items, data) =
            outputs_and_data_from_tool_results(invocations, results_by_id, &self.runtime.data_root);
        transcript.extend(items);
        result_data.extend(data);

        Ok(())
    }

    /// Join a spawned tool task. Cancellation must kill-and-wait inside the
    /// tool; dropping the wrapper with `abort()` is not process recovery.
    async fn join_tool_handle(
        &self,
        handle: tokio::task::JoinHandle<(String, ToolCallResult)>,
        captured_id: &str,
        results_by_id: &mut HashMap<String, ToolCallResult>,
        invocations: &[FunctionToolCall],
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
                // WP-188 observe-only: join panic → audit phase=panic
                let tool_name = invocations
                    .iter()
                    .find(|c| c.call_id == captured_id)
                    .map(|c| c.name.as_str())
                    .unwrap_or("unknown");
                let mut event = AuditEvent::new("panic", &self.session_id, tool_name, captured_id);
                event.ok = Some(false);
                event.error_kind = Some("panic".into());
                let msg = format!("tool execution task panicked: {join_err}");
                event.error = Some(msg.chars().take(200).collect());
                emit_tool_audit(&self.runtime.data_root, &event);
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
        result_data: &mut Vec<ToolResultData>,
    ) {
        let (items, data) =
            outputs_and_data_from_tool_results(invocations, results_by_id, &self.runtime.data_root);
        transcript.extend(items);
        result_data.extend(data);
    }
}
