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

    fn effects(
        &self,
        _input: &Value,
        _path_mode: crate::workspace::ToolPathMode,
        workspace_root: &Path,
    ) -> crate::tool::resource_gate::ToolEffects {
        let root = match root::locate(workspace_root) {
            KnowledgeRoot::Present(root) => root,
            KnowledgeRoot::Missing => root::PRIVATE_ROOT.to_string(),
        };
        crate::tool::resource_gate::ToolEffects::write(ResourceKey::File(root))
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
            let full_path = ws_path(&root, &node.path);
            json!({
                "key": node.key,
                "folder": folder_of(&full_path),
                "path": full_path,
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
            rows.push(issue_row(&root, node, &issue));
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
        .map(|issue| issue_row(&root, node, &issue))
        .collect();
    let body = read_node_file(workspace, &root, node);
    let incoming: Vec<Value> = corpus
        .incoming(&node.key)
        .into_iter()
        .map(|edge| ref_edge(&root, edge))
        .collect();
    let mut outgoing: Vec<Value> = Vec::new();
    let mut missing: Vec<String> = node.broken_knowledge_refs.clone();
    for rel in &node.relations {
        match corpus.by_key(rel) {
            Some(target) => outgoing.push(ref_edge(&root, target)),
            None => missing.push(rel.clone()),
        }
    }
    let full_path = ws_path(&root, &node.path);
    let data = json!({
        "key": node.key,
        "folder": folder_of(&full_path),
        "path": full_path,
        "status": status_label(node),
        "summary": node.summary,
        "body": body,
        "incoming": incoming,
        "outgoing": outgoing,
        "missing": missing,
        "issues": card_issues,
    });
    ToolCallResult::ok(format!("Node `{}` ({})", node.key, full_path)).with_data(data)
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
            .map(|folder| folder.trim().trim_end_matches('/'))
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
        let subfolder = folder
            .map(|folder| normalize_folder(&root, folder))
            .filter(|sub| !sub.is_empty());
        let rel = match subfolder {
            Some(sub) => format!("{sub}/{key}.md"),
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
    let folders = parse_string_list(input.get("folders"), "folders")?.map(|items| {
        items
            .into_iter()
            .map(|item| item.trim_end_matches('/').to_string())
            .collect::<Vec<_>>()
    });
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
            let sub = normalize_folder(root, folder);
            let dir = if sub.is_empty() {
                workspace.join(root)
            } else {
                workspace.join(root).join(sub)
            };
            if !dir.is_dir() {
                return Err(ToolCallResult::error(format!(
                    "There is no knowledge folder `{folder}`."
                )));
            }
            for node in corpus.in_folder(sub) {
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

fn ws_path(root: &str, node_path: &str) -> String {
    root::join(root, node_path)
}

fn normalize_folder<'a>(root: &str, folder: &'a str) -> &'a str {
    let folder = folder.trim_end_matches('/');
    if folder == root {
        return "";
    }
    if let Some(rest) = folder.strip_prefix(root).and_then(|s| s.strip_prefix('/')) {
        return rest;
    }
    folder
}

fn folder_of(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(dir, _)| dir.to_string())
        .unwrap_or_default()
}

fn issue_row(root: &str, node: &Node, issue: &Issue) -> Value {
    let full_path = ws_path(root, &node.path);
    json!({
        "key": node.key,
        "folder": folder_of(&full_path),
        "path": full_path,
        "severity": issue.severity.as_str(),
        "code": issue.code,
        "ref_key": issue.ref_key,
        "message": issue.message,
    })
}

fn ref_edge(root: &str, node: &Node) -> Value {
    json!({
        "key": node.key,
        "path": ws_path(root, &node.path),
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
            "folder": ".litecode/knowledge/arch",
            "path": ".litecode/knowledge/arch/seq.md",
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
            "folder": ".litecode/knowledge/arch",
            "path": ".litecode/knowledge/arch/seq.md",
            "severity": "warning",
            "code": "dangling_relation",
            "ref_key": "missing",
            "message": "missing target",
        }],
    }));
    let refs_sample = ToolCallResult::ok("ok").with_data(json!({
        "key": "seq",
        "folder": ".litecode/knowledge/arch",
        "path": ".litecode/knowledge/arch/seq.md",
        "status": "enabled",
        "summary": "Sequence contract",
        "body": "# Seq\n",
        "incoming": [{
            "key": "other",
            "path": ".litecode/knowledge/other.md",
            "status": "enabled",
            "summary": "Other",
        }],
        "outgoing": [{
            "key": "target",
            "path": ".litecode/knowledge/target.md",
            "status": "enabled",
            "summary": "Target",
        }],
        "missing": ["ghost"],
        "issues": [{
            "key": "seq",
            "folder": ".litecode/knowledge/arch",
            "path": ".litecode/knowledge/arch/seq.md",
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
            "```node\nnode : a\nstatus : enabled\nsummary : Alpha A\n```\n\nSee [missing_ref](knowledge/missing_ref.md).\n",
        )
        .unwrap();
        std::fs::write(
            root.join("beta/b.md"),
            "```node\nnode : b\nstatus : pending\nsummary : Beta B\n```\n\nLinks [a](knowledge/alpha/a.md).\n",
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

    #[test]
    fn knowledge_tools_unify_on_workspace_relative_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("knowledge");
        std::fs::create_dir_all(root.join("contracts")).unwrap();
        std::fs::create_dir_all(root.join("arch")).unwrap();
        std::fs::write(
            root.join("overview.md"),
            "```node\nnode : overview\nstatus : enabled\nsummary : Overview\n```\n\nSee [auth-flow](knowledge/contracts/auth-flow.md).\n",
        )
        .unwrap();
        std::fs::write(
            root.join("contracts/auth-flow.md"),
            "```node\nnode : auth-flow\nstatus : enabled\nsummary : Auth\n```\n\nCites [missing](knowledge/contracts/missing.md) and [session-store](knowledge/arch/session-store.md).\n",
        )
        .unwrap();
        std::fs::write(
            root.join("arch/session-store.md"),
            "```node\nnode : session-store\nstatus : enabled\nsummary : Store\n```\n",
        )
        .unwrap();

        let list_all = run_list(&json!({}), dir.path()).data.unwrap();
        let paths: Vec<&str> = list_all["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["path"].as_str().unwrap())
            .collect();
        assert_eq!(
            paths,
            vec![
                "knowledge/arch/session-store.md",
                "knowledge/contracts/auth-flow.md",
                "knowledge/overview.md",
            ]
        );

        let by_ws_folder = run_list(&json!({ "folders": ["knowledge/contracts"] }), dir.path())
            .data
            .unwrap();
        assert_eq!(by_ws_folder["total"], 1);
        assert_eq!(
            by_ws_folder["nodes"][0]["path"],
            "knowledge/contracts/auth-flow.md"
        );

        let by_root_folder = run_list(&json!({ "folders": ["knowledge/"] }), dir.path())
            .data
            .unwrap();
        assert_eq!(by_root_folder["total"], 3);

        let check_ws = run_check(&json!({ "folders": ["knowledge/contracts"] }), dir.path())
            .data
            .unwrap();
        assert_eq!(check_ws["issue_count"], 1);
        assert_eq!(
            check_ws["issues"][0]["path"],
            "knowledge/contracts/auth-flow.md"
        );

        let refs = run_refs(&json!({ "key": "auth-flow" }), dir.path())
            .data
            .unwrap();
        assert_eq!(refs["folder"], "knowledge/contracts");
        assert_eq!(refs["path"], "knowledge/contracts/auth-flow.md");
        assert_eq!(refs["incoming"][0]["path"], "knowledge/overview.md");
        assert_eq!(refs["outgoing"][0]["path"], "knowledge/arch/session-store.md");
        assert_eq!(refs["issues"][0]["path"], "knowledge/contracts/auth-flow.md");

        let overview_refs = run_refs(&json!({ "key": "overview" }), dir.path())
            .data
            .unwrap();
        assert_eq!(overview_refs["folder"], "knowledge");
        assert_eq!(overview_refs["path"], "knowledge/overview.md");
    }

    #[tokio::test]
    async fn knowledge_create_accepts_workspace_or_relative_folder_without_double_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let kroot = dir.path().join("knowledge");
        std::fs::create_dir_all(kroot.join("contracts")).unwrap();

        let engines = std::sync::Arc::new(crate::engines::WorkspaceEngines::new());
        let ide = crate::ide_base::IdeBaseHandle::open(dir.path().to_path_buf(), engines).unwrap();
        let tool = KnowledgeCreateTool::new(ide);

        let a = tool.run_create(&json!({ "key": "rate-limit", "folder": "knowledge/contracts" }), dir.path());
        assert!(a.error.is_none(), "{}", a.content);
        assert_eq!(a.data.as_ref().unwrap()["path"], "knowledge/contracts/rate-limit.md");
        assert!(dir.path().join("knowledge/contracts/rate-limit.md").is_file());
        assert!(!dir.path().join("knowledge/knowledge").exists());

        let b = tool.run_create(&json!({ "key": "top-note", "folder": "knowledge" }), dir.path());
        assert!(b.error.is_none(), "{}", b.content);
        assert_eq!(b.data.as_ref().unwrap()["path"], "knowledge/top-note.md");
        assert!(dir.path().join("knowledge/top-note.md").is_file());

        let c = tool.run_create(&json!({ "key": "sub-note", "folder": "contracts" }), dir.path());
        assert!(c.error.is_none(), "{}", c.content);
        assert_eq!(c.data.as_ref().unwrap()["path"], "knowledge/contracts/sub-note.md");
        assert!(dir.path().join("knowledge/contracts/sub-note.md").is_file());
    }
}
