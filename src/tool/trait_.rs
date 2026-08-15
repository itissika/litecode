use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::session::store::Session;
use crate::tool::write_lock::ResourceKey;
use crate::types::ToolCallResult;

/// Cross-cutting execution facts prepared once by the executor. These values
/// are deliberately separate from model-controlled tool JSON.
#[derive(Clone)]
pub struct ToolExecutionContext {
    pub path_mode: crate::workspace::ToolPathMode,
    pub workspace_root: std::path::PathBuf,
    pub call_id: String,
    pub cancel: tokio_util::sync::CancellationToken,
    pub output_limit: usize,
    pub session_id: String,
}

pub trait Tool: Send + Sync {
    fn name(&self) -> &str;

    fn schema(&self) -> Value;

    /// Unit-test / sync helper. Production Agent calls must enter through
    /// [`execute`].
    fn call(&self, input: Value) -> ToolCallResult {
        let mut result = self.call_inner(input).finalize_signals();
        let max = self.max_result_size();
        if max < usize::MAX {
            result.content = Session::truncated_tool_result(&result.content, max);
        }
        result
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        ToolCallResult::ok("")
    }

    /// Async helper used by unit tests and the default [`execute`] path for
    /// tools that do not need execution-context facts.
    fn call_async(
        &self,
        input: Value,
    ) -> Pin<Box<dyn Future<Output = ToolCallResult> + Send + '_>> {
        let result = self.call(input);
        Box::pin(std::future::ready(result))
    }

    /// Single production execution boundary.
    fn execute(
        &self,
        input: Value,
        _execution: ToolExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ToolCallResult> + Send + '_>> {
        self.call_async(input)
    }

    fn description(&self, ctx: &Context) -> String;

    fn timeout(&self) -> Option<u64> {
        None
    }

    /// Returns true if this tool can run concurrently with other concurrency-safe tools.
    /// Returns false if the tool must execute strictly serially.
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    /// Resources this call touches. Overlapping keys in the same step do not
    /// run concurrently (same-path read/write share one lock graph).
    ///
    /// Write/edit/read return [`ResourceKey::File`]; bash returns per-path keys
    /// or [`ResourceKey::Workspace`] when paths cannot be attributed.
    fn resource_keys(
        &self,
        _input: &Value,
        _path_mode: crate::workspace::ToolPathMode,
        _workspace_root: &std::path::Path,
    ) -> Vec<ResourceKey> {
        vec![]
    }

    /// True when turn-cancel can kill the tool's real work and join it.
    /// `tokio::task::abort` of the wrapper is not enough. Default false:
    /// blocking MCP/custom/webfetch threads cannot be claimed cancelled.
    fn is_cancellable(&self) -> bool {
        false
    }

    fn max_result_size(&self) -> usize {
        8000
    }

    fn validate_input(&self, _input: &Value) -> std::result::Result<(), String> {
        Ok(())
    }

    fn is_destructive(
        &self,
        _input: &Value,
        _path_mode: crate::workspace::ToolPathMode,
        _workspace_root: &std::path::Path,
    ) -> bool {
        false
    }

    /// Set the active session id for session-scoped tools (plan, todo).
    /// Default no-op for stateless tools.
    fn set_active_session(&self, _session_id: String) {}

    /// Shared TerminalHub for agent bash jobs (bash / wait_shell / kill_shell).
    fn agent_terminal(&self) -> Option<std::sync::Arc<crate::terminal::TerminalHub>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AsyncOnlyTool;

    impl Tool for AsyncOnlyTool {
        fn name(&self) -> &str {
            "async_only"
        }

        fn schema(&self) -> Value {
            serde_json::json!({})
        }

        fn call_inner(&self, _input: Value) -> ToolCallResult {
            ToolCallResult::ok("sync fallback")
        }

        fn call_async(
            &self,
            _input: Value,
        ) -> Pin<Box<dyn Future<Output = ToolCallResult> + Send + '_>> {
            Box::pin(std::future::ready(ToolCallResult::ok(
                "async implementation",
            )))
        }

        fn description(&self, _ctx: &Context) -> String {
            String::new()
        }
    }

    #[tokio::test]
    async fn execution_context_preserves_async_override() {
        let tool = AsyncOnlyTool;
        let result = tool
            .execute(
                Value::Null,
                ToolExecutionContext {
                    path_mode: crate::workspace::ToolPathMode::Safe,
                    workspace_root: std::path::PathBuf::from("."),
                    call_id: "test-call".into(),
                    cancel: tokio_util::sync::CancellationToken::new(),
                    output_limit: 8_000,
                    session_id: String::new(),
                },
            )
            .await;
        assert_eq!(result.content, "async implementation");
    }
}
