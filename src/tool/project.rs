//! One projection from a caller's visible tools to direct list, code surface, and rejections.
//!
//! Which tool is code comes from [`Tool::surface`](crate::tool::Tool::surface).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde::Serialize;

use crate::tool::code_manifest::{render_code_tool_description, validate_code_contract};
use crate::tool::trait_::Tool;

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
/// Each tool's own surface decides the side. A code tool that cannot produce a
/// complete function spec is removed from both sides and recorded in
/// [`ToolProjection::rejected`].
pub fn project_tools(tools: Vec<Arc<dyn Tool>>) -> ToolProjection {
    let mut direct = Vec::new();
    let mut code_candidates: Vec<(Arc<dyn Tool>, crate::tool::spec::CodeFunctionSpec)> = Vec::new();
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
        match crate::tool::spec::FunctionSpec::from_tool(tool.as_ref()) {
            Ok(crate::tool::spec::FunctionSpec::Direct(_)) => direct.push(tool),
            Ok(crate::tool::spec::FunctionSpec::Code(spec)) => code_candidates.push((tool, spec)),
            Err(reason) => {
                let name = tool.name().to_string();
                tracing::warn!(tool = %name, %reason, "tool failed function-spec admission");
                rejected.push(RejectedTool { name, reason });
            }
        }
    }

    let mut admitted = Vec::new();
    let mut admitted_specs = Vec::new();
    let mut type_owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (tool, spec) in code_candidates {
        match validate_code_contract(&spec) {
            Ok(type_base) => {
                type_owners.entry(type_base).or_default().push(spec.name.clone());
                admitted.push(tool);
                admitted_specs.push(spec);
            }
            Err(reason) => {
                tracing::warn!(tool = %spec.name, %reason, "code tool failed admission");
                rejected.push(RejectedTool {
                    name: spec.name,
                    reason,
                });
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
            let reason = format!("tool '{name}' SDK type '{type_base}' collides with {joined}");
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
        admitted_specs.retain(|spec| !collided.iter().any(|name| name == &spec.name));
    }

    admitted.sort_by(|left, right| left.name().cmp(right.name()));
    admitted_specs.sort_by(|left, right| left.name.cmp(&right.name));
    rejected.sort_by(|left, right| left.name.cmp(&right.name));

    let (sdk, whitelist) = if admitted.is_empty() {
        (String::new(), Vec::new())
    } else {
        match render_code_tool_description(&admitted_specs) {
            Ok(sdk) => {
                let whitelist = admitted
                    .iter()
                    .map(|tool| tool.name().to_string())
                    .collect();
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

/// Rejection-only projection for settings. It validates code contracts and SDK
/// type collisions without instantiating or retaining any Tool adapter.
pub fn project_code_specs(specs: Vec<crate::tool::spec::CodeFunctionSpec>) -> Vec<RejectedTool> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for spec in &specs {
        *counts.entry(spec.name.as_str()).or_default() += 1;
    }
    if let Some(name) = counts
        .iter()
        .filter(|(_, count)| **count > 1)
        .map(|(name, _)| *name)
        .min()
    {
        let reason = format!("tool name '{name}' is declared more than once");
        let mut rejected: Vec<_> = specs
            .iter()
            .map(|spec| RejectedTool {
                name: spec.name.clone(),
                reason: reason.clone(),
            })
            .collect();
        rejected.sort_by(|left, right| left.name.cmp(&right.name));
        return rejected;
    }
    if let Some(reason) = crate::tool::code_manifest::nested_name_collision(
        specs.iter().map(|spec| spec.name.as_str()),
    ) {
        let mut rejected: Vec<_> = specs
            .iter()
            .map(|spec| RejectedTool {
                name: spec.name.clone(),
                reason: reason.clone(),
            })
            .collect();
        rejected.sort_by(|left, right| left.name.cmp(&right.name));
        return rejected;
    }
    let mut rejected = Vec::new();
    let mut type_owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for spec in specs {
        match validate_code_contract(&spec) {
            Ok(type_base) => {
                type_owners.entry(type_base).or_default().push(spec.name.clone());
            }
            Err(reason) => rejected.push(RejectedTool {
                name: spec.name,
                reason,
            }),
        }
    }
    for (type_base, names) in &type_owners {
        if names.len() < 2 {
            continue;
        }
        let mut listed = names.clone();
        listed.sort();
        let joined = listed.join(", ");
        for name in names {
            rejected.push(RejectedTool {
                name: name.clone(),
                reason: format!("tool '{name}' SDK type '{type_base}' collides with {joined}"),
            });
        }
    }
    rejected.sort_by(|left, right| left.name.cmp(&right.name));
    rejected
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::spec::CodeFunctionSpec;
    use serde_json::json;

    fn spec(name: &str) -> CodeFunctionSpec {
        CodeFunctionSpec {
            name: name.into(),
            description: "probe".into(),
            input_schema: json!({ "type": "object", "properties": {} }),
            output_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    #[test]
    fn duplicate_code_names_reject_every_owner() {
        let rejected = project_code_specs(vec![spec("probe"), spec("probe")]);
        assert_eq!(rejected.len(), 2);
        assert!(rejected.iter().all(|item| item.reason.contains("more than once")));
    }

    #[test]
    fn nested_code_names_reject_every_owner() {
        let rejected = project_code_specs(vec![spec("deck"), spec("deck.only")]);
        assert_eq!(rejected.len(), 2);
        assert!(rejected.iter().all(|item| item.reason.contains("nests under")));
    }
}
