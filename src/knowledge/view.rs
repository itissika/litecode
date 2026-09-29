//! Markdown views. Empty sections disappear. Cards are the same shape everywhere.

use std::time::{Duration, SystemTime};

use super::corpus::Node;
use super::document::Status;
use super::mentions::knowledge_preview;
use super::validate::{Issue, Severity};

pub const CARD_LIMIT: usize = 5;
pub const SUMMARY_CHARS: usize = 120;
pub const BODY_CHARS: usize = 400;
pub const PAGE_CHARS: usize = 6_000;

const WELCOME: &str = "\
The knowledge base is human-owned. The agent helps record durable invariants: design principles, architecture, workflows, and goals.\n\
Do not record trivia or details that will change.";

const BUTTONS: &str = "\
> Use knowledge to look things up. Edit bodies with read and edit. Run check after changes. Quote a key or folder that contains spaces.
- `knowledge`: open this board
- `knowledge guide`: syntax, checks, and notes
- `knowledge list [folder]`: list nodes; a folder only narrows the scope
- `knowledge refs <key>`: one node and its citations
- `knowledge check [key or folder]`: list validation issues
- `knowledge create <key> [folder]`: create a pending node
- `knowledge rename <old key> <new key>`: rename a node and its citations
This tool cannot delete a node or change its status. People do that in the knowledge panel. Do not delete the files yourself.";

const GUIDE: &str = r#"# Guide

## Syntax

A markdown file is a node. A folder only nests files. It is not a node and cannot be cited.

The declaration is a `node` fence at the start of the file. Blank lines before it are allowed. Any other text before it means the file has no declaration.

```node
node : seq
status : enabled
summary : one line
```

`node` is the identity, unique in the library. A key may contain letters, digits, `_`, and `-`. A single space may separate words, as in `old key`. It must start with a letter, a digit, or `_`. Consecutive spaces, a slash, quotes, and brackets are not a key.

`status` is `enabled`, `disabled`, or `pending`. A missing status is treated as `enabled`.

`summary` is one line. The body is the markdown after the fence.

The only node citation is `[@ id="seq" label="seq"]`. Lookup uses `id`. `label` is the visible text. `id` comes first, and both values use double quotes.

A file citation is `[@ file="src/a.rs" label="a.rs"]`. It names a workspace path, not a node. A file and a directory both count. `..` and an absolute path are not a path.

A citation inside a fence (` ``` ` or `~~~`), inside inline code, or written as `@seq` or `/src/a.rs` is ordinary text.

The file is named `<key>.md`. If the name differs, the key is still the identity.

`x`, `y`, `w`, and `h` in the fence are canvas layout. Unrecognized field lines are kept. A `ref` line is ignored.

## Check

Errors:

- The status is not `enabled`, `disabled`, or `pending`.
- The declaration is missing, or the key is not valid.
- The same key is declared more than once.
- A citation `id` is the node's own key.
- A citation `id` matches no node.
- A file citation path is not in the workspace.

Warnings:

- The file name does not match the key.
- An enabled node cites a disabled or pending node.

## Notes

- `knowledge create` writes a pending node. This command does not delete a node or change its status.
- `knowledge rename` changes the key, the file name, and citation `id`s. A `label` equal to the old key changes too. Editing `node :` or the file name by hand does not.
- Edit the body in place and leave the fence as it is, so layout fields and unrecognized lines stay.
- Quote a key that contains a space.
- A markdown file with no `node` declaration is not a node. It is left in place.
"#;

pub fn guide() -> &'static str {
    GUIDE
}

pub struct Counts {
    pub total: usize,
    pub verified: usize,
    pub isolated: usize,
    pub enabled: usize,
    pub disabled: usize,
    pub pending: usize,
    pub warning: usize,
    pub error: usize,
}

pub fn counts(nodes: &[&Node], issues: &[Issue], incoming: impl Fn(&str) -> usize) -> Counts {
    let mut out = Counts {
        total: nodes.len(),
        verified: 0,
        isolated: 0,
        enabled: 0,
        disabled: 0,
        pending: 0,
        warning: issues
            .iter()
            .filter(|issue| issue.severity == Severity::Warning)
            .count(),
        error: issues
            .iter()
            .filter(|issue| issue.severity == Severity::Error)
            .count(),
    };
    for node in nodes {
        match node.status {
            Status::Enabled => out.enabled += 1,
            Status::Disabled => out.disabled += 1,
            Status::Pending => out.pending += 1,
        }
        let node_issues: Vec<_> = issues
            .iter()
            .filter(|issue| issue.node_id == node.id)
            .collect();
        let has_error = node_issues.iter().any(|issue| issue.severity == Severity::Error);
        if node.status == Status::Enabled && !has_error {
            out.verified += 1;
        }
        if incoming(&node.key) == 0 && node.relations.is_empty() {
            out.isolated += 1;
        }
    }
    out
}

pub fn stats_line(counts: &Counts) -> String {
    format!(
        "Total {} · Verified {} · Isolated {} · Enabled {} · Disabled {} · Pending {} · Warning {} · Error {}",
        counts.total,
        counts.verified,
        counts.isolated,
        counts.enabled,
        counts.disabled,
        counts.pending,
        counts.warning,
        counts.error
    )
}

const STATS_LEGEND: &str = "\
Verified = enabled and no error. Isolated = no in and no out. Warning and Error count issues; the other numbers count nodes.";

pub fn section(title: &str, body: &str) -> String {
    let body = body.trim();
    if body.is_empty() {
        String::new()
    } else {
        format!("{title}\n{body}")
    }
}

pub fn relative_time(modified: Option<SystemTime>, now: SystemTime) -> String {
    let Some(modified) = modified else {
        return "long".into();
    };
    let age = now.duration_since(modified).unwrap_or(Duration::ZERO);
    let secs = age.as_secs();
    if secs < 60 {
        "now".into()
    } else if secs < 3_600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3_600)
    } else if secs < 7 * 86_400 {
        format!("{}d", secs / 86_400)
    } else if secs < 30 * 86_400 {
        format!("{}w", secs / (7 * 86_400))
    } else if secs < 365 * 86_400 {
        format!("{}mo", secs / (30 * 86_400))
    } else {
        "long".into()
    }
}

pub fn clip(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let kept: String = text.chars().take(max_chars).collect();
    format!("{kept}…")
}

fn display_path(prefix: &str, path: &str) -> String {
    if prefix.is_empty() {
        path.to_string()
    } else if path.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}/{path}")
    }
}

/// One node, numbered. A fault replaces the citation counts.
pub fn node_card(
    index: usize,
    node: &Node,
    issues: &[Issue],
    incoming: usize,
    now: SystemTime,
    path_prefix: &str,
) -> String {
    let when = relative_time(node.modified, now);
    let tail = if node.status == Status::Pending {
        "pending".to_string()
    } else if node.status == Status::Disabled {
        "disabled".to_string()
    } else if !issues.is_empty() {
        "problem".to_string()
    } else {
        format!("in {incoming} out {}", node.relations.len())
    };
    let title = if node.key.trim().is_empty() {
        "(no declaration)".to_string()
    } else {
        format!("**{}**", node.key)
    };
    let path = display_path(path_prefix, &node.path);
    let mut lines = vec![format!("{index}. {title} · `{path}` · {when} · {tail}")];
    for issue in issues {
        lines.push(format!("   - {}: {}", issue.severity.as_str(), issue.message));
    }
    let summary = clip(node.summary.trim(), SUMMARY_CHARS);
    if !summary.is_empty() {
        lines.push(format!("   - {summary}"));
    }
    let body = clip(&knowledge_preview(&node.value, 8), BODY_CHARS);
    if !body.is_empty() && body != summary {
        lines.push(format!("   - {body}"));
    }
    lines.join("\n")
}

pub fn cards<'a>(
    nodes: impl IntoIterator<Item = &'a Node>,
    issues_for: impl Fn(&Node) -> Vec<Issue>,
    incoming: impl Fn(&str) -> usize,
    now: SystemTime,
    path_prefix: &str,
) -> String {
    nodes
        .into_iter()
        .enumerate()
        .map(|(index, node)| {
            let issues = issues_for(node);
            node_card(index + 1, node, &issues, incoming(&node.key), now, path_prefix)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn missing_board() -> String {
    join_sections(&[
        section(
            "# Welcome",
            &format!(
                "{WELCOME}\n\n`knowledge create \"<key>\"` creates `.litecode/knowledge`."
            ),
        ),
        section("# Buttons", BUTTONS),
    ])
}

fn unrecognized_note(paths: &[String]) -> String {
    if paths.is_empty() {
        return String::new();
    }
    let mut lines = vec!["These files are not nodes and were left in place:".to_string()];
    for path in paths {
        lines.push(format!("- `{path}`"));
    }
    lines.join("\n")
}

fn root_note(root: Option<&str>, ignored_root: Option<&str>) -> String {
    let mut lines = Vec::new();
    if let Some(root) = root {
        lines.push(format!("Current knowledge root: `{root}`."));
    }
    if let Some(ignored) = ignored_root {
        lines.push(format!(
            "`{ignored}` is not read. Nodes there are not citation targets."
        ));
    }
    lines.join("\n")
}

fn listed<'a>(
    mut nodes: Vec<&'a Node>,
    issues_for: &impl Fn(&Node) -> Vec<Issue>,
    incoming: &impl Fn(&str) -> usize,
    now: SystemTime,
    path_prefix: &str,
) -> String {
    nodes.sort_by(|a, b| b.modified.cmp(&a.modified));
    let hidden = nodes.len().saturating_sub(CARD_LIMIT);
    nodes.truncate(CARD_LIMIT);
    let body = cards(nodes, issues_for, incoming, now, path_prefix);
    if hidden == 0 || body.is_empty() {
        body
    } else {
        format!(
            "{body}\n\n{hidden} more omitted. Use `knowledge check` or `knowledge list` to see the rest."
        )
    }
}

pub fn dashboard(
    nodes: &[Node],
    issues: &[Issue],
    incoming: impl Fn(&str) -> usize,
    reminder: Option<&str>,
    now: SystemTime,
    root: Option<&str>,
    ignored_root: Option<&str>,
    unknown: &[String],
) -> String {
    let refs: Vec<&Node> = nodes.iter().collect();
    let stats = counts(&refs, issues, &incoming);
    let prefix = root.unwrap_or("");
    let recent: Vec<&Node> = nodes
        .iter()
        .filter(|node| {
            is_verified(node, issues)
                && !is_isolated(node, &incoming)
                && !worth_noting(node, issues, &incoming)
        })
        .collect();
    let noting: Vec<&Node> = nodes
        .iter()
        .filter(|node| worth_noting(node, issues, &incoming))
        .collect();

    let issues_for = |node: &Node| -> Vec<Issue> {
        issues
            .iter()
            .filter(|issue| issue.node_id == node.id)
            .cloned()
            .collect()
    };
    let mut status = format!("{}\n\n{STATS_LEGEND}", stats_line(&stats));
    let note = root_note(root, ignored_root);
    if !note.is_empty() {
        status.push_str("\n\n");
        status.push_str(&note);
    }
    let unrecognized = unrecognized_note(unknown);
    if !unrecognized.is_empty() {
        status.push_str("\n\n");
        status.push_str(&unrecognized);
    }
    join_sections(&[
        section("# Welcome", WELCOME),
        section("# Status", &status),
        section(
            "## Recent sound nodes",
            &listed(recent, &issues_for, &incoming, now, prefix),
        ),
        section(
            "## Worth noting",
            &listed(noting, &issues_for, &incoming, now, prefix),
        ),
        section("# Reminder", reminder.unwrap_or("")),
        section("# Buttons", BUTTONS),
    ])
}

pub fn is_verified(node: &Node, issues: &[Issue]) -> bool {
    node.status == Status::Enabled
        && !issues
            .iter()
            .any(|issue| issue.node_id == node.id && issue.severity == Severity::Error)
}

pub fn is_isolated(node: &Node, incoming: impl Fn(&str) -> usize) -> bool {
    incoming(&node.key) == 0 && node.relations.is_empty()
}

pub fn worth_noting(node: &Node, issues: &[Issue], incoming: impl Fn(&str) -> usize) -> bool {
    node.status != Status::Enabled
        || is_isolated(node, &incoming)
        || issues.iter().any(|issue| issue.node_id == node.id)
}

pub fn page(body: &str, cap: usize) -> (String, Option<String>) {
    if body.chars().count() <= cap {
        return (body.to_string(), None);
    }
    let head: String = body.chars().take(cap).collect();
    let rest: String = body.chars().skip(cap).collect();
    (head, Some(rest))
}

fn join_sections(parts: &[String]) -> String {
    parts
        .iter()
        .filter(|part| !part.is_empty())
        .map(|part| part.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::document::Status;

    fn node(key: &str, status: Status, relations: &[&str], age: Duration) -> Node {
        Node {
            id: key.into(),
            key: key.into(),
            value: format!("{key} 的正文"),
            summary: format!("{key} 摘要"),
            relations: relations.iter().map(|s| (*s).to_string()).collect(),
            status,
            path: format!("{key}.md"),
            folder_id: None,
            x: None,
            y: None,
            w: None,
            h: None,
            invalid_status: None,
            extras: vec![],
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000) - age),
        }
    }

    #[test]
    fn empty_section_disappears_and_recent_skips_faults() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let healthy = node("seq", Status::Enabled, &["session"], Duration::from_secs(3 * 3600));
        let pending = node("draft", Status::Pending, &[], Duration::from_secs(60));
        let nodes = vec![healthy, pending];
        let issues = vec![];
        let incoming = |key: &str| if key == "session" { 1 } else { 0 };
        let board = dashboard(
            &nodes,
            &issues,
            incoming,
            None,
            now,
            Some(".litecode/knowledge"),
            None,
            &[],
        );
        assert!(board.contains("# Welcome"));
        assert!(board.contains("human-owned"));
        assert!(board.contains("Total 2 · Verified 1 · Isolated 1"));
        assert!(board.contains("Warning and Error count issues"));
        assert!(board.contains("Current knowledge root: `.litecode/knowledge`."));
        assert!(board.contains("## Recent sound nodes"));
        assert!(board.contains("**seq**"));
        assert!(board.contains("in 0 out 1"));
        assert!(board.contains(".litecode/knowledge/seq.md"));
        assert!(board.contains("3h"));
        assert!(board.contains("## Worth noting"));
        assert!(board.contains("**draft**"));
        assert!(board.contains("· pending"));
        assert!(!board.contains("# Reminder"));
        assert!(board.contains("# Buttons"));
        assert!(board.contains("Quote a key"));
        assert!(board.contains("cannot delete"));
    }

    #[test]
    fn fault_card_hides_citation_counts() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let node = node("seq", Status::Enabled, &["missing"], Duration::from_secs(30));
        let issues = vec![Issue {
            node_id: "seq".into(),
            severity: Severity::Error,
            code: "dangling_relation".into(),
            message: "Citation \"missing\" does not exist.".into(),
            reference: Some("missing".into()),
        }];
        let card = node_card(1, &node, &issues, 2, now, "");
        assert!(card.contains("error: Citation \"missing\" does not exist."));
        assert!(card.contains("problem"));
        assert!(!card.contains("enabled"));
        assert!(!card.contains("in 2"));
        assert!(card.contains("now"));
    }

    #[test]
    fn empty_key_is_labeled_and_warning_nodes_leave_recent() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mut blank = node("seq", Status::Enabled, &[], Duration::from_secs(10));
        blank.key.clear();
        blank.id = "blank.md".into();
        let card = node_card(1, &blank, &[], 0, now, "knowledge");
        assert!(card.contains("(no declaration)"));
        assert!(card.contains("`knowledge/seq.md`"));
        assert!(!card.contains("****"));

        let healthy = node("seq", Status::Enabled, &["hub"], Duration::from_secs(30));
        let mut warned = node("warn", Status::Enabled, &["hub"], Duration::from_secs(10));
        warned.id = "warn".into();
        let nodes = vec![healthy, warned];
        let issues = vec![Issue {
            node_id: "warn".into(),
            severity: Severity::Warning,
            code: "filename_mismatch".into(),
            message: "Filename \"warn\" does not match declaration \"warn\".".into(),
            reference: Some("warn".into()),
        }];
        let incoming = |_: &str| 1;
        let board = dashboard(
            &nodes,
            &issues,
            incoming,
            None,
            now,
            Some("knowledge"),
            None,
            &[],
        );
        let recent = board.split("## Worth noting").next().unwrap();
        assert!(recent.contains("**seq**"));
        assert!(!recent.contains("**warn**"));
        assert!(board.contains("**warn**"));
        assert!(board.contains("· problem"));
    }

    #[test]
    fn sections_say_how_many_cards_were_omitted() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mut nodes = Vec::new();
        for index in 0..7 {
            nodes.push(node(
                &format!("n{index}"),
                Status::Enabled,
                &["hub"],
                Duration::from_secs(index * 60),
            ));
        }
        let board = dashboard(&nodes, &[], |_| 1, None, now, None, None, &[]);
        assert!(board.contains("2 more omitted"));
        assert!(board.contains("knowledge check"));
        assert!(board.contains("knowledge list"));
    }

    #[test]
    fn unrecognized_files_are_named_in_status_and_are_not_nodes() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let board = dashboard(
            &[],
            &[],
            |_| 0,
            None,
            now,
            Some(".litecode/knowledge"),
            None,
            &["notes.md".into(), "内核/scratch.md".into()],
        );
        assert!(board.contains("These files are not nodes and were left in place:"));
        assert!(board.contains("`notes.md`"));
        assert!(board.contains("`内核/scratch.md`"));
        assert!(board.contains("Total 0"));
        assert!(!board.contains("(no declaration)"));
        assert!(guide().contains("treated as `enabled`"));
        assert!(!guide().contains("[[node"));
    }

    #[test]
    fn months_use_mo_and_minutes_use_m() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000_000);
        assert_eq!(
            relative_time(Some(now - Duration::from_secs(90)), now),
            "1m"
        );
        assert_eq!(
            relative_time(Some(now - Duration::from_secs(40 * 86_400)), now),
            "1mo"
        );
        assert_eq!(
            relative_time(Some(now - Duration::from_secs(400 * 86_400)), now),
            "long"
        );
    }

    #[test]
    fn page_split_keeps_the_tail() {
        let (head, rest) = page("abcdef", 3);
        assert_eq!(head, "abc");
        assert_eq!(rest.as_deref(), Some("def"));
    }
}
