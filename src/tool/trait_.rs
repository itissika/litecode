use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::context_pipeline::Context;
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
    pub session: Option<crate::session::SessionDataReader>,
}

/// Provider-facing description and input schema for the outer `run_code` transport.
#[derive(Debug, Clone, PartialEq)]
pub struct TransportToolSpec {
    pub description: String,
    pub input_schema: Value,
}

/// Contract supplied by a runtime-defined custom or MCP code tool.
///
/// Built-in code tools declare their TypeScript contract in `sdk/src/api.d.ts`.
/// When `output_schema` is `None`, the SDK defaults to open `unknown` and skips
/// strict output-shape enforcement.
#[derive(Debug, Clone, PartialEq)]
pub struct DynamicCodeSpec {
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Option<Value>,
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

    /// Code-surface names bound by a transport such as `run_code`.
    fn code_bindings(&self) -> Option<Arc<[String]>> {
        None
    }

    /// The outer `run_code` transport itself. Only this returns `true`.
    fn is_code_transport(&self) -> bool {
        false
    }

    /// Provider-facing transport spec (implemented only by `run_code`).
    fn transport_spec(&self, _ctx: &Context) -> Option<TransportToolSpec> {
        None
    }

    /// Runtime-defined code tool contract (custom & MCP tools).
    fn dynamic_code_spec(&self) -> Option<DynamicCodeSpec> {
        None
    }

    /// Optional runtime JSDoc note appended to a built-in fragment (e.g.
    /// `subagent.launch`'s dynamic `allowed_subagents` allowlist).
    fn dynamic_sdk_note(&self) -> Option<String> {
        None
    }

    /// Hook before validation. Code file tools do not rewrite arguments.
    fn prepare_input(&self, _input: &mut Value) {}

    /// Arguments passed through the same schema projection used for validation.
    fn schema_arguments<'a>(&self, input: &'a Value) -> Cow<'a, Value> {
        Cow::Borrowed(input)
    }

    /// Unit-test / sync helper. Production Agent calls must enter through
    /// [`execute`]; output capping/spilling happens at the unified executor boundary.
    fn call(&self, input: Value) -> ToolCallResult {
        self.call_inner(input).finalize_signals()
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

    /// Safety-net budget for one call. `None` means this tool does not arm a
    /// deadline. The model cannot override it. A nested call is additionally
    /// capped by the enclosing `run_code` cap. Expiry cancels the call and
    /// waits for it to settle.
    fn timeout(&self) -> Option<Duration> {
        None
    }

    /// Resources this call reads and writes. Reads are shared. Writes are
    /// exclusive. The executor acquires them after authorization, in sorted
    /// key order. Empty means this call does not take a resource lock.
    fn effects(
        &self,
        _input: &Value,
        _path_mode: crate::workspace::ToolPathMode,
        _workspace_root: &std::path::Path,
    ) -> crate::tool::resource_gate::ToolEffects {
        crate::tool::resource_gate::ToolEffects::default()
    }

    /// Tools that do not declare [`effects`](Tool::effects) use this. A tool
    /// with writes is scheduled from those effects instead, so the five file
    /// tools do not special-case this method.
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    /// Paths a settled call should report to the file tracker. This is the
    /// resolved path, not the lock identity: the tracker opens it.
    ///
    /// Bash returns per-path keys when paths can be attributed, otherwise none.
    /// Locking uses [`effects`](Tool::effects).
    fn resource_keys(
        &self,
        _input: &Value,
        _path_mode: crate::workspace::ToolPathMode,
        _workspace_root: &std::path::Path,
    ) -> Vec<ResourceKey> {
        vec![]
    }

    /// When true, the executor hands this call's resource-gate and write-lock
    /// guards to [`crate::tool::run_scope::SpawnLocks`] instead of holding them
    /// for the whole call. The tool releases them once its process is spawned
    /// (`run`); every other exit path drops them with the call.
    fn releases_lock_after_spawn(&self) -> bool {
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
}

#[cfg(test)]
mod tests {}
