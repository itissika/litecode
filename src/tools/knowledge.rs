//! One knowledge tool. The command string is the whole interface.
//! Reads rebuild the corpus from disk. Writes go through the workspace
//! service so the editor and the knowledge panel see the change.

use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::knowledge::corpus::Corpus;
use crate::knowledge::document::{self, Status};
use crate::knowledge::mentions::{self, is_knowledge_key, normalize_key};
use crate::knowledge::root::{self, KnowledgeRoot};
use crate::knowledge::validate::{self, Issue};
use crate::knowledge::{reminder, view};
use crate::tool::trait_::{Tool, ToolExecutionContext};
use crate::tool::write_lock::ResourceKey;
use crate::types::ToolCallResult;

const DESCRIPTION: &str = "\
Workspace knowledge base: human-owned notes of durable ideas — architecture, principles, workflows, goals. \
Call with an empty command first; the board shows state and what to do next. \
Navigate by key, edit bodies with read/edit, and run check after changes. \
Rename and create through this tool so citations keep resolving. \
Record invariants, not details.";

pub struct KnowledgeTool {
    ide: Arc<crate::ide_base::IdeBaseHandle>,
}

impl KnowledgeTool {
    pub fn new(ide: Arc<crate::ide_base::IdeBaseHandle>) -> Self {
        Self { ide }
    }

    fn run(&self, input: &Value, workspace: &Path) -> ToolCallResult {
        let command = match input.get("command") {
            None => String::new(),
            Some(value) => match value.as_str() {
                Some(text) => text.to_string(),
                None => {
                    return usage("command must be a string", "list [folder]", "list notes");
                }
            },
        };
        let mut tokens = match tokenize(&command) {
            Ok(tokens) => tokens,
            Err(problem) => return usage(&problem, "refs <key>", "refs \"old key\""),
        };
        if tokens.first().map(String::as_str) == Some("knowledge") {
            tokens.remove(0);
        }
        let verb = tokens.first().map(String::as_str).unwrap_or("");
        let args = if tokens.is_empty() {
            &[][..]
        } else {
            &tokens[1..]
        };
        match verb {
            "" => self.board(workspace),
            "guide" => guide(args),
            "list" => self.list(workspace, args),
            "refs" => self.refs(workspace, args),
            "check" => self.check(workspace, args),
            "create" => self.create(workspace, args),
            "rename" => self.rename(workspace, args),
            other => unknown_command(other),
        }
    }

    fn board(&self, workspace: &Path) -> ToolCallResult {
        let corpus = Corpus::load(workspace);
        if corpus.root.is_none() || corpus.nodes.is_empty() {
            return ToolCallResult::ok(view::missing_board());
        }
        let issues = issues_of(workspace, &corpus);
        let now = SystemTime::now();
        let reminder = reminder::section(workspace, &corpus);
        ToolCallResult::ok(view::dashboard(
            &corpus.nodes,
            &issues,
            |key| corpus.incoming(key).len(),
            reminder.as_deref(),
            now,
            corpus.root.as_deref(),
            &corpus.unknown,
        ))
    }

    fn list(&self, workspace: &Path, args: &[String]) -> ToolCallResult {
        if args.len() > 1 {
            return too_many("list [folder]", "list \"my folder\"");
        }
        let corpus = Corpus::load(workspace);
        let Some(root) = corpus.root.clone() else {
            return ToolCallResult::ok(view::missing_board());
        };
        let folder = args.first().map(String::as_str);
        let nodes = if let Some(folder) = folder {
            if !folder_ok(folder) {
                return usage(
                    "A folder cannot contain . or ..",
                    "list [folder]",
                    "list notes",
                );
            }
            if !workspace.join(&root).join(folder).is_dir() {
                return usage(
                    &format!("There is no folder `{folder}`"),
                    "list [folder]",
                    "list notes",
                );
            }
            corpus.in_folder(folder)
        } else {
            corpus.nodes.iter().collect()
        };
        if nodes.is_empty() {
            let title = match folder {
                Some(folder) => format!("# List · {folder}"),
                None => "# List".into(),
            };
            let body = if corpus.nodes.is_empty() {
                let named = match folder {
                    Some(folder) => format!("`{folder}` has no nodes. "),
                    None => String::new(),
                };
                format!(
                    "{named}There are no nodes yet. Create the first with `knowledge create \"<key>\" [folder]`."
                )
            } else {
                format!("`{}` has no nodes.", folder.unwrap_or(""))
            };
            return ToolCallResult::ok(format!("{title}\n\n{body}"));
        }
        let issues = issues_of(workspace, &corpus);
        let now = SystemTime::now();
        let body = view::folder_tree(
            nodes,
            folder,
            |node| node_issues(&issues, &node.id),
            |key| corpus.incoming(key).len(),
            now,
            &root,
        );
        let title = match folder {
            Some(folder) => format!("# List · {folder}"),
            None => "# List".into(),
        };
        ToolCallResult::ok(paginate(workspace, &format!("{title}\n\n{body}")))
    }

    fn refs(&self, workspace: &Path, args: &[String]) -> ToolCallResult {
        if args.is_empty() {
            return usage("refs needs one key", "refs <key>", "refs seq");
        }
        if args.len() > 1 {
            return too_many("refs <key>", "refs \"old key\"");
        }
        let corpus = Corpus::load(workspace);
        let Some(root) = corpus.root.clone() else {
            return ToolCallResult::ok(view::missing_board());
        };
        let key = normalize_key(&args[0]);
        let Some(node) = corpus.by_key(&key) else {
            return usage(
                &format!("There is no node `{key}`"),
                "refs <key>",
                "refs seq",
            );
        };
        let issues = issues_of(workspace, &corpus);
        let now = SystemTime::now();
        let card_issues = node_issues(&issues, &node.id);
        let markdown = read_node_file(workspace, &root, node);
        let document = view::node_file(&root, &node.path, &card_issues, &markdown);
        let incoming = corpus.incoming(&node.key);
        let incoming_body = view::flat_rows(
            incoming.iter().copied(),
            |item| node_issues(&issues, &item.id),
            |item| corpus.incoming(item).len(),
            now,
            &root,
        );
        let mut outgoing = Vec::new();
        let mut missing = Vec::new();
        for rel in &node.relations {
            match corpus.by_key(rel) {
                Some(target) => outgoing.push(target),
                None => missing.push(rel.clone()),
            }
        }
        let outgoing_body = view::flat_rows(
            outgoing,
            |item| node_issues(&issues, &item.id),
            |item| corpus.incoming(item).len(),
            now,
            &root,
        );
        let missing_body = missing
            .iter()
            .filter(|rel| {
                !card_issues.iter().any(|issue| {
                    issue.code == "dangling_relation"
                        && issue.reference.as_deref() == Some(rel.as_str())
                })
            })
            .map(|rel| format!("- `{rel}` does not exist"))
            .collect::<Vec<_>>()
            .join("\n");
        let outgoing_body = [outgoing_body, missing_body]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let follow = if card_issues.is_empty() {
            String::new()
        } else {
            "Edit the body with `edit`. Rename with `knowledge rename`.".into()
        };
        ToolCallResult::ok(join_blocks(&[
            format!("# {key}\n\n{document}"),
            view::section("## Cited by", &incoming_body),
            view::section("## Cites", &outgoing_body),
            follow,
        ]))
    }

    fn check(&self, workspace: &Path, args: &[String]) -> ToolCallResult {
        if args.len() > 1 {
            return too_many("check [key or folder]", "check \"old key\"");
        }
        let corpus = Corpus::load(workspace);
        let Some(root) = corpus.root.clone() else {
            return ToolCallResult::ok(view::missing_board());
        };
        let issues = issues_of(workspace, &corpus);
        let now = SystemTime::now();
        let mut tree_scope: Option<&str> = None;
        let selected: Vec<&crate::knowledge::Node> = if let Some(scope) = args.first() {
            if !folder_ok(scope) {
                return usage(
                    "A scope cannot contain . or ..",
                    "check [key or folder]",
                    "check notes",
                );
            }
            if let Some(node) = corpus.by_key(scope) {
                vec![node]
            } else if workspace.join(&root).join(scope).is_dir() {
                tree_scope = Some(scope.as_str());
                corpus.in_folder(scope)
            } else {
                return usage(
                    &format!("There is no node or folder `{scope}`"),
                    "check [key or folder]",
                    "check notes",
                );
            }
        } else {
            corpus.nodes.iter().collect()
        };
        let checked = selected.len();
        let bad: Vec<_> = selected
            .into_iter()
            .filter(|node| !node_issues(&issues, &node.id).is_empty())
            .collect();
        let summary = check_summary(checked, args.first().map(String::as_str), bad.len());
        if bad.is_empty() {
            return ToolCallResult::ok(format!("# Check\n\n{summary}"));
        }
        let body = view::folder_tree(
            bad,
            tree_scope,
            |node| node_issues(&issues, &node.id),
            |key| corpus.incoming(key).len(),
            now,
            &root,
        );
        ToolCallResult::ok(paginate(
            workspace,
            &format!("# Check\n\n{summary}\n\n{body}"),
        ))
    }

    fn create(&self, workspace: &Path, args: &[String]) -> ToolCallResult {
        if args.is_empty() {
            return usage(
                "create needs a key. The folder is optional.",
                "create <key> [folder]",
                "create seq notes",
            );
        }
        if args.len() > 2 {
            return too_many("create <key> [folder]", "create \"my key\" notes");
        }
        let key = normalize_key(&args[0]);
        if key.is_empty() {
            return usage(
                "A key is required.",
                "create <key> [folder]",
                "create seq notes",
            );
        }
        if !is_knowledge_key(&key) {
            return usage(
                &format!("`{key}` is not a valid node key"),
                "create <key> [folder]",
                "create seq notes",
            );
        }
        let folder = args
            .get(1)
            .map(String::as_str)
            .filter(|folder| !folder.is_empty());
        if let Some(folder) = folder {
            if !folder_ok(folder) {
                return usage(
                    "A folder cannot contain . or ..",
                    "create <key> [folder]",
                    "create seq notes",
                );
            }
        }
        let corpus = Corpus::load(workspace);
        if corpus.by_key(&key).is_some() {
            return usage(
                &format!("`{key}` already exists"),
                "create <key> [folder]",
                "create seq notes",
            );
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
            return usage(
                &format!("`{full}` already exists"),
                "create <key> [folder]",
                "create seq notes",
            );
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
        if let Err(error) = self.ide.workspace.write_file(&full, &markdown) {
            return ToolCallResult::error(format!("# Could not write\n\n{error}"));
        }
        let next = format!(
            "Status is pending. The user enables it in the knowledge panel. Next, use `edit` on `{full}` to add a summary and a body, then run `knowledge check {}`.",
            quote_key(&key)
        );
        let corpus = Corpus::load(workspace);
        let issues = issues_of(workspace, &corpus);
        let now = SystemTime::now();
        let Some(node) = corpus.by_key(&key) else {
            return ToolCallResult::ok(format!("# Created\n\nWrote `{full}`.\n\n{next}"));
        };
        let card = view::node_row(
            node,
            &node_issues(&issues, &node.id),
            corpus.incoming(&node.key).len(),
            now,
            &root,
        );
        ToolCallResult::ok(format!("# Created\n\n{card}\n\n{next}"))
    }

    fn rename(&self, workspace: &Path, args: &[String]) -> ToolCallResult {
        if args.len() < 2 {
            return usage(
                "rename needs the old key and the new key",
                "rename <old key> <new key>",
                "rename seq order",
            );
        }
        if args.len() > 2 {
            return too_many(
                "rename <old key> <new key>",
                "rename \"old key\" \"new key\"",
            );
        }
        let from = normalize_key(&args[0]);
        let to = normalize_key(&args[1]);
        if !is_knowledge_key(&from) || !is_knowledge_key(&to) {
            return usage(
                "Both keys must be valid",
                "rename <old key> <new key>",
                "rename seq order",
            );
        }
        let corpus = Corpus::load(workspace);
        let Some(root) = corpus.root.clone() else {
            return ToolCallResult::ok(view::missing_board());
        };
        let Some(current) = corpus.by_key(&from) else {
            return usage(
                &format!("There is no node `{from}`"),
                "rename <old key> <new key>",
                "rename seq order",
            );
        };
        let current_id = current.id.clone();
        let current_path = current.path.clone();
        if from == to {
            let issues = issues_of(workspace, &corpus);
            let card = view::node_row(
                current,
                &node_issues(&issues, &current.id),
                corpus.incoming(&current.key).len(),
                SystemTime::now(),
                &root,
            );
            return ToolCallResult::ok(format!("# Unchanged\n\n{card}"));
        }
        if corpus.by_key(&to).is_some() {
            return usage(
                &format!("`{to}` already exists"),
                "rename <old key> <new key>",
                "rename seq order",
            );
        }
        let new_rel = renamed_rel(&current_path, &to);
        if new_rel != current_path && workspace.join(root::join(&root, &new_rel)).exists() {
            return usage(
                &format!("`{new_rel}` already exists"),
                "rename <old key> <new key>",
                "rename seq order",
            );
        }
        let mut writes: Vec<(String, String)> = Vec::new();
        for node in &corpus.nodes {
            let body = mentions::replace_mention_key(&node.value, &from, &to);
            if node.id == current_id {
                writes.push((
                    new_rel.clone(),
                    document::render_knowledge_markdown(document::RenderDoc {
                        key: &to,
                        status: node.status,
                        invalid_status: node.invalid_status.as_deref(),
                        body: &body,
                        summary: &node.summary,
                        x: node.x,
                        y: node.y,
                        w: node.w,
                        h: node.h,
                        extras: &node.extras,
                    }),
                ));
            } else if body != node.value {
                writes.push((
                    node.path.clone(),
                    document::render_knowledge_markdown(document::RenderDoc {
                        key: &node.key,
                        status: node.status,
                        invalid_status: node.invalid_status.as_deref(),
                        body: &body,
                        summary: &node.summary,
                        x: node.x,
                        y: node.y,
                        w: node.w,
                        h: node.h,
                        extras: &node.extras,
                    }),
                ));
            }
        }
        let mut wrote = Vec::new();
        for (rel, content) in &writes {
            let full = root::join(&root, rel);
            if let Err(error) = self.ide.workspace.write_file(&full, content) {
                return ToolCallResult::error(format!(
                    "# Stopped partway\n\n{error}{}",
                    already_wrote(&wrote)
                ));
            }
            wrote.push(full);
        }
        let mut deleted = Vec::new();
        if new_rel != current_path {
            let old = root::join(&root, &current_path);
            if let Err(error) = self.ide.workspace.delete_path(&old, false) {
                return ToolCallResult::error(format!(
                    "# The new file is written and the old file is still there\n\nCould not delete `{old}`: {error}{}",
                    already_wrote(&wrote)
                ));
            }
            deleted.push(old);
        }
        let corpus = Corpus::load(workspace);
        let issues = issues_of(workspace, &corpus);
        let now = SystemTime::now();
        let incoming = corpus.incoming(&to).len();
        let card = corpus.by_key(&to).map(|node| {
            view::node_row(node, &node_issues(&issues, &node.id), incoming, now, &root)
        });
        let mut parts = vec!["# Renamed".to_string()];
        if !wrote.is_empty() {
            parts.push(format!("## Wrote\n{}", bullet_list(&wrote)));
        }
        if !deleted.is_empty() {
            parts.push(format!("## Deleted\n{}", bullet_list(&deleted)));
        }
        if let Some(card) = card {
            parts.push(card);
        }
        parts.push(format!(
            "Next, run `knowledge check {}` to confirm citations still resolve.",
            quote_key(&to)
        ));
        ToolCallResult::ok(parts.join("\n\n"))
    }
}

impl Tool for KnowledgeTool {
    fn name(&self) -> &str {
        "knowledge"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Empty opens the board. Otherwise one command: list, refs, check, create, or rename. Quote a key or folder that contains spaces."
                }
            }
        })
    }

    fn description(&self, _ctx: &Context) -> String {
        DESCRIPTION.into()
    }

    fn is_concurrency_safe(&self, input: &Value) -> bool {
        !is_write(input)
    }

    fn resource_keys(
        &self,
        input: &Value,
        _path_mode: crate::workspace::ToolPathMode,
        workspace_root: &Path,
    ) -> Vec<ResourceKey> {
        if !is_write(input) {
            return Vec::new();
        }
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
        let result = self.run(&input, &execution.workspace_root);
        Box::pin(std::future::ready(result))
    }
}

fn is_write(input: &Value) -> bool {
    let Some(command) = input.get("command").and_then(Value::as_str) else {
        return false;
    };
    let Ok(mut tokens) = tokenize(command) else {
        return false;
    };
    if tokens.first().map(String::as_str) == Some("knowledge") {
        tokens.remove(0);
    }
    matches!(
        tokens.first().map(String::as_str),
        Some("create" | "rename")
    )
}

fn issues_of(workspace: &Path, corpus: &Corpus) -> Vec<Issue> {
    let checks: Vec<validate::CheckNode<'_>> = corpus
        .nodes
        .iter()
        .map(|node| validate::CheckNode {
            id: &node.id,
            key: &node.key,
            value: &node.value,
            status: node.status,
            invalid_status: node.invalid_status.as_deref(),
            path: &node.path,
        })
        .collect();
    let mut symbols = crate::knowledge::symbol_check::SymbolCache::new();
    validate::validate(
        &checks,
        &|path| crate::knowledge::mentions::workspace_file_exists(workspace, path),
        &mut |file, chain, _node_path| symbols.check(workspace, file, chain),
    )
}

fn read_node_file(workspace: &Path, root: &str, node: &crate::knowledge::Node) -> String {
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

fn renamed_rel(path: &str, to: &str) -> String {
    match path.rfind('/') {
        Some(slash) => format!("{}/{to}.md", &path[..slash]),
        None => format!("{to}.md"),
    }
}

fn quote_key(key: &str) -> String {
    if key.is_empty() || key.chars().any(char::is_whitespace) {
        format!("\"{key}\"")
    } else {
        key.to_string()
    }
}

fn too_many(syntax: &str, example: &str) -> ToolCallResult {
    usage(
        "The name was split into several words. Quote a key or folder that contains spaces.",
        syntax,
        example,
    )
}

fn guide(args: &[String]) -> ToolCallResult {
    if !args.is_empty() {
        return usage("guide takes no arguments", "guide", "guide");
    }
    ToolCallResult::ok(view::guide().to_string())
}

fn unknown_command(other: &str) -> ToolCallResult {
    ToolCallResult::error(format!(
        "# `{other}` is not a knowledge command\n\n\
- `knowledge`\n\
- `knowledge guide`\n\
- `knowledge list [folder]`\n\
- `knowledge refs <key>`\n\
- `knowledge check [key or folder]`\n\
- `knowledge create <key> [folder]`\n\
- `knowledge rename <old key> <new key>`\n\n\
Quote a key or folder that contains spaces."
    ))
}

fn bullet_list(paths: &[String]) -> String {
    paths
        .iter()
        .map(|path| format!("- `{path}`"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn already_wrote(paths: &[String]) -> String {
    if paths.is_empty() {
        return String::new();
    }
    format!("\n\nAlready wrote:\n{}", bullet_list(paths))
}

fn tokenize(input: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&ch) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }
        if ch == '"' {
            chars.next();
            let mut buf = String::new();
            let mut closed = false;
            for next in chars.by_ref() {
                if next == '"' {
                    closed = true;
                    break;
                }
                buf.push(next);
            }
            if !closed {
                return Err("A quote was not closed".into());
            }
            out.push(buf);
            continue;
        }
        let mut buf = String::new();
        while let Some(&next) = chars.peek() {
            if next.is_whitespace() {
                break;
            }
            buf.push(next);
            chars.next();
        }
        out.push(buf);
    }
    Ok(out)
}

fn check_summary(checked: usize, scope: Option<&str>, bad: usize) -> String {
    let noun = if checked == 1 { "node" } else { "nodes" };
    let where_scope = match scope {
        Some(scope) => format!(" in `{scope}`"),
        None => String::new(),
    };
    if bad == 0 {
        format!("Checked {checked} {noun}{where_scope}. No issues.")
    } else if bad == 1 {
        format!("Checked {checked} {noun}{where_scope}. 1 node has an issue.")
    } else {
        format!("Checked {checked} {noun}{where_scope}. {bad} nodes have issues.")
    }
}

fn usage(problem: &str, syntax: &str, example: &str) -> ToolCallResult {
    ToolCallResult::error(format!(
        "# {problem}\n\n`knowledge {syntax}`\n\nExample: `knowledge {example}`"
    ))
}

fn join_blocks(parts: &[String]) -> String {
    parts
        .iter()
        .filter(|part| !part.trim().is_empty())
        .map(|part| part.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn paginate(workspace: &Path, markdown: &str) -> String {
    let (head, rest) = view::page(markdown, view::PAGE_CHARS);
    let Some(rest) = rest else {
        return head;
    };
    match spill(workspace, &rest) {
        Some(location) => format!(
            "{head}\n\n`{location}` holds only the part that did not fit here. Narrow to a folder, or read / grep that file."
        ),
        None => format!(
            "{head}\n\nThe rest of this reply was not kept. Narrow to a folder and ask again."
        ),
    }
}

fn spill(workspace: &Path, rest: &str) -> Option<String> {
    let dir = workspace.join(".litecode").join("bash");
    std::fs::create_dir_all(&dir).ok()?;
    let name = format!("knowledge_{}.txt", crate::terminal::bash_nonce());
    std::fs::write(dir.join(&name), rest).ok()?;
    Some(format!(".litecode/bash/{name}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Arc;

    use crate::knowledge::document::{RenderDoc, render_knowledge_markdown};
    use crate::knowledge::mentions::mention_source;

    fn ide_at(root: &Path) -> Arc<crate::ide_base::IdeBaseHandle> {
        let workspace =
            crate::workspace::WorkspaceService::new(root.to_path_buf()).expect("workspace");
        let engines = Arc::new(crate::engines::WorkspaceEngines::new());
        let hub = Arc::new(crate::terminal::TerminalHub::new());
        crate::terminal::install_hub(Arc::clone(&hub));
        crate::ide_base::IdeBaseHandle::new(workspace, engines, hub)
    }

    fn ctx(root: &Path) -> ToolExecutionContext {
        ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: root.to_path_buf(),
            call_id: String::new(),
            cancel: tokio_util::sync::CancellationToken::new(),
            output_limit: 8000,
            session_id: String::new(),
            session: None,
        }
    }

    fn plant(root: &Path, rel: &str, key: &str, status: Status, body: &str) {
        let full = root.join(".litecode").join("knowledge").join(rel);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(
            full,
            render_knowledge_markdown(RenderDoc {
                key,
                status,
                invalid_status: None,
                body,
                summary: key,
                x: None,
                y: None,
                w: None,
                h: None,
                extras: &[],
            }),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn commands_cover_the_board_and_writes() {
        let dir = tempfile::tempdir().unwrap();
        let tool = KnowledgeTool::new(ide_at(dir.path()));
        let missing = tool
            .execute(serde_json::json!({ "command": "" }), ctx(dir.path()))
            .await;
        assert!(missing.content.contains("# Start"));
        assert!(missing.content.contains("2 to 5 nodes"));
        assert!(missing.content.contains("creates `.litecode/knowledge`"));
        assert!(missing.content.contains("# Guide"));
        assert!(missing.content.contains("syntax, checks, and notes"));
        assert!(missing.content.contains("## Syntax"));
        assert!(missing.content.contains("# Commands"));
        assert!(missing.content.contains("knowledge check"));
        assert!(!missing.content.contains("# Status"));
        fs::create_dir_all(dir.path().join(".litecode").join("knowledge")).unwrap();
        let empty_dir = tool
            .execute(serde_json::json!({ "command": "" }), ctx(dir.path()))
            .await;
        assert!(empty_dir.content.contains("# Start"));
        assert!(!empty_dir.content.contains("# Status"));

        let created = tool
            .execute(
                serde_json::json!({ "command": "create seq 内核" }),
                ctx(dir.path()),
            )
            .await;
        assert_eq!(created.level, crate::types::ToolSignalLevel::Ok);
        assert!(created.content.contains("**seq**"));
        assert!(created.content.contains("pending"));
        let raw = fs::read_to_string(dir.path().join(".litecode/knowledge/内核/seq.md")).unwrap();
        assert!(raw.contains("status : pending"));
        let boarded = tool
            .execute(serde_json::json!({ "command": "" }), ctx(dir.path()))
            .await;
        assert!(boarded.content.contains("# Status"));
        assert!(!boarded.content.contains("# Start"));

        let again = tool
            .execute(
                serde_json::json!({ "command": "create seq" }),
                ctx(dir.path()),
            )
            .await;
        assert_eq!(again.level, crate::types::ToolSignalLevel::Error);
        assert!(again.content.contains("already exists"));

        let bad = tool
            .execute(
                serde_json::json!({ "command": "create \"bad/key\"" }),
                ctx(dir.path()),
            )
            .await;
        assert_eq!(bad.level, crate::types::ToolSignalLevel::Error);

        plant(
            dir.path(),
            "内核/session.md",
            "session",
            Status::Enabled,
            &format!("见 {}", mention_source("seq")),
        );
        let quoted = tool
            .execute(
                serde_json::json!({ "command": "refs \"seq\"" }),
                ctx(dir.path()),
            )
            .await;
        assert!(quoted.content.contains("## Cited by"));
        assert!(quoted.content.contains("**session**"));
        assert!(quoted.content.contains("node : seq"));
        assert!(quoted.content.contains("```node"));

        let renamed = tool
            .execute(
                serde_json::json!({ "command": "rename seq order" }),
                ctx(dir.path()),
            )
            .await;
        assert_eq!(
            renamed.level,
            crate::types::ToolSignalLevel::Ok,
            "{}",
            renamed.content
        );
        assert!(!dir.path().join(".litecode/knowledge/内核/seq.md").exists());
        let session =
            fs::read_to_string(dir.path().join(".litecode/knowledge/内核/session.md")).unwrap();
        assert!(session.contains("key=\"order\""));
        assert!(!session.contains("key=\"seq\""));
        assert!(!session.contains("id=\"seq\""));
        let corpus = Corpus::load(dir.path());
        assert!(corpus.by_key("order").is_some());
        assert!(corpus.by_key("seq").is_none());
        let issues = issues_of(dir.path(), &corpus);
        assert!(issues.iter().all(|issue| issue.code != "dangling_relation"));
        assert!(issues.iter().all(|issue| issue.code != "duplicate_key"));

        let checked = tool
            .execute(serde_json::json!({ "command": "check" }), ctx(dir.path()))
            .await;
        assert!(checked.content.contains("# Check"));
        assert!(!checked.content.contains("Check ·"));
        assert!(checked.content.contains("Cites pending \"order\"."));

        let listed = tool
            .execute(
                serde_json::json!({ "command": "list 内核" }),
                ctx(dir.path()),
            )
            .await;
        assert!(listed.content.contains("**order**"));
        assert!(listed.content.contains("**session**"));
        assert!(listed.content.contains(".litecode/knowledge/内核/order.md"));
        assert!(renamed.content.contains("# Renamed"));
        assert!(renamed.content.contains("## Wrote"));
        assert!(renamed.content.contains("## Deleted"));
        assert!(created.content.contains("knowledge check seq"));
        assert!(created.content.contains("knowledge panel"));
    }

    #[tokio::test]
    async fn list_spills_the_tail_outside_the_knowledge_root() {
        let dir = tempfile::tempdir().unwrap();
        let tool = KnowledgeTool::new(ide_at(dir.path()));
        for index in 0..80 {
            plant(
                dir.path(),
                &format!("n{index}.md"),
                &format!("n{index}"),
                Status::Enabled,
                &"正文".repeat(40),
            );
        }
        let listed = tool
            .execute(serde_json::json!({ "command": "list" }), ctx(dir.path()))
            .await;
        assert!(listed.content.contains(".litecode/bash/knowledge_"));
        let bash = dir.path().join(".litecode").join("bash");
        assert!(bash.is_dir());
        assert!(
            !dir.path()
                .join(".litecode/knowledge")
                .join("knowledge_spill.txt")
                .exists()
        );
    }

    #[test]
    fn writes_take_one_lock_and_reads_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let tool = KnowledgeTool::new(ide_at(dir.path()));
        assert!(tool.is_concurrency_safe(&serde_json::json!({ "command": "list" })));
        assert!(!tool.is_concurrency_safe(&serde_json::json!({ "command": "rename a b" })));
        let keys = tool.resource_keys(
            &serde_json::json!({ "command": "create seq" }),
            crate::workspace::ToolPathMode::All,
            dir.path(),
        );
        assert_eq!(keys, vec![ResourceKey::File(".litecode/knowledge".into())]);
    }

    #[tokio::test]
    async fn spaced_keys_are_quoted_and_arity_errors_differ() {
        let dir = tempfile::tempdir().unwrap();
        let tool = KnowledgeTool::new(ide_at(dir.path()));
        let created = tool
            .execute(
                serde_json::json!({ "command": "create \"blind test\"" }),
                ctx(dir.path()),
            )
            .await;
        assert_eq!(
            created.level,
            crate::types::ToolSignalLevel::Ok,
            "{}",
            created.content
        );
        assert!(created.content.contains("knowledge check \"blind test\""));
        assert!(
            dir.path()
                .join(".litecode/knowledge/blind test.md")
                .is_file()
        );
        let checked = tool
            .execute(serde_json::json!({ "command": "check" }), ctx(dir.path()))
            .await;
        assert!(checked.content.contains("# Check"));
        assert!(!checked.content.contains("Check ·"));
        assert!(checked.content.contains("Checked 1 node. No issues."));

        let split = tool
            .execute(
                serde_json::json!({ "command": "refs blind test" }),
                ctx(dir.path()),
            )
            .await;
        assert!(split.content.contains("split into several words"));
        let missing = tool
            .execute(serde_json::json!({ "command": "refs" }), ctx(dir.path()))
            .await;
        assert!(missing.content.contains("needs one key"));
        assert!(!missing.content.contains("split into several words"));

        let unknown = tool
            .execute(serde_json::json!({ "command": "explode" }), ctx(dir.path()))
            .await;
        assert!(unknown.content.contains("is not a knowledge command"));
        assert!(unknown.content.contains("knowledge guide"));
        assert!(unknown.content.contains("knowledge list [folder]"));
        assert!(
            unknown
                .content
                .contains("knowledge rename <old key> <new key>")
        );

        fs::create_dir_all(dir.path().join(".litecode/knowledge/empty")).unwrap();
        let listed = tool
            .execute(
                serde_json::json!({ "command": "list empty" }),
                ctx(dir.path()),
            )
            .await;
        assert!(listed.content.contains("`empty` has no nodes"));
        assert!(!listed.content.contains("Create the first"));
    }

    #[tokio::test]
    async fn board_names_the_root_in_use_when_both_exist() {
        let dir = tempfile::tempdir().unwrap();
        let tool = KnowledgeTool::new(ide_at(dir.path()));
        let public = dir.path().join("knowledge");
        fs::create_dir_all(&public).unwrap();
        fs::write(
            public.join("seq.md"),
            render_knowledge_markdown(RenderDoc {
                key: "seq",
                status: Status::Enabled,
                invalid_status: None,
                body: "seq",
                summary: "seq",
                x: None,
                y: None,
                w: None,
                h: None,
                extras: &[],
            }),
        )
        .unwrap();
        plant(
            dir.path(),
            "hidden.md",
            "hidden",
            Status::Enabled,
            "private",
        );
        let board = tool
            .execute(serde_json::json!({ "command": "" }), ctx(dir.path()))
            .await;
        assert!(
            board
                .content
                .contains("Current knowledge root: `knowledge`.")
        );
        assert!(
            !board
                .content
                .contains("is not read. Nodes there are not citation targets.")
        );
        assert!(board.content.contains("**seq**"));
        assert!(!board.content.contains("**hidden**"));
    }

    #[tokio::test]
    async fn rename_retargets_the_filename_to_the_new_key() {
        let dir = tempfile::tempdir().unwrap();
        let tool = KnowledgeTool::new(ide_at(dir.path()));
        plant(
            dir.path(),
            "alias.md",
            "other-name",
            Status::Enabled,
            "body",
        );
        let renamed = tool
            .execute(
                serde_json::json!({ "command": "rename other-name canonical" }),
                ctx(dir.path()),
            )
            .await;
        assert_eq!(
            renamed.level,
            crate::types::ToolSignalLevel::Ok,
            "{}",
            renamed.content
        );
        let next = dir.path().join(".litecode/knowledge/canonical.md");
        assert!(next.is_file());
        assert!(!dir.path().join(".litecode/knowledge/alias.md").exists());
        let text = fs::read_to_string(next).unwrap();
        assert!(text.contains("node : canonical"));

        let tail = format!("FULLTEXT-{}", "y".repeat(500));
        plant(
            dir.path(),
            "note.md",
            "note",
            Status::Enabled,
            &format!("{}\n{tail}", mention_source("missing")),
        );
        let refs = tool
            .execute(
                serde_json::json!({ "command": "refs note" }),
                ctx(dir.path()),
            )
            .await;
        assert!(
            refs.content
                .contains("Citation \"missing\" does not exist.")
        );
        assert!(!refs.content.contains("- `missing` does not exist"));
        assert!(
            refs.content
                .contains("Edit the body with `edit`. Rename with `knowledge rename`.")
        );
        assert!(refs.content.contains("node : note"));
        assert!(refs.content.contains(&tail));
        assert!(!refs.content.contains("**note**"));
    }

    #[tokio::test]
    async fn rename_same_key_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let tool = KnowledgeTool::new(ide_at(dir.path()));
        plant(dir.path(), "seq.md", "seq", Status::Enabled, "body");
        let same = tool
            .execute(
                serde_json::json!({ "command": "rename seq seq" }),
                ctx(dir.path()),
            )
            .await;
        assert!(same.content.contains("# Unchanged"));
        assert!(!same.content.contains("# Renamed"));
    }

    #[tokio::test]
    async fn guide_states_the_rules_without_reading_disk() {
        let dir = tempfile::tempdir().unwrap();
        let tool = KnowledgeTool::new(ide_at(dir.path()));
        let guide = tool
            .execute(serde_json::json!({ "command": "guide" }), ctx(dir.path()))
            .await;
        assert_eq!(guide.level, crate::types::ToolSignalLevel::Ok);
        assert!(guide.content.contains("## Syntax"));
        assert!(guide.content.contains("## Check"));
        assert!(guide.content.contains("[@ file=\"src/a.rs\"]"));
        assert!(!guide.content.contains("uses the old form"));
        assert!(
            guide
                .content
                .contains("A file citation path is not in the workspace.")
        );
        assert!(!guide.content.contains("mtime"));
        assert!(!guide.content.contains("[[node"));
        assert!(!dir.path().join(".litecode").exists());
        let extra = tool
            .execute(
                serde_json::json!({ "command": "guide extra" }),
                ctx(dir.path()),
            )
            .await;
        assert!(extra.content.contains("guide takes no arguments"));
    }

    #[tokio::test]
    async fn missing_file_citation_is_an_error_until_the_path_exists() {
        let dir = tempfile::tempdir().unwrap();
        let tool = KnowledgeTool::new(ide_at(dir.path()));
        plant(
            dir.path(),
            "seq.md",
            "seq",
            Status::Enabled,
            "see [@ file=\"src/a.rs\"] and [@ key=\"nope\"]",
        );
        let missing = tool
            .execute(serde_json::json!({ "command": "check" }), ctx(dir.path()))
            .await;
        assert!(
            missing
                .content
                .contains("File \"src/a.rs\" does not exist.")
        );
        assert!(
            missing
                .content
                .contains("Citation \"nope\" does not exist.")
        );
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/a.rs"), "fn main() {}\n").unwrap();
        let found = tool
            .execute(serde_json::json!({ "command": "check" }), ctx(dir.path()))
            .await;
        assert!(!found.content.contains("src/a.rs"));
        plant(
            dir.path(),
            "seq.md",
            "seq",
            Status::Enabled,
            "dir [@ file=\"src\"]",
        );
        let folder = tool
            .execute(serde_json::json!({ "command": "check" }), ctx(dir.path()))
            .await;
        assert!(!folder.content.contains("does not exist"));
        plant(
            dir.path(),
            "seq.md",
            "seq",
            Status::Enabled,
            "```\n[@ file=\"missing.rs\"]\n```\n",
        );
        let fenced = tool
            .execute(serde_json::json!({ "command": "check" }), ctx(dir.path()))
            .await;
        assert!(!fenced.content.contains("missing.rs"));
        plant(
            dir.path(),
            "seq.md",
            "seq",
            Status::Enabled,
            "[@ file=\"../secret\"]",
        );
        let escaped = tool
            .execute(serde_json::json!({ "command": "check" }), ctx(dir.path()))
            .await;
        assert!(
            escaped
                .content
                .contains("File \"../secret\" does not exist.")
        );
    }

    #[tokio::test]
    async fn plain_markdown_stays_on_disk_and_is_not_a_node() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".litecode").join("knowledge");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("notes.md"), "just a note\n").unwrap();
        fs::write(
            root.join("broken.md"),
            "```node\nnode : a/b\nstatus : enabled\nsummary : \n```\n",
        )
        .unwrap();
        let tool = KnowledgeTool::new(ide_at(dir.path()));
        let board = tool
            .execute(serde_json::json!({ "command": "" }), ctx(dir.path()))
            .await;
        assert!(
            board
                .content
                .contains("These files are not nodes and were left in place:")
        );
        assert!(board.content.contains("`notes.md`"));
        assert!(!board.content.contains("`broken.md`"));
        assert!(board.content.contains("**a/b**") || board.content.contains("a/b"));
        assert!(root.join("notes.md").is_file());
        let text = fs::read_to_string(root.join("notes.md")).unwrap();
        assert_eq!(text, "just a note\n");
        let checked = tool
            .execute(serde_json::json!({ "command": "check" }), ctx(dir.path()))
            .await;
        assert!(checked.content.contains("not valid"));
        assert!(!checked.content.contains("notes.md"));
        assert!(!checked.content.contains("Declaration is missing"));
    }
}
