//! Shared workspace engine availability views for human HTTP and Agent adapters.

use std::path::Path;

use serde_json::Value;

use super::{EngineState, EngineStatus, WorkspaceEngines};
use crate::lsp::deps::probe_workspace_servers;

#[derive(Debug, Clone)]
pub struct EngineUsability {
    pub usable: &'static str,
    pub status: EngineStatus,
}

impl WorkspaceEngines {
    /// Compute retrieval + LSP detail payloads used by HTTP and other adapters.
    pub fn engines_detail_view(&self, workspace_root: &Path) -> Value {
        let statuses = self.workspace_engine_statuses(workspace_root);
        let retrieval_status = statuses
            .get("code_search")
            .cloned()
            .unwrap_or(EngineStatus {
                desired: false,
                state: None,
                error: None,
            });
        let lsp_status = statuses.get("lsp").cloned().unwrap_or(EngineStatus {
            desired: false,
            state: None,
            error: None,
        });

        let retrieval = self.retrieval_detail(workspace_root, &retrieval_status);
        let lsp = self.lsp_detail(workspace_root, &lsp_status);
        serde_json::json!({
            "retrieval": retrieval,
            "lsp": lsp,
        })
    }

    pub fn lsp_usability(&self, workspace_root: &Path) -> EngineUsability {
        let status = self
            .workspace_engine_statuses(workspace_root)
            .get("lsp")
            .cloned()
            .unwrap_or(EngineStatus {
                desired: false,
                state: None,
                error: None,
            });
        let configured_servers = crate::config::workspace::lsp_servers_from_engines(workspace_root);
        let probes = probe_workspace_servers(workspace_root);
        let configured_ok = configured_servers.iter().all(|id| {
            probes
                .iter()
                .find(|probe| &probe.id == id)
                .is_some_and(|probe| {
                    matches!(probe.status, crate::lsp::deps::LspDepStatus::Available)
                })
        });
        let usable = if !status.desired {
            "stopped"
        } else if configured_servers.is_empty() || !configured_ok {
            "unavailable"
        } else if matches!(status.state, Some(EngineState::Warming)) {
            "warming"
        } else if matches!(status.state, Some(EngineState::Warm)) {
            "ready"
        } else if matches!(status.state, Some(EngineState::Failed)) {
            "unavailable"
        } else {
            "warming"
        };
        EngineUsability { usable, status }
    }

    fn retrieval_detail(&self, workspace_root: &Path, retrieval_status: &EngineStatus) -> Value {
        let model = crate::engines::code_search::probe_embedding_model().ok();
        let meta = crate::engines::code_search::read_meta(workspace_root)
            .ok()
            .flatten();
        let vectors_ready = crate::engines::code_search::index_files_exist(workspace_root);
        let index_exists = meta.is_some() || vectors_ready;
        let needs_rebuild = meta
            .as_ref()
            .is_some_and(|m| crate::engines::code_search::needs_rebuild(m) || !vectors_ready);
        let index_view =
            crate::engines::code_search::resolve_index_view(workspace_root, retrieval_status.state);
        let retrieval_error = retrieval_status
            .error
            .clone()
            .or_else(|| index_view.job_error.clone());
        let retrieval_usable = if !retrieval_status.desired {
            "stopped"
        } else if model.as_ref().is_none_or(|m| !m.ready) {
            "unavailable"
        } else if matches!(retrieval_status.state, Some(EngineState::Failed))
            || matches!(
                index_view.status,
                crate::engines::code_search::IndexStatus::Failed
            )
        {
            "unavailable"
        } else if matches!(retrieval_status.state, Some(EngineState::Warming))
            || matches!(
                index_view.status,
                crate::engines::code_search::IndexStatus::Building
                    | crate::engines::code_search::IndexStatus::Refreshing
            )
        {
            "warming"
        } else if matches!(retrieval_status.state, Some(EngineState::Warm))
            && meta.as_ref().is_some_and(|m| m.indexed_chunks > 0)
            && !needs_rebuild
            && matches!(
                index_view.status,
                crate::engines::code_search::IndexStatus::Ready
                    | crate::engines::code_search::IndexStatus::Stale
            )
        {
            "ready"
        } else {
            "warming"
        };

        let model_json = model.map_or_else(
            || serde_json::json!({ "ready": false }),
            |model| serde_json::to_value(model).expect("model status serializes"),
        );
        let index_json = serde_json::json!({
            "status": index_view.status,
            "progress": index_view.progress,
            "exists": index_exists,
            "needs_rebuild": needs_rebuild
                || matches!(
                    index_view.status,
                    crate::engines::code_search::IndexStatus::NeedsRebuild
                        | crate::engines::code_search::IndexStatus::Absent
                ),
            "vectors_ready": vectors_ready,
            "indexed_files": meta.as_ref().map_or(0, |m| m.indexed_files),
            "indexed_chunks": meta.as_ref().map_or(0, |m| m.indexed_chunks),
            "created_at": meta.as_ref().map(|m| m.created_at.clone()),
            "model_id": meta.as_ref().map(|m| m.model_id.clone()),
            "embedder_id": meta.as_ref().map(|m| m.embedder_id.clone()),
            "pipeline_version": meta.as_ref().map(|m| m.pipeline_version),
            "pending_updates": crate::engines::code_search::read_pending_hint(workspace_root),
        });
        let policy_json = serde_json::json!({
            "product_internal_dirs": crate::workspace::filter::PRODUCT_INTERNAL_DIRS,
            "exclude_globs": crate::workspace::filter::exclude_globs(
                crate::workspace::filter::FilterPreset::Index
            ),
            "extensions": crate::workspace::filter::TEXT_EXTENSIONS,
            "max_file_bytes": crate::workspace::filter::MAX_INDEX_FILE_BYTES,
            "binary_files": true,
            "lockfiles": true,
            "minified_files": true,
        });
        serde_json::json!({
            "desired": retrieval_status.desired,
            "state": retrieval_status.state,
            "error": retrieval_error,
            "usable": retrieval_usable,
            "model": model_json,
            "index": index_json,
            "policy": policy_json,
        })
    }

    fn lsp_detail(&self, workspace_root: &Path, lsp_status: &EngineStatus) -> Value {
        let configured_servers = crate::config::workspace::lsp_servers_from_engines(workspace_root);
        let probes = probe_workspace_servers(workspace_root);
        let usability = self.lsp_usability(workspace_root);
        serde_json::json!({
            "desired": lsp_status.desired,
            "state": lsp_status.state,
            "error": lsp_status.error,
            "usable": usability.usable,
            "configured_servers": configured_servers,
            "probes": probes,
            "servers": self.lsp_hub().instance_statuses(),
        })
    }
}
