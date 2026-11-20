//! Workspace knowledge tools (`knowledge.list`, `knowledge.check`, `knowledge.refs`, `knowledge.create`).
//!
//! Reads rebuild the corpus from disk. Writes go through the workspace service
//! so the editor and knowledge panel observe changes immediately.

use std::collections::HashSet;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::knowledge::corpus::{Corpus, Node};
use crate::knowledge::document::{self, Status};
use crate::knowledge::mentions::{is_knowledge_key, normalize_key};
use crate::knowledge::root::{self, KnowledgeRoot};
use crate::knowledge::validate::Issue;
use crate::tool::trait_::{Tool, ToolExecutionContext};
use crate::tool::write_lock::ResourceKey;
use crate::types::ToolCallResult;

pub struct KnowledgeListTool;

impl KnowledgeListTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for KnowledgeListTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for KnowledgeListTool {
    fn name(&self) -> &str {
        "knowledge.list"
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(std::future::ready(run_list(&input, &execution.workspace_root)))
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        ToolCallResult::error("knowledge.list requires an active workspace execution context")
    }
}

pub struct KnowledgeCheckTool;

impl KnowledgeCheckTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for KnowledgeCheckTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for KnowledgeCheckTool {
    fn name(&self) -> &str {
        "knowledge.check"
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(std::future::ready(run_check(&input, &execution.workspace_root)))
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        ToolCallResult::error("knowledge.check requires an active workspace execution context")
    }
}

pub struct KnowledgeRefsTool;

impl KnowledgeRefsTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for KnowledgeRefsTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for KnowledgeRefsTool {
    fn name(&self) -> &str {
        "knowledge.refs"
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(std::future::ready(run_refs(&input, &execution.workspace_root)))
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        ToolCallResult::error("knowledge.refs requires an active workspace execution context")
    }
}

pub struct KnowledgeCreateTool {
    ide: Option<Arc<crate::ide_base::IdeBaseHandle>>,
}

impl KnowledgeCreateTool {
    pub fn new(ide: Arc<crate::ide_base::IdeBaseHandle>) -> Self {
        Self { ide: Some(ide) }
    }

    pub fn cold() -> Self {
        Self { ide: None }
    }
}

impl Tool for KnowledgeCreateTool {
    fn name(&self) -> &str {
        "knowledge.create"
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn resource_keys(
        &self,
        _input: &Value,
        _path_mode: crate::workspace::ToolPathMode,
        workspace_root: &Path,
    ) -> Vec<ResourceKey> {
        let root = match root::locate(workspace_root) {
            KnowledgeRoot::Present(root) => root,
            KnowledgeRoot::Missing => root::PRIVATE_ROOT.to_string(),
        };
        vec![ResourceKey::File(root)]
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(std::future::ready(self.run_create(&input, &execution.workspace_root)))
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        ToolCallResult::error("knowledge.create requires an active workspace execution context")
    }
}

fn run_list(input: &Value, workspace: &Path) -> ToolCallResult {
    let scope = match parse_scope(input) {
        Ok(scope) => scope,
        Err(err) => return ToolCallResult::error(err),
    };
    let corpus = Corpus::load(workspace);
    let Some(root) = corpus.root.clone() else {
        return ToolCallResult::ok("No knowledge directory in this workspace.").with_data(json!({
            "root": null,
            "total": 0,
            "nodes": [],
        }));
    };
    let selected = match select_nodes(&corpus, workspace, &root, &scope) {
        Ok(selected) => selected,
        Err(err) => return err,
    };
    let issues = issues_of(workspace, &corpus);
    let rows: Vec<Value> = selected
        .iter()
        .map(|node| {
            let issue_count = node_issues(&issues, &node.id).len();
            let incoming_count = corpus.incoming(&node.key).len();
            json!({
                "key": node.key,
                "folder": folder_of(&node.path),
                "path": node.path,
                "status": status_label(node),
                "incoming_count": incoming_count,
                "issue_count": issue_count,
                "summary": node.summary,
            })
        })
        .collect();
    let total = rows.len();
    ToolCallResult::ok(format!("Listed {total} knowledge node(s) in `{root}`.")).with_data(json!({
        "root": root,
        "total": total,
        "nodes": rows,
    }))
}

fn run_check(input: &Value, workspace: &Path) -> ToolCallResult {
    let scope = match parse_scope(input) {
        Ok(scope) => scope,
        Err(err) => return ToolCallResult::error(err),
    };
    let corpus = Corpus::load(workspace);
    let Some(root) = corpus.root.clone() else {
        return ToolCallResult::ok("No knowledge directory in this workspace.").with_data(json!({
            "root": null,
            "checked": 0,
            "issue_count": 0,
            "issues": [],
        }));
    };
    let selected = match select_nodes(&corpus, workspace, &root, &scope) {
        Ok(selected) => selected,
        Err(err) => return err,
    };
    let checked = selected.len();
    let issues = issues_of(workspace, &corpus);
    let mut rows: Vec<Value> = Vec::new();
    for node in selected {
        for issue in node_issues(&issues, &node.id) {
            rows.push(issue_row(node, &issue));
        }
    }
    let issue_count = rows.len();
    ToolCallResult::ok(format!(
        "Checked {checked} knowledge node(s); {issue_count} issue(s)."
    ))
    .with_data(json!({
        "root": root,
        "checked": checked,
        "issue_count": issue_count,
        "issues": rows,
    }))
}

fn run_refs(input: &Value, workspace: &Path) -> ToolCallResult {
    let raw_key = match crate::tool::require_nonempty_string(input, "key") {
        Ok(key) => key,
        Err(err) => return ToolCallResult::error(err),
    };
    let corpus = Corpus::load(workspace);
    let Some(root) = corpus.root.clone() else {
        return ToolCallResult::error("No knowledge directory in this workspace.");
    };
    let key = normalize_key(raw_key);
    let Some(node) = corpus.by_key(&key) else {
        return ToolCallResult::error(format!("There is no knowledge node `{key}`."));
    };
    let issues = issues_of(workspace, &corpus);
    let card_issues: Vec<Value> = node_issues(&issues, &node.id)
        .into_iter()
        .map(|issue| issue_row(node, &issue))
        .collect();
    let body = read_node_file(workspace, &root, node);
    let incoming: Vec<Value> = corpus
        .incoming(&node.key)
        .into_iter()
        .map(ref_edge)
        .collect();
    let mut outgoing: Vec<Value> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for rel in &node.relations {
        match corpus.by_key(rel) {
            Some(target) => outgoing.push(ref_edge(target)),
            None => missing.push(rel.clone()),
        }
    }
    let data = json!({
        "key": node.key,
        "folder": folder_of(&node.path),
        "path": node.path,
        "status": status_label(node),
        "summary": node.summary,
        "body": body,
        "incoming": incoming,
        "outgoing": outgoing,
        "missing": missing,
        "issues": card_issues,
    });
    ToolCallResult::ok(format!("Node `{}` ({})", node.key, node.path)).with_data(data)
}

impl KnowledgeCreateTool {
    fn run_create(&self, input: &Value, workspace: &Path) -> ToolCallResult {
        let raw_key = match crate::tool::require_nonempty_string(input, "key") {
            Ok(key) => key,
            Err(err) => return ToolCallResult::error(err),
        };
        let key = normalize_key(raw_key);
        if key.is_empty() || !is_knowledge_key(&key) {
            return ToolCallResult::error(format!("`{key}` is not a valid knowledge node key"));
        }
        let folder = input
            .get("folder")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|folder| !folder.is_empty());
        if let Some(folder) = folder
            && !folder_ok(folder)
        {
            return ToolCallResult::error("A folder cannot contain `.`, `..`, or `\\`.");
        }
        let corpus = Corpus::load(workspace);
        if corpus.by_key(&key).is_some() {
            return ToolCallResult::error(format!("knowledge node `{key}` already exists"));
        }
        let root = match root::locate(workspace) {
            KnowledgeRoot::Present(root) => root,
            KnowledgeRoot::Missing => root::PRIVATE_ROOT.to_string(),
        };
        let rel = match folder {
            Some(folder) => format!("{folder}/{key}.md"),
            None => format!("{key}.md"),
        };
        let full = root::join(&root, &rel);
        if workspace.join(&full).exists() {
            return ToolCallResult::error(format!("`{full}` already exists"));
        }
        let markdown = document::render_knowledge_markdown(document::RenderDoc {
            key: &key,
            status: Status::Pending,
            invalid_status: None,
            body: "",
            summary: "",
            x: None,
            y: None,
            w: None,
            h: None,
            extras: &[],
        });
        let Some(ide) = &self.ide else {
            return ToolCallResult::error("knowledge.create requires an active runtime");
        };
        if let Err(error) = ide.workspace.write_file(&full, &markdown) {
            return ToolCallResult::error(format!("could not write `{full}`: {error}"));
        }
        let _ = Corpus::load(workspace);
        ToolCallResult::ok(format!(
            "Created pending knowledge node `{full}`. Add summary/body and verify with `tools.knowledge.check`."
        ))
        .with_data(json!({
            "key": key,
            "path": full,
            "status": "pending",
        }))
    }
}

struct ScopeFilter {
    folders: Option<Vec<String>>,
    nodes: Option<Vec<String>>,
}

fn parse_scope(input: &Value) -> Result<ScopeFilter, String> {
    let folders = parse_string_list(input.get("folders"), "folders")?;
    if let Some(list) = &folders {
        for folder in list {
            if !folder_ok(folder) {
                return Err(format!("invalid folder `{folder}` (cannot contain . or ..)"));
            }
        }
    }
    let nodes = parse_string_list(input.get("nodes"), "nodes")?.map(|items| {
        items
            .into_iter()
            .map(|item| normalize_key(&item))
            .filter(|k| !k.is_empty())
            .collect::<Vec<_>>()
    });
    Ok(ScopeFilter {
        folders: folders.filter(|v| !v.is_empty()),
        nodes: nodes.filter(|v| !v.is_empty()),
    })
}

fn parse_string_list(value: Option<&Value>, field: &str) -> Result<Option<Vec<String>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let arr = value
        .as_array()
        .ok_or_else(|| crate::tool::expected_type(field, "array", value))?;
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        let s = item
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("{field}[{idx}] must be a non-empty string"))?;
        out.push(s.to_string());
    }
    Ok(Some(out))
}

fn select_nodes<'a>(
    corpus: &'a Corpus,
    workspace: &Path,
    root: &str,
    scope: &ScopeFilter,
) -> Result<Vec<&'a Node>, ToolCallResult> {
    if scope.folders.is_none() && scope.nodes.is_none() {
        return Ok(corpus.nodes.iter().collect());
    }
    let mut matched_paths = HashSet::<String>::new();
    if let Some(folders) = &scope.folders {
        for folder in folders {
            if !workspace.join(root).join(folder).is_dir() {
                return Err(ToolCallResult::error(format!(
                    "There is no knowledge folder `{folder}`."
                )));
            }
            for node in corpus.in_folder(folder) {
                matched_paths.insert(node.path.clone());
            }
        }
    }
    if let Some(nodes) = &scope.nodes {
        for key in nodes {
            let Some(node) = corpus.by_key(key) else {
                return Err(ToolCallResult::error(format!(
                    "There is no knowledge node `{key}`."
                )));
            };
            matched_paths.insert(node.path.clone());
        }
    }
    Ok(corpus
        .nodes
        .iter()
        .filter(|node| matched_paths.contains(&node.path))
        .collect())
}

fn status_label(node: &Node) -> &'static str {
    if node.invalid_status.is_some() {
        "invalid"
    } else {
        node.status.as_str()
    }
}

fn folder_of(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(dir, _)| dir.to_string())
        .unwrap_or_default()
}

fn issue_row(node: &Node, issue: &Issue) -> Value {
    json!({
        "key": node.key,
        "folder": folder_of(&node.path),
        "path": node.path,
        "severity": issue.severity.as_str(),
        "code": issue.code,
        "ref_key": issue.ref_key,
        "message": issue.message,
    })
}

fn ref_edge(node: &Node) -> Value {
    json!({
        "key": node.key,
        "path": node.path,
        "status": status_label(node),
        "summary": node.summary,
    })
}

fn issues_of(workspace: &Path, _corpus: &Corpus) -> Vec<Issue> {
    crate::knowledge::index::fresh_content(workspace)
        .issues
        .clone()
}

fn read_node_file(workspace: &Path, root: &str, node: &Node) -> String {
    let path = workspace.join(root).join(&node.path);
    if let Ok(text) = std::fs::read_to_string(&path) {
        return text;
    }
    document::render_knowledge_markdown(document::RenderDoc {
        key: &node.key,
        status: node.status,
        invalid_status: node.invalid_status.as_deref(),
        body: &node.value,
        summary: &node.summary,
        x: node.x,
        y: node.y,
        w: node.w,
        h: node.h,
        extras: &node.extras,
    })
}

fn node_issues(issues: &[Issue], id: &str) -> Vec<Issue> {
    issues
        .iter()
        .filter(|issue| issue.node_id == id)
        .cloned()
        .collect()
}

fn folder_ok(folder: &str) -> bool {
    !folder.is_empty()
        && !folder.contains('\\')
        && folder
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

#[cfg(test)]
pub(crate) fn drift_cases() -> Vec<crate::tool::code_contract::DriftCase> {
    use crate::tool::code_contract::DriftCase;
    let list_sample = ToolCallResult::ok("ok").with_data(json!({
        "root": ".litecode/knowledge",
        "total": 1,
        "nodes": [{
            "key": "seq",
            "folder": "arch",
            "path": "arch/seq.md",
            "status": "enabled",
            "incoming_count": 2,
            "issue_count": 0,
            "summary": "Sequence contract",
        }],
    }));
    let check_sample = ToolCallResult::ok("ok").with_data(json!({
        "root": ".litecode/knowledge",
        "checked": 1,
        "issue_count": 1,
        "issues": [{
            "key": "seq",
            "folder": "arch",
            "path": "arch/seq.md",
            "severity": "warning",
            "code": "dangling_relation",
            "ref_key": "missing",
            "message": "missing target",
        }],
    }));
    let refs_sample = ToolCallResult::ok("ok").with_data(json!({
        "key": "seq",
        "folder": "arch",
        "path": "arch/seq.md",
        "status": "enabled",
        "summary": "Sequence contract",
        "body": "# Seq\n",
        "incoming": [{
            "key": "other",
            "path": "other.md",
            "status": "enabled",
            "summary": "Other",
        }],
        "outgoing": [{
            "key": "target",
            "path": "target.md",
            "status": "enabled",
            "summary": "Target",
        }],
        "missing": ["ghost"],
        "issues": [{
            "key": "seq",
            "folder": "arch",
            "path": "arch/seq.md",
            "severity": "warning",
            "code": "dangling_relation",
            "ref_key": "ghost",
            "message": "missing target",
        }],
    }));
    let create_sample = ToolCallResult::ok("created").with_data(json!({
        "key": "seq",
        "path": ".litecode/knowledge/arch/seq.md",
        "status": "pending",
    }));
    vec![
        DriftCase {
            tool: "knowledge.list",
            all_optional_present: list_sample.clone(),
            all_optional_absent: list_sample,
        },
        DriftCase {
            tool: "knowledge.check",
            all_optional_present: check_sample.clone(),
            all_optional_absent: check_sample,
        },
        DriftCase {
            tool: "knowledge.refs",
            all_optional_present: refs_sample.clone(),
            all_optional_absent: refs_sample,
        },
        DriftCase {
            tool: "knowledge.create",
            all_optional_present: create_sample.clone(),
            all_optional_absent: create_sample,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_and_check_scope_supports_folders_and_nodes_union() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".litecode").join("knowledge");
        std::fs::create_dir_all(root.join("alpha")).unwrap();
        std::fs::create_dir_all(root.join("beta")).unwrap();
        std::fs::write(
            root.join("alpha/a.md"),
            "```node\nnode : a\nstatus : enabled\nsummary : Alpha A\n```\n\nSee [@ key=\"missing_ref\"].\n",
        )
        .unwrap();
        std::fs::write(
            root.join("beta/b.md"),
            "```node\nnode : b\nstatus : pending\nsummary : Beta B\n```\n\nLinks [@ key=\"a\"].\n",
        )
        .unwrap();

        let list_res = run_list(&json!({ "folders": ["alpha"], "nodes": ["b"] }), dir.path());
        let data = list_res.data.expect(&list_res.content);
        assert_eq!(data["total"], 2);
        let nodes = data["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 2);

        let check_res = run_check(&json!({ "folders": ["alpha"] }), dir.path());
        let cdata = check_res.data.expect("check data");
        assert_eq!(cdata["checked"], 1);
        assert!(cdata["issue_count"].as_u64().unwrap() >= 1);
        let first_issue = &cdata["issues"][0];
        assert!(first_issue["severity"].is_string());
        assert!(!first_issue.as_object().unwrap().values().any(|v| v.is_object() || v.is_array()));
    }
}
