//! Corpus checks. Error codes match the frontend `KnowledgeIssueCode` set.

use super::document::{self, Status};
use super::mentions::{extract_markers, is_knowledge_key, normalize_key};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub node_id: String,
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub reference: Option<String>,
}

pub struct CheckNode<'a> {
    pub id: &'a str,
    pub key: &'a str,
    pub value: &'a str,
    pub status: Status,
    pub invalid_status: Option<&'a str>,
    pub path: &'a str,
}

pub fn validate<'a>(nodes: &[CheckNode<'a>]) -> Vec<Issue> {
    let mut key_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for node in nodes {
        let key = normalize_key(node.key);
        if !is_knowledge_key(&key) {
            continue;
        }
        *key_counts.entry(key).or_insert(0) += 1;
    }

    let mut by_key: std::collections::HashMap<String, Status> = std::collections::HashMap::new();
    for node in nodes {
        let key = normalize_key(node.key);
        if is_knowledge_key(&key) {
            by_key.entry(key).or_insert(node.status);
        }
    }

    let mut issues = Vec::new();
    for node in nodes {
        let key = normalize_key(node.key);
        if let Some(raw) = node.invalid_status {
            issues.push(Issue {
                node_id: node.id.to_string(),
                severity: Severity::Error,
                code: "invalid_status".into(),
                message: format!(
                    "Status \"{raw}\" is not valid. Use enabled, disabled, or pending."
                ),
                reference: Some(raw.to_string()),
            });
        }
        if key.is_empty() {
            issues.push(Issue {
                node_id: node.id.to_string(),
                severity: Severity::Error,
                code: "empty_key".into(),
                message: "Declaration is missing.".into(),
                reference: None,
            });
        } else if !is_knowledge_key(&key) {
            issues.push(Issue {
                node_id: node.id.to_string(),
                severity: Severity::Error,
                code: "empty_key".into(),
                message: format!("Key \"{key}\" is not valid."),
                reference: Some(key.clone()),
            });
        } else if key_counts.get(&key).copied().unwrap_or(0) > 1 {
            issues.push(Issue {
                node_id: node.id.to_string(),
                severity: Severity::Error,
                code: "duplicate_key".into(),
                message: format!("Key \"{key}\" is duplicated."),
                reference: Some(key.clone()),
            });
        } else if !node.path.is_empty() && document::file_stem(node.path) != key {
            let stem = document::file_stem(node.path);
            issues.push(Issue {
                node_id: node.id.to_string(),
                severity: Severity::Warning,
                code: "filename_mismatch".into(),
                message: format!("Filename \"{stem}\" does not match declaration \"{key}\"."),
                reference: Some(key.clone()),
            });
        }

        let mut seen = std::collections::HashSet::new();
        for ref_key in extract_markers(node.value) {
            if ref_key.is_empty() || !seen.insert(ref_key.clone()) {
                continue;
            }
            if ref_key == key {
                issues.push(Issue {
                    node_id: node.id.to_string(),
                    severity: Severity::Error,
                    code: "self_relation".into(),
                    message: "Citation points at itself.".into(),
                    reference: Some(ref_key),
                });
                continue;
            }
            match by_key.get(&ref_key) {
                None => issues.push(Issue {
                    node_id: node.id.to_string(),
                    severity: Severity::Error,
                    code: "dangling_relation".into(),
                    message: format!("Citation \"{ref_key}\" does not exist."),
                    reference: Some(ref_key),
                }),
                Some(target) if node.status == Status::Enabled && *target != Status::Enabled => {
                    issues.push(Issue {
                        node_id: node.id.to_string(),
                        severity: Severity::Warning,
                        code: "inactive_target".into(),
                        message: format!("Cites {} \"{ref_key}\".", status_label(*target)),
                        reference: Some(ref_key),
                    });
                }
                Some(_) => {}
            }
        }
    }
    issues
}

fn status_label(status: Status) -> &'static str {
    match status {
        Status::Disabled => "disabled",
        Status::Pending => "pending",
        Status::Enabled => "enabled",
    }
}
