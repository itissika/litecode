//! One projection from a caller's visible tools to direct list, code surface, and rejections.
//!
//! The temporary `LITECODE_CODE_MODE` gate is a single boolean. It does not decide
//! which tool is code; that comes from [`Tool::surface`](crate::tool::Tool::surface).

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Serialize;

use crate::tool::code_manifest::{render_code_tool_description, validate_code_contract};
use crate::tool::trait_::{Tool, ToolSurface};

/// A code tool removed from both surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RejectedTool {
    pub name: String,
    pub reason: String,
}

/// Direct list, code surface, and admission failures for one caller.
///
/// `direct` does not include `run_code`. The caller appends that transport when
/// `code_tools` is non-empty. `sdk` and `whitelist` are pure functions of
/// `code_tools`.
#[derive(Clone)]
pub struct ToolProjection {
    pub direct: Vec<Arc<dyn Tool>>,
    pub code_tools: Vec<Arc<dyn Tool>>,
    pub sdk: String,
    pub whitelist: Vec<String>,
    pub rejected: Vec<RejectedTool>,
}

/// Project `tools` after the caller has already applied its own visibility filter.
///
/// When `gate_open` is false, every tool stays Direct. When it is true, each
/// tool's own [`Tool::surface`] decides the side. Code tools that fail admission
/// are removed from both sides and recorded in [`ToolProjection::rejected`].
pub fn project_tools(tools: Vec<Arc<dyn Tool>>, gate_open: bool) -> ToolProjection {
    if !gate_open {
        return ToolProjection {
            direct: tools,
            code_tools: Vec::new(),
            sdk: String::new(),
            whitelist: Vec::new(),
            rejected: Vec::new(),
        };
    }

    let mut direct = Vec::new();
    let mut code_tools = Vec::new();
    let mut rejected = Vec::new();
    for tool in tools {
        if tool.surface() == ToolSurface::Code {
            code_tools.push(tool);
        } else {
            direct.push(tool);
        }
    }

    let mut admitted = Vec::new();
    let mut type_owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for tool in code_tools {
        let name = tool.name().to_string();
        match validate_code_contract(tool.as_ref()) {
            Ok(type_base) => {
                type_owners.entry(type_base).or_default().push(name);
                admitted.push(tool);
            }
            Err(reason) => {
                tracing::warn!(tool = %name, %reason, "code tool failed admission");
                rejected.push(RejectedTool { name, reason });
            }
        }
    }

    let mut collided = Vec::new();
    for (type_base, names) in &type_owners {
        if names.len() < 2 {
            continue;
        }
        let mut listed = names.clone();
        listed.sort();
        let joined = listed.join(", ");
        for name in names {
            let reason = format!(
                "tool '{name}' SDK type '{type_base}' collides with {joined}"
            );
            tracing::warn!(tool = %name, %reason, "code tool failed admission");
            collided.push(name.clone());
            rejected.push(RejectedTool {
                name: name.clone(),
                reason,
            });
        }
    }
    if !collided.is_empty() {
        admitted.retain(|tool| !collided.iter().any(|name| name == tool.name()));
    }

    admitted.sort_by(|left, right| left.name().cmp(right.name()));
    rejected.sort_by(|left, right| left.name.cmp(&right.name));

    let (sdk, whitelist) = if admitted.is_empty() {
        (String::new(), Vec::new())
    } else {
        match render_code_tool_description(&admitted) {
            Ok(sdk) => {
                let whitelist = admitted.iter().map(|tool| tool.name().to_string()).collect();
                (sdk, whitelist)
            }
            Err(reason) => {
                tracing::warn!(%reason, "code surface failed to render");
                for tool in &admitted {
                    rejected.push(RejectedTool {
                        name: tool.name().to_string(),
                        reason: reason.clone(),
                    });
                }
                rejected.sort_by(|left, right| left.name.cmp(&right.name));
                admitted.clear();
                (String::new(), Vec::new())
            }
        }
    };

    ToolProjection {
        direct,
        code_tools: admitted,
        sdk,
        whitelist,
        rejected,
    }
}
