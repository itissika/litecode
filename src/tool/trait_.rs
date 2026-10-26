use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::session::store::Session;
use crate::tool::write_lock::ResourceKey;
use crate::types::ToolCallResult;

/// Where a tool may be called. The declaration lives on the tool and is the
/// same enum the settings/catalog layers persist as `CallSurface`.
pub use crate::config::schema::CallSurface as ToolSurface;

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

    /// Hook before validation. Code file tools do not rewrite arguments.
    fn prepare_input(&self, _input: &mut Value) {}

    /// Arguments checked against [`schema`](Tool::schema).
    fn schema_arguments<'a>(&self, input: &'a Value) -> Cow<'a, Value> {
        Cow::Borrowed(input)
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

    /// Safety-net budget for one call. `None` means this tool does not arm a
    /// deadline. The model cannot override it. `run_code.timeout_ms` is the
    /// only program budget. Expiry cancels the call and waits for it to settle.
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

    /// True when turn-cancel can kill the tool's real work and join it.
    /// `tokio::task::abort` of the wrapper is not enough. Default false:
    /// blocking MCP/custom/webfetch threads cannot be claimed cancelled.
    fn is_cancellable(&self) -> bool {
        false
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
mod tests {
    


}
