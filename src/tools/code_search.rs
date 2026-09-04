use serde_json::Value;
use std::time::{Duration, Instant};

use crate::context_pipeline::Context;
use crate::engines::code_search::{
    DEFAULT_TOP_K, IndexStatus, MAX_TOP_K, ResolvedIndexView, resolve_index_view,
};
use crate::engines::{
    CodeSearchCallGate, EngineState, RetrievalCorpus, RetrievalFilters, RetrievalHit,
    RetrievalModality, RetrievalQuery, WorkspaceEngines,
};
use crate::tool::Tool;
use crate::types::ToolCallResult;

const CODE_SEARCH_WARM_WAIT: Duration = Duration::from_secs(60);
const CODE_SEARCH_WARM_POLL: Duration = Duration::from_millis(50);

pub struct CodeSearchTool {
    engines: WorkspaceEngines,
}

impl CodeSearchTool {
    pub fn new(engines: WorkspaceEngines) -> Self {
        Self { engines }
    }
}

impl Tool for CodeSearchTool {
    fn name(&self) -> &str {
        "code_search"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language or keyword query to find relevant code"
                },
                "include_pattern": {
                    "type": "string",
                    "description": "Optional glob filter for file paths (e.g. '**/*.rs', '**/*.{ts,tsx}')"
                },
                "top_k": {
                    "type": "integer",
                    "description": format!("Number of results to return (default {DEFAULT_TOP_K}, max {MAX_TOP_K})")
                }
            },
            "required": ["query"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        let query = match crate::tool::require_nonempty_string_trimmed(&input, "query") {
            Ok(q) => q,
            Err(e) => return ToolCallResult::error(e),
        };

        let glob = input["include_pattern"].as_str().filter(|s| !s.is_empty());
        let top_k = input["top_k"]
            .as_u64()
            .map(|k| k as usize)
            .unwrap_or(DEFAULT_TOP_K)
            .clamp(1, MAX_TOP_K);

        match self.engines.state("code_search") {
            Some(EngineState::Failed) => {
                let detail = self
                    .engines
                    .last_error("code_search")
                    .unwrap_or_else(|| "code_search engine failed".into());
                return ToolCallResult::error(detail);
            }
            _ => {}
        }

        if !self.engines.is_warmed("code_search") {
            let started = Instant::now();
            while started.elapsed() < CODE_SEARCH_WARM_WAIT {
                if self.engines.is_warmed("code_search") {
                    break;
                }
                if matches!(self.engines.state("code_search"), Some(EngineState::Failed)) {
                    break;
                }
                std::thread::sleep(CODE_SEARCH_WARM_POLL);
            }
        }

        match self.engines.state("code_search") {
            Some(EngineState::Failed) => {
                let detail = self
                    .engines
                    .last_error("code_search")
                    .unwrap_or_else(|| "code_search engine failed".into());
                return ToolCallResult::error(detail);
            }
            Some(EngineState::Warm) => {}
            _ => {
                return ToolCallResult::ok(indexing_wait_message(&self.engines));
            }
        }

        match self.engines.code_search_call_gate() {
            CodeSearchCallGate::Failed(detail) => {
                return ToolCallResult::error(detail);
            }
            CodeSearchCallGate::Wait => {
                return ToolCallResult::ok(indexing_wait_message(&self.engines));
            }
            CodeSearchCallGate::Ready => {}
        }

        match self.engines.search(RetrievalQuery {
            query: query.to_string(),
            corpus: RetrievalCorpus::Code,
            modality: RetrievalModality::Semantic,
            filters: RetrievalFilters {
                glob: glob.map(str::to_string),
                ..Default::default()
            },
            top_k,
            workspace_root: None,
            offset: 0,
        }) {
            Ok(hits) => {
                if hits.is_empty() {
                    let scope = glob
                        .map(|p| format!(" for include_pattern '{p}'"))
                        .unwrap_or_default();
                    return ToolCallResult::ok(format!("No matching code chunks found{scope}."));
                }
                let lines: Vec<String> = hits
                    .iter()
                    .filter_map(|h| match h {
                        RetrievalHit::Code {
                            path,
                            start_line,
                            end_line,
                            summary,
                            score,
                        } => Some(format_code_hit(
                            path,
                            *start_line,
                            *end_line,
                            *score,
                            summary,
                        )),
                        _ => None,
                    })
                    .collect();
                ToolCallResult::ok(lines.join("\n"))
            }
            Err(e) => ToolCallResult::error(e.to_string()),
        }
    }

    fn description(&self, _ctx: &Context) -> String {
        "Semantic code search over the workspace index when the engine is Warm.".into()
    }

    fn timeout(&self) -> Option<u64> {
        Some(60)
    }
}

fn indexing_wait_message(engines: &WorkspaceEngines) -> String {
    let root = engines.code_search().workspace_root();
    let state = engines.state("code_search");
    if let Some(root) = root {
        let view = resolve_index_view(&root, state);
        if matches!(view.status, IndexStatus::Building | IndexStatus::Refreshing) {
            return format_index_progress(&view);
        }
    }
    if engines.is_refresh_busy() {
        return "code_search index is refreshing. Try again shortly.".into();
    }
    "code_search engine is still starting. Try again shortly.".into()
}

fn format_code_hit(
    path: &str,
    start_line: u32,
    end_line: u32,
    score: f64,
    summary: &str,
) -> String {
    format!(
        "{} (score {score:.3}): {summary}",
        crate::tool::format_path_lines(path, start_line, end_line)
    )
}

fn format_index_progress(view: &ResolvedIndexView) -> String {
    let kind = match view.status {
        IndexStatus::Building => "building",
        IndexStatus::Refreshing => "refreshing",
        other => {
            return format!("code_search index status is {other:?}. Try again shortly.");
        }
    };
    if let Some(p) = &view.progress {
        let eta = if p.files_total > 0 && p.files_done > 0 {
            format!(
                " (~{}% files)",
                (p.files_done.saturating_mul(100)) / p.files_total
            )
        } else {
            String::new()
        };
        format!(
            "code_search index is {kind} ({}): {}/{} files, {} chunks done{eta}. Try again shortly.",
            format!("{:?}", p.phase).to_lowercase(),
            p.files_done,
            p.files_total,
            p.chunks_done,
        )
    } else {
        format!("code_search index is {kind}. Try again shortly.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_up_to_sixty_seconds_then_searches_not_loading() {
        assert_eq!(CODE_SEARCH_WARM_WAIT, Duration::from_secs(60));
        assert_eq!(CODE_SEARCH_WARM_POLL, Duration::from_millis(50));
    }

    #[test]
    fn indexing_wait_message_mentions_progress() {
        let view = ResolvedIndexView {
            status: IndexStatus::Building,
            progress: Some(crate::engines::code_search::IndexingProgress {
                phase: crate::engines::code_search::IndexPhase::Embedding,
                files_done: 3,
                files_total: 10,
                chunks_done: 12,
            }),
            job_error: None,
        };
        let msg = format_index_progress(&view);
        assert!(msg.contains("building"), "{msg}");
        assert!(msg.contains("3/10"), "{msg}");
    }

    #[test]
    fn indexing_wait_message_reads_job_json() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        crate::engines::code_search::init_workspace_index(root).unwrap();
        crate::engines::code_search::begin_building(root);
        crate::engines::code_search::update_build_progress(
            root,
            crate::engines::code_search::IndexingProgress {
                phase: crate::engines::code_search::IndexPhase::Embedding,
                files_done: 2,
                files_total: 8,
                chunks_done: 4,
            },
        );

        let engines = WorkspaceEngines::new();
        engines.code_search().set_workspace(root.to_path_buf());
        engines.set_state_for_test("code_search", EngineState::Warming);

        let msg = indexing_wait_message(&engines);
        assert!(msg.contains("building"), "{msg}");
        assert!(msg.contains("2/8"), "{msg}");
        assert!(
            !msg.contains("No matching code"),
            "must not fake empty hits while indexing: {msg}"
        );
    }

    #[test]
    fn failed_engine_returns_last_error() {
        let engines = WorkspaceEngines::new();
        engines.set_state_for_test("code_search", EngineState::Failed);
        engines.set_last_error_for_test("code_search", "embedder missing");
        let tool = CodeSearchTool::new(engines);
        let result = tool.call_inner(serde_json::json!({ "query": "auth" }));
        assert_eq!(result.level, crate::types::ToolSignalLevel::Error);
        assert!(
            result.content.contains("embedder missing"),
            "{}",
            result.content
        );
    }

    #[test]
    fn warm_refreshing_returns_wait_not_hits() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        crate::engines::code_search::init_workspace_index(root).unwrap();
        crate::engines::code_search::begin_refreshing(root);

        let engines = WorkspaceEngines::new();
        engines.code_search().set_workspace(root.to_path_buf());
        engines.set_state_for_test("code_search", EngineState::Warm);

        let tool = CodeSearchTool::new(engines);
        let result = tool.call_inner(serde_json::json!({ "query": "auth" }));
        assert_eq!(result.level, crate::types::ToolSignalLevel::Ok);
        assert!(result.content.contains("refreshing"), "{}", result.content);
        assert!(
            !result.content.contains("No matching code"),
            "must not search stale corpus while refreshing: {}",
            result.content
        );
    }

    #[test]
    fn warm_refresh_busy_returns_wait_not_hits() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        crate::engines::code_search::init_workspace_index(root).unwrap();

        let engines = WorkspaceEngines::new();
        engines.code_search().set_workspace(root.to_path_buf());
        engines.set_state_for_test("code_search", EngineState::Warm);
        engines.set_refresh_busy_for_test(true);

        let tool = CodeSearchTool::new(engines);
        let result = tool.call_inner(serde_json::json!({ "query": "auth" }));
        assert_eq!(result.level, crate::types::ToolSignalLevel::Ok);
        assert!(
            result.content.contains("refreshing") || result.content.contains("Try again shortly"),
            "{}",
            result.content
        );
        assert!(
            !result.content.contains("No matching code"),
            "{}",
            result.content
        );
    }

    #[test]
    fn warm_failed_index_job_returns_error_not_hits() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        crate::engines::code_search::init_workspace_index(root).unwrap();
        crate::engines::code_search::mark_index_job_failed(root, "embed exploded");

        let engines = WorkspaceEngines::new();
        engines.code_search().set_workspace(root.to_path_buf());
        engines.set_state_for_test("code_search", EngineState::Warm);

        let tool = CodeSearchTool::new(engines);
        let result = tool.call_inner(serde_json::json!({ "query": "auth" }));
        assert_eq!(result.level, crate::types::ToolSignalLevel::Error);
        assert!(
            result.content.contains("embed exploded"),
            "{}",
            result.content
        );
    }

    #[test]
    fn hit_line_uses_path_l_label() {
        assert_eq!(
            format_code_hit("src/a.rs", 12, 20, 0.5, "fn foo"),
            "src/a.rs:L12-20 (score 0.500): fn foo"
        );
        assert_eq!(
            format_code_hit("src/a.rs", 12, 12, 0.25, "let x"),
            "src/a.rs:L12 (score 0.250): let x"
        );
    }
}
