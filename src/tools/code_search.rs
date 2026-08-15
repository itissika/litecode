use serde_json::Value;

use crate::context_pipeline::Context;
use crate::engines::code_search::{DEFAULT_TOP_K, MAX_TOP_K};
use crate::engines::{
    RetrievalCorpus, RetrievalFilters, RetrievalHit, RetrievalModality, RetrievalQuery,
    WorkspaceEngines,
};
use crate::tool::Tool;
use crate::types::ToolCallResult;

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
                        } => Some(format!(
                            "{path}:{start_line}-{end_line} (score {score:.3}): {summary}"
                        )),
                        _ => None,
                    })
                    .collect();
                ToolCallResult::ok(lines.join("\n"))
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.to_lowercase().contains("warm") || msg.to_lowercase().contains("not ready") {
                    ToolCallResult::error(format!(
                        "{msg}. Enable code_search in Settings → Engines and wait until Warm"
                    ))
                } else {
                    ToolCallResult::error(msg)
                }
            }
        }
    }

    fn description(&self, _ctx: &Context) -> String {
        "Semantic code search over the workspace index when the engine is Warm.".into()
    }

    fn timeout(&self) -> Option<u64> {
        Some(60)
    }
}
