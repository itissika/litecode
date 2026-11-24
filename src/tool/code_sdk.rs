//! Deterministic prompt assembly from JS/TS-owned SDK fragments.
//!
//! This module only selects, orders, and renders fragments produced by
//! `sdk/scripts/generate-contract.mjs`. It never inspects Rust tool schemas.

use std::collections::{BTreeMap, BTreeSet};

use crate::tool::code_contract::{SdkFragments, SdkToolFragment};

const SUITE_ORDER: &[&str] = &[
    "file", "search", "process", "web", "browser", "lsp", "memory", "todo", "plan", "ask_user",
    "knowledge", "workspace", "subagent", "bot",
];

fn sort_fragments<'a>(
    fragments: &'a SdkFragments,
    selected: &BTreeSet<String>,
) -> Result<Vec<&'a SdkToolFragment>, String> {
    let mut entries = selected
        .iter()
        .map(|name| {
            fragments
                .tools
                .get(name)
                .ok_or_else(|| format!("tool '{name}' has no SDK fragment"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by(|left, right| left.path.join(".").cmp(&right.path.join(".")));
    Ok(entries)
}

fn render_doc(doc: &str, indent: &str) -> String {
    let doc = doc.trim();
    if doc.is_empty() {
        return String::new();
    }
    if !doc.contains('\n') {
        return format!("{indent}/** {doc} */\n");
    }
    let mut out = String::new();
    out.push_str(indent);
    out.push_str("/**\n");
    for line in doc.lines() {
        let line = line.trim();
        if line.is_empty() {
            out.push_str(indent);
            out.push_str(" *\n");
        } else {
            out.push_str(indent);
            out.push_str(" * ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(indent);
    out.push_str(" */\n");
    out
}

fn render_tool(tool: &SdkToolFragment, indent: &str) -> String {
    let mut out = render_doc(&tool.doc, indent);
    for line in tool.signature.split('\n') {
        out.push_str(indent);
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn suite_interface_name(suite: &str) -> String {
    let clean: String = suite
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut chars = clean.chars();
    match chars.next() {
        Some(first) => {
            first.to_uppercase().collect::<String>() + chars.as_str() + "Tools"
        }
        None => "Tools".to_string(),
    }
}

fn suite_property_key(suite: &str) -> String {
    let valid = !suite.is_empty()
        && suite.chars().enumerate().all(|(i, c)| {
            (c.is_ascii_alphabetic() || c == '_' || c == '$')
                || (i > 0 && (c.is_ascii_alphanumeric() || c == '_' || c == '$'))
        });
    if valid {
        suite.to_string()
    } else {
        serde_json::to_string(suite).unwrap_or_else(|_| "\"custom\"".to_string())
    }
}

fn render_tools_interface(
    entries: &[&SdkToolFragment],
    local_types: &BTreeMap<String, String>,
) -> Result<String, String> {
    let mut flat_by_suite: BTreeMap<&str, Vec<&SdkToolFragment>> = BTreeMap::new();
    let mut nested_by_suite: BTreeMap<&str, Vec<&SdkToolFragment>> = BTreeMap::new();
    for tool in entries {
        if tool.path.len() == 1 {
            flat_by_suite
                .entry(tool.suite.as_str())
                .or_default()
                .push(*tool);
        } else if tool.path.len() == 2 {
            nested_by_suite
                .entry(tool.path[0].as_str())
                .or_default()
                .push(*tool);
        } else {
            return Err(format!(
                "dynamic SDK paths deeper than one suite level are not supported: {}",
                tool.path.join(".")
            ));
        }
    }

    let mut ordered_suites: Vec<&str> = SUITE_ORDER
        .iter()
        .copied()
        .filter(|suite| nested_by_suite.contains_key(suite))
        .collect();
    let mut custom_suites: Vec<&str> = nested_by_suite
        .keys()
        .copied()
        .filter(|suite| !SUITE_ORDER.contains(suite))
        .collect();
    custom_suites.sort_unstable();
    ordered_suites.extend(custom_suites);

    let mut out = String::new();
    for suite in &ordered_suites {
        if let Some(local) = local_types.get(*suite) {
            out.push_str(local);
            out.push_str("\n\n");
        }
        out.push_str("interface ");
        out.push_str(&suite_interface_name(suite));
        out.push_str(" {\n");
        for tool in nested_by_suite.get(suite).expect("ordered suite") {
            out.push_str(&render_tool(tool, "  "));
        }
        out.push_str("}\n\n");
    }

    // Flat tools have no suite interface; keep any suite-local type block
    // immediately above the top-level interface.
    for suite in flat_by_suite.keys() {
        if ordered_suites.contains(suite) {
            continue;
        }
        if let Some(local) = local_types.get(*suite) {
            out.push_str(local);
            out.push_str("\n\n");
        }
    }

    out.push_str("interface Tools {\n");
    for suite in SUITE_ORDER {
        let Some(group) = flat_by_suite.get(suite) else {
            continue;
        };
        for tool in group {
            out.push_str(&render_tool(tool, "  "));
        }
        out.push('\n');
    }
    for suite in flat_by_suite
        .keys()
        .copied()
        .filter(|suite| !SUITE_ORDER.contains(suite))
    {
        for tool in flat_by_suite.get(suite).expect("flat suite") {
            out.push_str(&render_tool(tool, "  "));
        }
        out.push('\n');
    }
    for suite in &ordered_suites {
        out.push_str("  ");
        out.push_str(&suite_property_key(suite));
        out.push_str(": ");
        out.push_str(&suite_interface_name(suite));
        out.push_str(";\n");
    }
    if ordered_suites.is_empty() && out.ends_with("\n\n") {
        out.pop();
    }
    out.push_str("}\n");
    Ok(out)
}

fn render_ts(fragments: &SdkFragments, entries: &[&SdkToolFragment]) -> Result<String, String> {
    let mut type_names = BTreeSet::new();
    for tool in entries {
        for dep in &tool.type_deps {
            if !fragments.types.contains_key(dep) {
                return Err(format!(
                    "tool '{}' references missing SDK type '{dep}'",
                    tool.path.join(".")
                ));
            }
            type_names.insert(dep.clone());
        }
    }

    let mut global_names = Vec::new();
    let mut local_names: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in type_names {
        let suite = fragments
            .type_suites
            .get(&name)
            .filter(|suite| entries.iter().any(|tool| tool.suite == **suite));
        match suite {
            Some(suite) => local_names.entry(suite.clone()).or_default().push(name),
            None => global_names.push(name),
        }
    }

    let global_types = global_names
        .iter()
        .map(|name| fragments.types.get(name).expect("checked above").as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let local_blocks: BTreeMap<String, String> = local_names
        .into_iter()
        .map(|(suite, names)| {
            let text = names
                .iter()
                .map(|name| fragments.types.get(name).expect("checked above").as_str())
                .collect::<Vec<_>>()
                .join("\n");
            (suite, text)
        })
        .collect();
    let tools = render_tools_interface(entries, &local_blocks)?;
    Ok(format!(
        "{}\n\n{}\n\n{}\n\n{}\n\ndeclare const tools: Tools;\n",
        fragments.declarations.trim(),
        fragments.libs.trim(),
        global_types,
        tools.trim_end(),
    ))
}

const RUN_CODE_SDK_HEADER: &str =
    "Execute an async TypeScript/JavaScript body using the bound SDK below. Call inspect(value) to emit outputs (top-level return values are not supported).";

/// Assemble the prompt-facing SDK for one bound tool set.
pub fn assemble(fragments: &SdkFragments, selected: &BTreeSet<String>) -> Result<String, String> {
    let entries = sort_fragments(fragments, selected)?;
    let ts = render_ts(fragments, &entries)?;
    Ok(format!("{RUN_CODE_SDK_HEADER}\n\n```ts\n{ts}```\n"))
}
