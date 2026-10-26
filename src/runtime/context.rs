use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::llm::ToolDef;
use crate::permission::{PermissionEngine, PermissionSink};
use crate::tool::trait_::Tool;
use crate::tool::write_lock::WorkspaceWriteLock;

/// Shared runtime dependencies for tool execution and LLM tool schemas (Phase 7 R7.1).
#[derive(Clone)]
pub struct RuntimeContext {
    pub tools: Vec<Arc<dyn Tool>>,
    /// Tool schemas derived from `tools` + `ctx`. Rebuild whenever `ctx` changes
    /// (see [`RuntimeContext::derive_defs`]); the per-step request path reuses it
    /// instead of re-deriving descriptions and JSON schemas on every call.
    pub defs: Vec<ToolDef>,
    pub permission: PermissionEngine,
    pub ctx: Context,
    pub agent_name: String,
    pub permission_sink: Arc<dyn PermissionSink>,
    pub cancel: CancellationToken,
    pub data_root: PathBuf,
    pub spill_threshold: usize,
    /// Turn-start snapshot stem (`next_seq` after the user row). -1 = unset.
    pub turn_anchor_seq: Arc<AtomicI64>,
    /// Process-wide write lock for cross-session resource exclusion (Phase 3).
    pub write_lock: Arc<WorkspaceWriteLock>,
    pub session: Option<crate::session::SessionDataReader>,
}

impl RuntimeContext {
    pub fn new(
        tools: Vec<Arc<dyn Tool>>,
        permission: PermissionEngine,
        ctx: Context,
        agent_name: impl Into<String>,
        permission_sink: Arc<dyn PermissionSink>,
        cancel: CancellationToken,
        data_root: PathBuf,
        spill_threshold: usize,
        write_lock: Arc<WorkspaceWriteLock>,
        session: Option<crate::session::SessionDataReader>,
    ) -> Self {
        let defs = Self::derive_defs(&tools, &ctx);
        Self {
            defs,
            tools,
            permission,
            ctx,
            agent_name: agent_name.into(),
            permission_sink,
            cancel,
            data_root,
            spill_threshold,
            turn_anchor_seq: Arc::new(AtomicI64::new(-1)),
            write_lock,
            session,
        }
    }

    pub fn set_turn_anchor_seq(&self, seq: i64) {
        self.turn_anchor_seq.store(seq, Ordering::Relaxed);
    }

    pub fn turn_anchor_seq(&self) -> Option<i64> {
        let seq = self.turn_anchor_seq.load(Ordering::Relaxed);
        (seq >= 0).then_some(seq)
    }

    pub fn without_spill(
        tools: Vec<Arc<dyn Tool>>,
        permission: PermissionEngine,
        ctx: Context,
        agent_name: impl Into<String>,
        permission_sink: Arc<dyn PermissionSink>,
        cancel: CancellationToken,
        write_lock: Arc<WorkspaceWriteLock>,
    ) -> Self {
        Self::new(
            tools,
            permission,
            ctx,
            agent_name,
            permission_sink,
            cancel,
            PathBuf::from("."),
            0,
            write_lock,
            None,
        )
    }

    pub fn tool_defs(&self) -> Vec<ToolDef> {
        self.defs.clone()
    }

    /// The single derivation of tool schemas from a tool list and its context.
    pub(crate) fn derive_defs(tools: &[Arc<dyn Tool>], ctx: &Context) -> Vec<ToolDef> {
        tools
            .iter()
            .map(|t| ToolDef {
                name: t.name().to_string(),
                description: t.description(ctx),
                input_schema: t.schema(),
            })
            .collect()
    }
}
