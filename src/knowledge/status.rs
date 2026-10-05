//! One class for the knowledge base, computed when a person sends a message.
//!
//! An empty knowledge base is not a class. The caller writes nothing.

use std::collections::HashSet;
use std::path::Path;

use super::corpus::Corpus;
use super::document::Status;
use super::symbol_check::SymbolCache;
use super::validate::{self, Issue, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Error,
    Warning,
    Pending,
    Ok,
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Pending => "pending",
            Self::Ok => "ok",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub class: Class,
    pub text: String,
}

/// `None` when there is no knowledge base, or it has no nodes.
pub fn note(workspace: &Path) -> Option<Note> {
    let corpus = Corpus::load(workspace);
    if corpus.root.is_none() || corpus.nodes.is_empty() {
        return None;
    }
    let issues = collect_issues(workspace, &corpus);
    let errors = nodes_with(&issues, Severity::Error, |_| true);
    if !errors.is_empty() {
        return Some(Note {
            class: Class::Error,
            text: error_text(errors.len()),
        });
    }
    let warnings = nodes_with(&issues, Severity::Warning, |issue| {
        issue.code != "inactive_target"
    });
    if !warnings.is_empty() {
        return Some(Note {
            class: Class::Warning,
            text: warning_text(warnings.len()),
        });
    }
    let pending = corpus
        .nodes
        .iter()
        .filter(|node| node.status == Status::Pending)
        .count();
    if pending > 0 {
        return Some(Note {
            class: Class::Pending,
            text: pending_text(pending),
        });
    }
    let enabled = corpus
        .nodes
        .iter()
        .filter(|node| node.status == Status::Enabled)
        .count();
    Some(Note {
        class: Class::Ok,
        text: ok_text(enabled),
    })
}

fn nodes_with(
    issues: &[Issue],
    severity: Severity,
    keep: impl Fn(&Issue) -> bool,
) -> HashSet<String> {
    issues
        .iter()
        .filter(|issue| issue.severity == severity && keep(issue))
        .map(|issue| issue.node_id.clone())
        .collect()
}

fn collect_issues(workspace: &Path, corpus: &Corpus) -> Vec<Issue> {
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
    let mut symbols = SymbolCache::new();
    validate::validate(
        &checks,
        &|path| super::mentions::workspace_file_exists(workspace, path),
        &mut |file, chain, _node_path| symbols.check(workspace, file, chain),
    )
}

fn error_text(count: usize) -> String {
    let nodes = if count == 1 { "node has" } else { "nodes have" };
    format!(
        "Knowledge base: {count} {nodes} errors. You can run `knowledge check` and tell the user."
    )
}

fn warning_text(count: usize) -> String {
    let nodes = if count == 1 { "node has" } else { "nodes have" };
    format!(
        "Knowledge base: {count} {nodes} warnings. You can run `knowledge check`, tell the user, and suggest a fix."
    )
}

fn pending_text(count: usize) -> String {
    if count == 1 {
        "Knowledge base: 1 node is pending review.".into()
    } else {
        format!("Knowledge base: {count} nodes are pending review.")
    }
}

fn ok_text(count: usize) -> String {
    let nodes = if count == 1 { "node" } else { "nodes" };
    format!("Knowledge base: {count} enabled {nodes}.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_node(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(".litecode").join("knowledge").join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    fn enabled(key: &str, body: &str) -> String {
        format!("```node\nnode : {key}\nstatus : enabled\nsummary : {key}\n```\n{body}")
    }

    #[test]
    fn an_empty_library_is_not_a_note() {
        let dir = tempfile::tempdir().unwrap();
        assert!(note(dir.path()).is_none());
        fs::create_dir_all(dir.path().join(".litecode").join("knowledge")).unwrap();
        assert!(note(dir.path()).is_none());
    }

    #[test]
    fn classes_cover_errors_warnings_pending_and_enabled() {
        let errors = tempfile::tempdir().unwrap();
        write_node(
            errors.path(),
            "seq.md",
            &enabled("seq", "see [@ key=\"missing\"]\n"),
        );
        let error = note(errors.path()).unwrap();
        assert_eq!(error.class, Class::Error);
        assert_eq!(
            error.text,
            "Knowledge base: 1 node has errors. You can run `knowledge check` and tell the user."
        );

        let two = tempfile::tempdir().unwrap();
        write_node(two.path(), "a.md", &enabled("a", "see [@ key=\"gone\"]\n"));
        write_node(two.path(), "b.md", &enabled("b", "see [@ key=\"also\"]\n"));
        assert_eq!(
            note(two.path()).unwrap().text,
            "Knowledge base: 2 nodes have errors. You can run `knowledge check` and tell the user."
        );

        let warning = tempfile::tempdir().unwrap();
        write_node(warning.path(), "other.md", &enabled("seq", ""));
        let warned = note(warning.path()).unwrap();
        assert_eq!(warned.class, Class::Warning);
        assert_eq!(
            warned.text,
            "Knowledge base: 1 node has warnings. You can run `knowledge check`, tell the user, and suggest a fix."
        );

        let pending = tempfile::tempdir().unwrap();
        write_node(
            pending.path(),
            "draft.md",
            "```node\nnode : draft\nstatus : pending\nsummary : draft\n```\n",
        );
        write_node(pending.path(), "live.md", &enabled("live", ""));
        let held = note(pending.path()).unwrap();
        assert_eq!(held.class, Class::Pending);
        assert_eq!(held.text, "Knowledge base: 1 node is pending review.");

        let sound = tempfile::tempdir().unwrap();
        write_node(sound.path(), "seq.md", &enabled("seq", ""));
        write_node(sound.path(), "session.md", &enabled("session", ""));
        write_node(
            sound.path(),
            "old.md",
            "```node\nnode : old\nstatus : disabled\nsummary : old\n```\n",
        );
        let ok = note(sound.path()).unwrap();
        assert_eq!(ok.class, Class::Ok);
        assert_eq!(ok.text, "Knowledge base: 2 enabled nodes.");
    }

    #[test]
    fn citing_a_pending_node_stays_pending() {
        let dir = tempfile::tempdir().unwrap();
        write_node(
            dir.path(),
            "live.md",
            &enabled("live", "see [@ key=\"draft\"]\n"),
        );
        write_node(
            dir.path(),
            "draft.md",
            "```node\nnode : draft\nstatus : pending\nsummary : draft\n```\n",
        );
        let held = note(dir.path()).unwrap();
        assert_eq!(held.class, Class::Pending);
        assert_eq!(held.text, "Knowledge base: 1 node is pending review.");
    }
}
