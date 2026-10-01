//! Load the knowledge directory into nodes and citation edges.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use super::document::{self, ParsedFile, Status};
use super::mentions::{is_knowledge_key, normalize_key};
use super::root::{self, KnowledgeRoot};

#[derive(Debug, Clone)]
pub struct Node {
    pub id: String,
    pub key: String,
    pub value: String,
    pub summary: String,
    pub relations: Vec<String>,
    pub status: Status,
    pub invalid_status: Option<String>,
    pub path: String,
    pub folder_id: Option<String>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub w: Option<f64>,
    pub h: Option<f64>,
    pub extras: Vec<String>,
    pub modified: Option<SystemTime>,
}

#[derive(Debug, Clone)]
pub struct Corpus {
    pub root: Option<String>,
    pub nodes: Vec<Node>,
    /// Markdown files with no `node` declaration. Left on disk.
    pub unknown: Vec<String>,
}

impl Corpus {
    pub fn load(workspace: &Path) -> Self {
        let KnowledgeRoot::Present(root) = root::locate(workspace) else {
            return Self {
                root: None,
                nodes: Vec::new(),
                unknown: Vec::new(),
            };
        };
        let mut files = Vec::new();
        walk(workspace, &root, &root, &mut files);
        files.sort_by(|a, b| a.parsed.path.cmp(&b.parsed.path));
        let (nodes, unknown) = nodes_from_parsed(files);
        Self {
            root: Some(root),
            nodes,
            unknown,
        }
    }

    pub fn latest_modified(&self) -> Option<SystemTime> {
        self.nodes.iter().filter_map(|node| node.modified).max()
    }

    pub fn by_key(&self, key: &str) -> Option<&Node> {
        let key = normalize_key(key);
        self.nodes
            .iter()
            .find(|node| normalize_key(&node.key) == key && is_knowledge_key(&key))
    }

    pub fn incoming(&self, key: &str) -> Vec<&Node> {
        let key = normalize_key(key);
        self.nodes
            .iter()
            .filter(|node| node.relations.iter().any(|rel| rel == &key))
            .collect()
    }

    /// Nodes whose folder is `folder` or a folder inside it.
    pub fn in_folder<'a>(&'a self, folder: &str) -> Vec<&'a Node> {
        let folder = folder.trim_matches('/');
        self.nodes
            .iter()
            .filter(|node| folder_matches(node.folder_id.as_deref(), folder))
            .collect()
    }
}

fn folder_matches(folder_id: Option<&str>, folder: &str) -> bool {
    if folder.is_empty() {
        return true;
    }
    match folder_id {
        Some(id) => id == folder || id.starts_with(&format!("{folder}/")),
        None => false,
    }
}

struct LoadedFile {
    parsed: ParsedFile,
    modified: Option<SystemTime>,
}

fn walk(workspace: &Path, root: &str, dir_rel: &str, out: &mut Vec<LoadedFile>) {
    let abs = workspace.join(dir_rel);
    let Ok(entries) = fs::read_dir(&abs) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(|entry| entry.ok()).collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let rel = if dir_rel == root {
            format!("{root}/{name}")
        } else {
            format!("{dir_rel}/{name}")
        };
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            walk(workspace, root, &rel, out);
            continue;
        }
        if !name.to_ascii_lowercase().ends_with(".md") {
            continue;
        }
        let Ok(markdown) = fs::read_to_string(entry.path()) else {
            continue;
        };
        let path = rel
            .strip_prefix(&format!("{root}/"))
            .unwrap_or(rel.as_str())
            .to_string();
        out.push(LoadedFile {
            parsed: document::parse_knowledge_markdown(&path, &markdown),
            modified: meta.modified().ok(),
        });
    }
}

fn nodes_from_parsed(files: Vec<LoadedFile>) -> (Vec<Node>, Vec<String>) {
    let mut unknown = Vec::new();
    let mut declared = Vec::new();
    for file in files {
        if normalize_key(&file.parsed.key).is_empty() {
            unknown.push(file.parsed.path.clone());
        } else {
            declared.push(file);
        }
    }
    let mut key_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for file in &declared {
        let key = normalize_key(&file.parsed.key);
        if !is_knowledge_key(&key) {
            continue;
        }
        *key_counts.entry(key).or_insert(0) += 1;
    }
    let mut used = std::collections::HashSet::new();
    let nodes = declared
        .into_iter()
        .map(|file| {
            let key = normalize_key(&file.parsed.key);
            let unique = is_knowledge_key(&key) && key_counts.get(&key).copied() == Some(1);
            let mut id = if unique {
                key.clone()
            } else {
                file.parsed.path.clone()
            };
            if used.contains(&id) {
                id = file.parsed.path.clone();
            }
            used.insert(id.clone());
            Node {
                id,
                key,
                value: file.parsed.body,
                summary: file.parsed.summary,
                relations: file.parsed.refs,
                status: file.parsed.status,
                invalid_status: file.parsed.invalid_status,
                path: file.parsed.path,
                folder_id: file.parsed.folder_id,
                x: file.parsed.x,
                y: file.parsed.y,
                w: file.parsed.w,
                h: file.parsed.h,
                extras: file.parsed.extras,
                modified: file.modified,
            }
        })
        .collect();
    (nodes, unknown)
}

#[cfg(test)]
mod fixture_tests {
    use super::*;
    use std::path::PathBuf;

    use crate::knowledge::document::{self, RenderDoc};
    use crate::knowledge::validate::{self, CheckNode};

    #[test]
    fn shared_fixtures_match_expected_json() {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let corpus = Corpus::load(&workspace);
        assert_eq!(corpus.root.as_deref(), Some("knowledge"));
        let expected: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(workspace.join("knowledge/expected.json")).unwrap(),
        )
        .unwrap();

        let mut actual = Vec::new();
        for node in &corpus.nodes {
            actual.push(serde_json::json!({
                "path": node.path,
                "key": node.key,
                "status": node.status.as_str(),
                "invalidStatus": node.invalid_status,
                "summary": node.summary,
                "refs": node.relations,
                "extras": node.extras,
            }));
        }
        actual.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
        let mut wanted = expected["files"].as_array().unwrap().clone();
        wanted.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
        assert_eq!(
            serde_json::Value::Array(actual),
            serde_json::Value::Array(wanted)
        );

        let checks: Vec<CheckNode<'_>> = corpus
            .nodes
            .iter()
            .map(|node| CheckNode {
                id: &node.id,
                key: &node.key,
                value: &node.value,
                status: node.status,
                invalid_status: node.invalid_status.as_deref(),
                path: &node.path,
            })
            .collect();
        let mut issues: Vec<_> = validate::validate(
            &checks,
            &|path| crate::knowledge::mentions::workspace_file_exists(&workspace, path),
            &mut |_, _, _| crate::knowledge::symbol_check::SymbolCheck::Present,
        )
        .into_iter()
        .map(|issue| format!("{}:{}", issue.node_id, issue.code))
        .collect();
        issues.sort();
        let mut wanted_issues: Vec<String> = expected["issues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|issue| {
                format!(
                    "{}:{}",
                    issue["node"].as_str().unwrap(),
                    issue["code"].as_str().unwrap()
                )
            })
            .collect();
        wanted_issues.sort();
        assert_eq!(issues, wanted_issues);

        let seq = corpus.by_key("seq").unwrap();
        let rendered = document::render_knowledge_markdown(RenderDoc {
            key: &seq.key,
            status: seq.status,
            invalid_status: seq.invalid_status.as_deref(),
            body: &seq.value,
            summary: &seq.summary,
            x: seq.x,
            y: seq.y,
            w: seq.w,
            h: seq.h,
            extras: &seq.extras,
        });
        let again = document::parse_knowledge_markdown(&seq.path, &rendered);
        assert_eq!(again.extras, vec!["tags : a".to_string()]);
        assert_eq!(again.refs, vec!["session".to_string()]);

        let bad = corpus.by_key("bad-status").unwrap();
        assert_eq!(bad.status, document::Status::Enabled);
        assert_eq!(bad.invalid_status.as_deref(), Some("nope"));
        let rendered = document::render_knowledge_markdown(RenderDoc {
            key: &bad.key,
            status: bad.status,
            invalid_status: bad.invalid_status.as_deref(),
            body: &bad.value,
            summary: &bad.summary,
            x: bad.x,
            y: bad.y,
            w: bad.w,
            h: bad.h,
            extras: &bad.extras,
        });
        assert!(rendered.contains("status : nope"));
        let again = document::parse_knowledge_markdown(&bad.path, &rendered);
        assert_eq!(again.invalid_status.as_deref(), Some("nope"));
    }
}
