use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::session::store::Session;
use crate::tool::write_lock::ResourceKey;
use crate::types::ToolCallResult;

/// Where a tool may be called. The declaration lives on the tool.
///
/// `Direct` is the model-facing function call. `Code` is reachable only as
/// `tools["name"]` inside `run_code`. A tool has exactly one surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSurface {
    Direct,
    Code,
}

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
    pub session: Option<crate::session::SessionDataReader>,
}

impl ToolExecutionContext {
    pub fn session_reader(&self) -> crate::types::Result<&crate::session::SessionDataReader> {
        self.session.as_ref().ok_or_else(|| {
            crate::types::LitecodeError::ToolExecution("session reader not injected".into())
        })
    }
}

pub trait Tool: Send + Sync {
    fn name(&self) -> &str;

    /// Call surface declared by this tool. Custom and MCP tools keep [`ToolSurface::Direct`].
    fn surface(&self) -> ToolSurface {
        ToolSurface::Direct
    }

    /// Code-surface names bound by a transport such as `run_code`.
    fn code_bindings(&self) -> Option<Arc<[String]>> {
        None
    }

    /// The code transport itself. Permission allow follows this identity, not the name.
    fn is_code_transport(&self) -> bool {
        false
    }

    fn schema(&self) -> Value;

    /// Historical `file_path` is still an invocation of this tool. The schema
    /// the model sees only advertises `path`.
    fn accepts_historical_file_path(&self) -> bool {
        false
    }

    /// Copy a historical `file_path` onto `path` (and the reverse) before
    /// validation, permission, and execution.
    fn prepare_input(&self, input: &mut Value) {
        if self.accepts_historical_file_path() {
            crate::tools::file_path::alias_code_path(input);
        }
    }

    /// Arguments checked against [`schema`](Tool::schema). Tools that accept
    /// historical `file_path` hide that key here so the code-mode schema stays
    /// `path` only.
    fn schema_arguments<'a>(&self, input: &'a Value) -> Cow<'a, Value> {
        if self.accepts_historical_file_path() {
            Cow::Owned(crate::tools::file_path::without_legacy_file_path(input))
        } else {
            Cow::Borrowed(input)
        }
    }

    /// JSON Schema for [`ToolCallResult::data`]. Direct-call tools may omit it;
    /// tools projected into a code bundle must provide it so the generated
    /// interface can expose a stable result type.
    fn data_schema(&self) -> Option<Value> {
        None
    }

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

    /// Cooperative call budget in seconds. `None` means the tool cannot accept
    /// a deadline. A dispatcher may request a shorter call budget, but it never
    /// extends this ceiling; expiry must reach a cancellation-aware tool.
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
    /// when paths can be attributed, otherwise none.
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

    /// Inject the turn's permission Ask sink (plan create approval, etc.).
    /// Default no-op for tools that do not Ask the human themselves.
    fn set_permission_sink(&self, _sink: std::sync::Arc<dyn crate::permission::PermissionSink>) {}

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
                    session: None,
                },
            )
            .await;
        assert_eq!(result.content, "async implementation");
    }
}
