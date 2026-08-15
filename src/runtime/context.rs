use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::hook::HookDispatcher;
use crate::llm::ToolDef;
use crate::permission::{PermissionEngine, PermissionSink};
use crate::tool::trait_::Tool;
use crate::tool::write_lock::WorkspaceWriteLock;

/// Shared runtime dependencies for tool execution and LLM tool schemas (Phase 7 R7.1).
#[derive(Clone)]
pub struct RuntimeContext {
    pub tools: Vec<Arc<dyn Tool>>,
    pub permission: PermissionEngine,
    pub hook_dispatcher: HookDispatcher,
    pub ctx: Context,
    pub agent_name: String,
    pub permission_sink: Arc<dyn PermissionSink>,
    pub cancel: CancellationToken,
    pub data_root: PathBuf,
    pub spill_threshold: usize,
    /// §5.2 anchor k for the active turn (-1 = unset).
    pub turn_anchor_k: Arc<AtomicI64>,
    /// 进程级写锁，用于跨 session 的资源写互斥（Phase 3）。
    pub write_lock: Arc<WorkspaceWriteLock>,
}

impl RuntimeContext {
    pub fn new(
        tools: Vec<Arc<dyn Tool>>,
        permission: PermissionEngine,
        hook_dispatcher: HookDispatcher,
        ctx: Context,
        agent_name: impl Into<String>,
        permission_sink: Arc<dyn PermissionSink>,
        cancel: CancellationToken,
        data_root: PathBuf,
        spill_threshold: usize,
        write_lock: Arc<WorkspaceWriteLock>,
    ) -> Self {
        Self {
            tools,
            permission,
            hook_dispatcher,
            ctx,
            agent_name: agent_name.into(),
            permission_sink,
            cancel,
            data_root,
            spill_threshold,
            turn_anchor_k: Arc::new(AtomicI64::new(-1)),
            write_lock,
        }
    }

    pub fn set_turn_anchor_k(&self, k: i64) {
        self.turn_anchor_k.store(k, Ordering::Relaxed);
    }

    pub fn turn_anchor_k(&self) -> Option<i64> {
        let k = self.turn_anchor_k.load(Ordering::Relaxed);
        (k >= 0).then_some(k)
    }

    pub fn without_spill(
        tools: Vec<Arc<dyn Tool>>,
        permission: PermissionEngine,
        hook_dispatcher: HookDispatcher,
        ctx: Context,
        agent_name: impl Into<String>,
        permission_sink: Arc<dyn PermissionSink>,
        cancel: CancellationToken,
        write_lock: Arc<WorkspaceWriteLock>,
    ) -> Self {
        Self::new(
            tools,
            permission,
            hook_dispatcher,
            ctx,
            agent_name,
            permission_sink,
            cancel,
            PathBuf::from("."),
            0,
            write_lock,
        )
    }

    pub fn tool_defs(&self) -> Vec<ToolDef> {
        self.tools
            .iter()
            .map(|t| ToolDef {
                name: t.name().to_string(),
                description: t.description(&self.ctx),
                input_schema: t.schema(),
            })
            .collect()
    }
}
