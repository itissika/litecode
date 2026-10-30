//! Deterministic prompt assembly from JS/TS-owned SDK fragments.
//!
//! This module only selects, orders, and renders fragments produced by
//! `sdk/scripts/generate-contract.mjs`. It never inspects Rust tool schemas.

use std::collections::BTreeSet;

use crate::tool::code_contract::{SdkFragments, SdkToolFragment};

const SUITE_ORDER: &[&str] = &[
    "file", "search", "process", "web", "memory", "browser", "lsp",
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
    out.push_str(indent);
    out.push_str(&tool.signature);
    out.push('\n');
    out
}

fn render_tools_interface(entries: &[&SdkToolFragment]) -> Result<String, String> {
    let mut out = String::from("interface Tools {\n");
    let flat: Vec<&&SdkToolFragment> = entries.iter().filter(|tool| tool.path.len() == 1).collect();
    let mut flat_by_suite: std::collections::BTreeMap<&str, Vec<&&SdkToolFragment>> =
        std::collections::BTreeMap::new();
    for tool in &flat {
        flat_by_suite
            .entry(tool.suite.as_str())
            .or_default()
            .push(*tool);
    }
    let mut emitted = std::collections::BTreeSet::new();
    for suite in SUITE_ORDER {
        let Some(group) = flat_by_suite.get(suite) else {
            continue;
        };
        emitted.insert(*suite);
        out.push_str("  // ");
        out.push_str(suite);
        out.push('\n');
        for tool in group {
            out.push_str(&render_tool(tool, "  "));
        }
        out.push('\n');
    }
    for (suite, group) in flat_by_suite {
        if emitted.contains(suite) {
            continue;
        }
        out.push_str("  // ");
        out.push_str(suite);
        out.push('\n');
        for tool in group {
            out.push_str(&render_tool(tool, "  "));
        }
        out.push('\n');
    }
    let mut memory: Vec<&&SdkToolFragment> = entries
        .iter()
        .filter(|tool| tool.path.len() == 2 && tool.path[0] == "memory")
        .collect();
    memory.sort_by(|left, right| left.path[1].cmp(&right.path[1]));
    if !memory.is_empty() {
        out.push_str("  // memory\n");
        out.push_str("  memory: {\n");
        for tool in memory {
            out.push_str(&render_tool(tool, "    "));
        }
        out.push_str("  };\n\n");
    }
    // Runtime-defined custom suites are one level deep: suite.member.
    // Render them after the static memory block without changing the built-in
    // memory formatting.
    let mut dynamic_suites: std::collections::BTreeMap<&str, Vec<&&SdkToolFragment>> =
        std::collections::BTreeMap::new();
    for tool in entries {
        if tool.path.len() == 2 && tool.path[0] != "memory" {
            dynamic_suites
                .entry(tool.path[0].as_str())
                .or_default()
                .push(tool);
        } else if tool.path.len() > 2 {
            return Err(format!(
                "dynamic SDK paths deeper than one suite level are not supported: {}",
                tool.path.join(".")
            ));
        }
    }
    for (suite, group) in dynamic_suites {
        out.push_str(&format!("  // {suite}\n"));
        out.push_str(&format!("  {suite}: {{\n"));
        for tool in group {
            out.push_str(&render_tool(tool, "    "));
        }
        out.push_str("  };\n\n");
    }
    while out.ends_with('\n') {
        out.pop();
    }
    out.push_str("\n}\n");
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
    let types = type_names
        .iter()
        .map(|name| fragments.types.get(name).expect("checked above").as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let tools = render_tools_interface(entries)?;
    Ok(format!(
        "{}\n\n{}\n\n{}\n\n{}\n\ndeclare const tools: Tools;\n",
        fragments.declarations.trim(),
        fragments.libs.trim(),
        types,
        tools.trim_end(),
    ))
}

/// Assemble the prompt-facing SDK for one bound tool set.
pub fn assemble(fragments: &SdkFragments, selected: &BTreeSet<String>) -> Result<String, String> {
    let entries = sort_fragments(fragments, selected)?;
    let ts = render_ts(fragments, &entries)?;
    Ok(format!("{}\n\n```ts\n{}```\n", fragments.prose.trim(), ts))
}
