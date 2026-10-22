//! One class for the knowledge base, attached when a person sends a message.
//!
//! Only enabled nodes count. A library with no enabled node is not a class,
//! and the caller writes nothing. The class is computed by the resident index
//! when it publishes a snapshot; a turn only reads it (see [`super::index::turn_note`]).

use std::collections::HashSet;
use std::path::Path;

use super::corpus::Corpus;
use super::document::Status;
use super::validate::{Issue, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Error,
    Warning,
    Ok,
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Ok => "ok",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub class: Class,
    pub text: String,
}

/// `None` when there is no knowledge base, it has no enabled nodes, or the
/// resident index is not ready yet.
pub fn note(workspace: &Path) -> Option<Note> {
    super::index::turn_note(workspace)
}

/// Turn-reminder class of a resolved corpus.
///
/// Pending and disabled nodes are ignored, including their issues. No enabled
/// node means no note.
pub fn note_from(corpus: &Corpus, issues: &[Issue]) -> Option<Note> {
    if corpus.root.is_none() {
        return None;
    }
    let enabled: HashSet<&str> = corpus
        .nodes
        .iter()
        .filter(|node| node.status == Status::Enabled)
        .map(|node| node.path.as_str())
        .collect();
    if enabled.is_empty() {
        return None;
    }
    let errors = nodes_with(issues, Severity::Error, |issue| {
        enabled.contains(issue.node_path.as_str())
    });
    if !errors.is_empty() {
        return Some(Note {
            class: Class::Error,
            text: error_text(errors.len()),
        });
    }
    let warnings = nodes_with(issues, Severity::Warning, |issue| {
        issue.code != "inactive_target" && enabled.contains(issue.node_path.as_str())
    });
    if !warnings.is_empty() {
        return Some(Note {
            class: Class::Warning,
            text: warning_text(warnings.len()),
        });
    }
    Some(Note {
        class: Class::Ok,
        text: ok_text(enabled.len()),
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
        .map(|issue| issue.node_path.clone())
        .collect()
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

    fn pending(key: &str, body: &str) -> String {
        format!("```node\nnode : {key}\nstatus : pending\nsummary : {key}\n```\n{body}")
    }

    #[test]
    fn classes_cover_errors_warnings_and_enabled() {
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

        let mixed = tempfile::tempdir().unwrap();
        write_node(mixed.path(), "draft.md", &pending("draft", ""));
        write_node(mixed.path(), "live.md", &enabled("live", ""));
        let held = note(mixed.path()).unwrap();
        assert_eq!(held.class, Class::Ok);
        assert_eq!(held.text, "Knowledge base: 1 enabled node.");

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

}
