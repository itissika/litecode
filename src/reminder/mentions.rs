//! Attachment of what a user message cited.
//!
//! Written immediately after that user row. A citation is left out when a
//! mentions reminder still on the live surface carries the same snapshot.
//! Compaction removes that copy, so citing it again attaches a fresh one.
//! It is not a seam reminder: nothing restores it on its own.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use sha2::{Digest, Sha256};

use crate::engines::code_search::{ScopeMatch, find_scope};
use crate::knowledge::corpus::Corpus;
use crate::knowledge::document::Status;
use crate::knowledge::mentions::{OrderedRef, extract_refs_in_order};
use crate::knowledge::view;

use super::kinds::{MentionRef, MentionsBody, Reminder};

pub const MENTION_CODE_LINES: usize = 80;

const OPENING: &str =
    "Context the user referenced in the message above, in mention order, snapshot at send time.";

/// Expand node and symbol citations. File citations stay in the user message.
///
/// `delivered` maps a citation key to the snapshot digest still visible to the
/// model. A matching digest is unchanged context and is not attached again.
pub fn build(
    workspace: &Path,
    user_text: &str,
    delivered: &HashMap<String, String>,
) -> Option<Reminder> {
    let refs = extract_refs_in_order(user_text);
    if refs.is_empty() {
        return None;
    }
    let corpus = Corpus::load(workspace);
    let mut seen = HashSet::new();
    let mut sections = Vec::new();
    let mut snaps = Vec::new();
    for citation in refs {
        match citation {
            OrderedRef::File { .. } => {}
            OrderedRef::Node { id, label } => {
                let key = format!("node:{id}");
                if !seen.insert(key.clone()) {
                    continue;
                }
                if let Some((section, digest)) = node_section(workspace, &corpus, &id, &label) {
                    if delivered.get(&key) == Some(&digest) {
                        continue;
                    }
                    snaps.push(MentionRef {
                        kind: "node".into(),
                        key,
                        digest: Some(digest),
                    });
                    sections.push(section);
                }
            }
            OrderedRef::Symbol {
                path,
                symbol,
                lines,
                label: _,
            } => {
                let identity = match symbol
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                {
                    Some(chain) => format!("symbol:{path}:{chain}"),
                    None => format!(
                        "range:{path}:{}",
                        lines
                            .map(crate::knowledge::mentions::format_line_span)
                            .unwrap_or_default()
                    ),
                };
                if !seen.insert(identity.clone()) {
                    continue;
                }
                if let Some((section, digest)) =
                    symbol_section(workspace, &path, symbol.as_deref(), lines)
                {
                    if delivered.get(&identity) == Some(&digest) {
                        continue;
                    }
                    snaps.push(MentionRef {
                        kind: "symbol".into(),
                        key: identity,
                        digest: Some(digest),
                    });
                    sections.push(section);
                }
            }
        }
    }
    if sections.is_empty() {
        return None;
    }
    let text = format!("{OPENING}\n\n{}", sections.join("\n\n"));
    Some(Reminder::Mentions(MentionsBody { refs: snaps, text }))
}

fn digest_of(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn pack(section: String) -> (String, String) {
    let digest = digest_of(&section);
    (section, digest)
}

fn node_section(
    workspace: &Path,
    corpus: &Corpus,
    id: &str,
    label: &str,
) -> Option<(String, String)> {
    let Some(node) = corpus.by_key(id) else {
        return Some(pack(format!("Node \"{label}\" was not found.")));
    };
    if node.status == Status::Disabled {
        return Some(pack(format!("Node \"{}\" is disabled.", node.key)));
    }
    let incoming = corpus.incoming(&node.key).len();
    let root = corpus.root.as_deref().unwrap_or("");
    let card = view::node_card(1, node, &[], incoming, SystemTime::now(), root);
    let body = read_node_body(workspace, root, &node.path).unwrap_or_else(|| node.value.clone());
    let body = body.trim();
    let section = if body.is_empty() {
        card
    } else {
        format!("{card}\n\n{body}")
    };
    // The card's relative time changes on its own. The digest follows the note.
    let digest = digest_of(&format!(
        "{}\n{}\n{}\n{incoming}\n{}\n{}\n{body}",
        node.key,
        node.path,
        node.status.as_str(),
        node.relations.len(),
        node.summary,
    ));
    Some((section, digest))
}

fn read_node_body(workspace: &Path, root: &str, path: &str) -> Option<String> {
    if root.is_empty()
        || path.is_empty()
        || !crate::knowledge::mentions::is_workspace_file_ref(path)
    {
        return None;
    }
    fs::read_to_string(workspace.join(root).join(path)).ok()
}

fn symbol_section(
    workspace: &Path,
    path: &str,
    symbol: Option<&str>,
    lines: Option<crate::knowledge::mentions::LineSpan>,
) -> Option<(String, String)> {
    let chain = symbol.map(str::trim).filter(|text| !text.is_empty());
    if !crate::knowledge::mentions::is_workspace_file_ref(path) || !workspace.join(path).is_file() {
        return Some(pack(match chain {
            Some(chain) => format!("Symbol \"{chain}\" was not found in \"{path}\"."),
            None => format!("File \"{path}\" was not found."),
        }));
    }
    let content = fs::read_to_string(workspace.join(path)).unwrap_or_default();
    if content.is_empty() && chain.is_some() {
        let chain = chain.unwrap_or("");
        return Some(pack(format!(
            "Symbol \"{chain}\" was not found in \"{path}\"."
        )));
    }
    let file_lines: Vec<&str> = content.lines().collect();
    if let Some(chain) = chain {
        return Some(pack(match find_scope(path, &content, chain) {
            ScopeMatch::Unique(scope) => code_section(
                path,
                Some(chain),
                scope.start_line,
                scope.end_line,
                &file_lines,
            ),
            ScopeMatch::Ambiguous(_) => {
                format!("Symbol \"{chain}\" is not unique in \"{path}\".")
            }
            ScopeMatch::Missing => format!("Symbol \"{chain}\" was not found in \"{path}\"."),
        }));
    }
    let span = lines?;
    Some(pack(code_section(
        path,
        None,
        span.start,
        span.end,
        &file_lines,
    )))
}

fn code_section(
    path: &str,
    chain: Option<&str>,
    start: u32,
    end: u32,
    file_lines: &[&str],
) -> String {
    let start_idx = start.saturating_sub(1) as usize;
    let end_idx = (end as usize).min(file_lines.len());
    let available = file_lines.get(start_idx..end_idx).unwrap_or(&[]);
    let shown = available
        .iter()
        .take(MENTION_CODE_LINES)
        .copied()
        .collect::<Vec<_>>();
    let hidden = available.len().saturating_sub(shown.len());
    let title = match chain {
        Some(chain) => format!("## {path} › {chain} · L{start}-{end}"),
        None => format!("## {path} · L{start}-{end}"),
    };
    let lang = fence_lang(path);
    let mut body = format!("{title}\n\n```{lang}\n{}\n```", shown.join("\n"));
    if hidden > 0 {
        body.push_str(&format!("\n\n还有 {hidden} 行，读文件查看"));
    }
    body
}

fn fence_lang(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("rs") => "rust",
        Some("ts") | Some("tsx") => "ts",
        Some("js") | Some("jsx") => "js",
        Some("py") => "python",
        Some("go") => "go",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TurnGuard;
    use crate::knowledge::mentions::{mention_source, symbol_mention_source};
    use crate::reminder::{Reminder, hidden_kind, render_text};
    use crate::session::manager::SessionManager;
    use std::sync::Arc;

    fn rust_file(lines: usize) -> String {
        let mut body = String::from("fn save() {\n");
        for n in 0..lines {
            body.push_str(&format!("    let _v{n} = {n};\n"));
        }
        body.push_str("}\n");
        body
    }

    #[test]
    fn sections_follow_mention_order_and_skip_repeat_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("src/a.rs"),
            "fn save() {\n    let n = 1;\n}\n",
        )
        .unwrap();
        let text = format!(
            "see {} and {} then {}",
            mention_source("missing-node", "missing-node"),
            symbol_mention_source("src/a.rs", Some("fn save"), None, "fn save"),
            "[@ file=\"src/a.rs\" label=\"a.rs\"]",
        );
        let reminder = build(dir.path(), &text, &HashMap::new()).unwrap();
        let rendered = reminder.text();
        assert!(rendered.starts_with(OPENING));
        let node_at = rendered.find("was not found").unwrap();
        let symbol_at = rendered.find("## src/a.rs › fn save").unwrap();
        assert!(node_at < symbol_at);
        assert!(rendered.contains("let n = 1;"));
        assert_eq!(rendered.matches("## src/a.rs").count(), 1);
        let again = format!(
            "{text} {}",
            symbol_mention_source("src/a.rs", Some("fn save"), None, "fn save")
        );
        assert_eq!(
            build(dir.path(), &again, &HashMap::new())
                .unwrap()
                .text()
                .matches("fn save")
                .count(),
            rendered.matches("fn save").count()
        );
    }

    #[test]
    fn truncates_symbol_body_and_explains_a_missing_chain() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/a.rs"), rust_file(100)).unwrap();
        let long = symbol_mention_source("src/a.rs", Some("fn save"), None, "fn save");
        let text = build(dir.path(), &long, &HashMap::new())
            .unwrap()
            .text()
            .to_string();
        assert!(text.contains("还有"));
        assert!(text.contains("读文件查看"));
        let missing = symbol_mention_source("src/a.rs", Some("fn gone"), None, "fn gone");
        let note = build(dir.path(), &missing, &HashMap::new())
            .unwrap()
            .text()
            .to_string();
        assert!(note.contains("was not found"));
    }

    #[test]
    fn ambiguous_chain_says_it_is_not_unique() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/a.rs"), "fn new() {}\nfn new() {}\n").unwrap();
        let text = symbol_mention_source("src/a.rs", Some("fn new"), None, "fn new");
        let note = build(dir.path(), &text, &HashMap::new())
            .unwrap()
            .text()
            .to_string();
        assert!(note.contains("not unique"));
    }

    #[test]
    fn range_only_uses_the_cited_lines() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/a.rs"), "one\ntwo\nthree\n").unwrap();
        let text = symbol_mention_source(
            "src/a.rs",
            None,
            Some(crate::knowledge::mentions::LineSpan { start: 2, end: 3 }),
            "a.rs",
        );
        let rendered = build(dir.path(), &text, &HashMap::new())
            .unwrap()
            .text()
            .to_string();
        assert!(rendered.contains("## src/a.rs · L2-3"));
        assert!(rendered.contains("two\nthree"));
        assert!(!rendered.contains("one"));
    }

    #[test]
    fn file_only_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let text = "[@ file=\"src/a.rs\" label=\"a.rs\"]";
        assert!(build(dir.path(), &text, &HashMap::new()).is_none());
    }

    #[test]
    fn user_row_is_followed_by_a_hidden_mentions_reminder_and_revert_drops_both() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("ws");
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::write(
            workspace.join("src/a.rs"),
            "fn save() {\n    let n = 1;\n}\n",
        )
        .unwrap();
        let db = dir.path().join("sessions.db");
        let mgr = SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            db.to_str().unwrap().to_string(),
        );
        let sid = mgr
            .open_session_sync(workspace.to_str().unwrap(), "default", None)
            .unwrap();
        let text = symbol_mention_source("src/a.rs", Some("fn save"), None, "fn save");
        mgr.append_user_message_with_mentions(&sid, text.as_str(), &workspace)
            .unwrap();
        let events = mgr.data().events_blocking(&sid).unwrap();
        let kinds: Vec<_> = events
            .iter()
            .map(|event| event.event_type.as_str().to_string())
            .collect();
        let user = kinds.iter().position(|kind| kind == "item/user").unwrap();
        assert_eq!(kinds[user + 1], "reminder/mentions");
        assert!(hidden_kind("reminder/mentions"));
        let stored: Reminder = serde_json::from_value(events[user + 1].data.clone()).unwrap();
        let projected = render_text(stored.text());
        assert!(projected.contains("<system-reminder>"));
        assert!(projected.contains("fn save"));
        assert!(projected.contains("let n = 1"));
        let anchor = events[user].seq;
        mgr.entry_revert_to_user_anchor(&sid, anchor as i64)
            .unwrap();
        let after = mgr.data().events_blocking(&sid).unwrap();
        assert!(after.iter().all(|event| event.seq < anchor));
    }

    #[test]
    fn unchanged_symbol_is_not_attached_again_until_the_body_changes() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("ws");
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::write(
            workspace.join("src/a.rs"),
            "fn save() {\n    let n = 1;\n}\n",
        )
        .unwrap();
        let db = dir.path().join("sessions.db");
        let mgr = SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            db.to_str().unwrap().to_string(),
        );
        let sid = mgr
            .open_session_sync(workspace.to_str().unwrap(), "default", None)
            .unwrap();
        let text = symbol_mention_source("src/a.rs", Some("fn save"), None, "fn save");
        mgr.append_user_message_with_mentions(&sid, text.as_str(), &workspace)
            .unwrap();
        mgr.append_user_message_with_mentions(&sid, text.as_str(), &workspace)
            .unwrap();
        let events = mgr.data().events_blocking(&sid).unwrap();
        let mentions: Vec<_> = events
            .iter()
            .filter(|event| event.event_type.as_str() == "reminder/mentions")
            .collect();
        assert_eq!(mentions.len(), 1);

        fs::write(
            workspace.join("src/a.rs"),
            "fn save() {\n    let n = 2;\n}\n",
        )
        .unwrap();
        mgr.append_user_message_with_mentions(&sid, text.as_str(), &workspace)
            .unwrap();
        let events = mgr.data().events_blocking(&sid).unwrap();
        let mentions: Vec<_> = events
            .iter()
            .filter(|event| event.event_type.as_str() == "reminder/mentions")
            .collect();
        assert_eq!(mentions.len(), 2);
        let latest: Reminder = serde_json::from_value(mentions[1].data.clone()).unwrap();
        assert!(latest.text().contains("let n = 2"));
        assert!(!latest.text().contains("let n = 1"));
    }
}
