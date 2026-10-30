//! One projection from a caller's visible tools to direct list and code surface.
//!
//! Code tool public types are owned by the JS/TS SDK. Projection only classifies
//! tools, resolves the generated contract for the bound code set, and records
//! duplicate/unsafe names.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;

use crate::tool::ToolSurface;
use crate::tool::code_contract::{CodeSurface, build_surface};
use crate::tool::trait_::Tool;

/// A tool removed from the runtime surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RejectedTool {
    pub name: String,
    pub reason: String,
}

/// Direct list, code surface, and admission failures for one caller.
#[derive(Clone)]
pub struct ToolProjection {
    pub direct: Vec<Arc<dyn Tool>>,
    pub code_tools: Vec<Arc<dyn Tool>>,
    /// Present iff the code tool set is non-empty and its JS contract assembled.
    pub code_surface: Option<Arc<CodeSurface>>,
    pub rejected: Vec<RejectedTool>,
}

impl ToolProjection {
    pub fn whitelist(&self) -> Vec<String> {
        match &self.code_surface {
            Some(surface) => surface.tools.keys().cloned().collect(),
            None => Vec::new(),
        }
    }

    pub fn sdk(&self) -> &str {
        self.code_surface
            .as_ref()
            .map(|surface| surface.sdk.as_str())
            .unwrap_or("")
    }
}

fn unsafe_name_reason(name: &str) -> Option<String> {
    if name.is_empty() || name.chars().any(char::is_control) {
        return Some("tool name is empty or contains control characters".into());
    }
    if name.split('.').any(str::is_empty) {
        return Some("tool name contains an empty suite segment".into());
    }
    None
}

/// Project `tools` after the caller has already applied its own visibility filter.
pub fn project_tools(tools: Vec<Arc<dyn Tool>>) -> ToolProjection {
    let mut direct = Vec::new();
    let mut code_candidates = Vec::new();
    let mut rejected = Vec::new();
    let mut name_counts: HashMap<String, usize> = HashMap::new();
    for tool in &tools {
        *name_counts.entry(tool.name().to_string()).or_default() += 1;
    }

    for tool in tools {
        let name = tool.name().to_string();
        if name_counts.get(&name).copied().unwrap_or(0) > 1 {
            let reason = format!("tool name '{name}' is declared more than once");
            tracing::warn!(tool = %name, %reason, "duplicate tool name rejected");
            rejected.push(RejectedTool { name, reason });
            continue;
        }
        if let Some(reason) = unsafe_name_reason(&name) {
            tracing::warn!(tool = %name, %reason, "unsafe tool name rejected");
            rejected.push(RejectedTool { name, reason });
            continue;
        }
        match tool.surface() {
            ToolSurface::Direct => direct.push(tool),
            ToolSurface::Code => code_candidates.push(tool),
        }
    }

    direct.sort_by(|left, right| left.name().cmp(right.name()));
    code_candidates.sort_by(|left, right| left.name().cmp(right.name()));

    // Admit code tools one at a time. A malformed dynamic tool or a duplicate
    // generated type removes only that tool, not the rest of the code surface.
    let mut admitted = Vec::new();
    let mut surface = None;
    for tool in code_candidates {
        let mut trial = admitted.clone();
        trial.push(Arc::clone(&tool));
        match build_surface(&trial) {
            Ok(assembled) => {
                admitted.push(tool);
                surface = Some(assembled);
            }
            Err(reason) => {
                tracing::warn!(tool = %tool.name(), %reason, "code tool failed contract admission");
                rejected.push(RejectedTool {
                    name: tool.name().to_string(),
                    reason,
                });
            }
        }
    }
    let code_surface = surface.map(Arc::new);

    rejected.sort_by(|left, right| left.name.cmp(&right.name));
    ToolProjection {
        direct,
        code_tools: admitted,
        code_surface,
        rejected,
    }
}
