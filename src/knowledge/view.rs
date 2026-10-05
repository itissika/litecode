//! Markdown views. Empty sections disappear. Cards are the same shape everywhere.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use super::corpus::Node;
use super::document::Status;
use super::mentions::knowledge_preview;
use super::validate::{Issue, Severity};

pub const CARD_LIMIT: usize = 5;
pub const SUMMARY_CHARS: usize = 120;
pub const PAGE_CHARS: usize = 6_000;

const WELCOME: &str = "\
The knowledge base is human-owned. The agent helps record durable invariants: design principles, architecture, workflows, and goals.\n\
Do not record trivia or details that will change.";

const BUTTONS: &str = "\
> This tool wraps the common commands. If they are not enough, combine them with other tools to read and edit. Run check after changes. Quote a key or folder that contains spaces.
- `knowledge guide`: syntax, checks, and notes
- `knowledge list [folder]`: list nodes; a folder only narrows the scope
- `knowledge refs <key>`: that node's full file, plus summaries for the nodes that cite it and the nodes it cites
- `knowledge check [key or folder]`: list issues
- `knowledge create <key> [folder]`: create a pending node; the folder is optional
- `knowledge rename <old key> <new key>`: rename a node and its citations
This tool cannot delete a node or change its status. The user does that in the knowledge panel. Do not delete the files yourself.";

const GUIDE: &str = r#"# Guide

## Syntax

A markdown file is a node. A folder only nests files. It is not a node and cannot be cited.

The declaration is a `node` fence at the start of the file. Blank lines before it are allowed. Any other text before it means the file has no declaration.

```node
node : seq
status : enabled
summary : one line
```

`node` is the identity, unique in the knowledge base. A key may contain letters, digits, `_`, and `-`. A single space may separate words, as in `old key`. It must start with a letter, a digit, or `_`. Consecutive spaces, a slash, quotes, and brackets are not a key.

`status` is `enabled`, `disabled`, or `pending`. A missing status is treated as `enabled`.

`summary` is one line. The body is the markdown after the fence.

The only citation that names a node is `[@ key="seq"]`. Lookup uses the key.

A file citation is `[@ file="src/a.rs"]`. It names a workspace path, not a node. A file and a directory both count. `..` and an absolute path are not a path.

A symbol citation is `[@ file="src/a.rs" symbol="impl Store › fn save"]`. `symbol` is the ancestor chain and is the identity. `lines` is optional and is not checked. A chain that is missing or not unique is an error. When git can answer, a chain whose body on disk differs from that file at HEAD is a warning. Commit that file and the warning goes away. Without git, only existence is checked.

A line range with no symbol is `[@ file="src/a.rs" lines="4-9"]`.

A citation inside a fence (` ``` ` or `~~~`), inside inline code, or written as `@seq` or `/src/a.rs` is ordinary text.

The file is named `<key>.md`. If the name differs, the key is still the identity.

`x`, `y`, `w`, and `h` in the fence are the human panel's layout; ignore them when reading. Unrecognized field lines are kept.

## Check

Errors:

- The status is not `enabled`, `disabled`, or `pending`.
- The declaration is missing, or the key is not valid.
- The same key is declared more than once.
- A node cites its own key.
- A citation `key` matches no node.
- A file citation path is not in the workspace.
- A symbol citation's chain is missing from that file, or the same chain occurs more than once.

Warnings:

- The file name does not match the key.
- An enabled node cites a disabled or pending node.
- A symbol citation's body on disk differs from that file at HEAD.

## Notes

- `knowledge create` writes a pending node. This command does not delete a node or change its status.
- `knowledge rename` changes the key, the file name, and citation keys. A node citation is rewritten as `[@ key="new"]`. Editing `node :` or the file name by hand does not.
- A card's `cites` and `cited by` count node citations only. File and symbol citations are not counted.
- `knowledge refs` shows the file as stored, so a citation there stays in the bracket form.
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
        let has_error = node_issues
            .iter()
            .any(|issue| issue.severity == Severity::Error);
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
Verified = enabled and no error. Isolated = nothing cites it and it cites nothing. Warning and Error count issues; the other numbers count nodes.";

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

/// One node's file as stored. The path and any issues sit above the text.
pub fn node_file(path_prefix: &str, path: &str, issues: &[Issue], markdown: &str) -> String {
    let shown = display_path(path_prefix, path);
    let mut head = vec![format!("`{shown}`")];
    for issue in issues {
        head.push(format!("- {}: {}", issue.severity.as_str(), issue.message));
    }
    let body = markdown.trim_start_matches('\u{feff}').trim_end();
    if body.is_empty() {
        head.join("\n")
    } else {
        format!("{}\n\n{body}", head.join("\n"))
    }
}

/// One node row. An issue replaces the citation counts.
pub fn node_row(
    node: &Node,
    issues: &[Issue],
    incoming: usize,
    now: SystemTime,
    path_prefix: &str,
) -> String {
    row_at(node, issues, incoming, now, path_prefix, 0)
}

/// Relation lists and write receipts: the same row, in the given order.
pub fn flat_rows<'a>(
    nodes: impl IntoIterator<Item = &'a Node>,
    issues_for: impl Fn(&Node) -> Vec<Issue>,
    incoming: impl Fn(&str) -> usize,
    now: SystemTime,
    path_prefix: &str,
) -> String {
    nodes
        .into_iter()
        .map(|node| {
            let issues = issues_for(node);
            node_row(node, &issues, incoming(&node.key), now, path_prefix)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The library tree. `scope` starts inside that folder. Loose nodes at the
/// library root go under `(top level)`, after the named folders.
pub fn folder_tree<'a>(
    nodes: impl IntoIterator<Item = &'a Node>,
    scope: Option<&str>,
    issues_for: impl Fn(&Node) -> Vec<Issue>,
    incoming: impl Fn(&str) -> usize,
    now: SystemTime,
    path_prefix: &str,
) -> String {
    let mut root = Dir::default();
    let mut top_level = Vec::new();
    for node in nodes {
        match place(node, scope) {
            Place::Skip => {}
            Place::Top => top_level.push(node),
            Place::Direct => root.nodes.push(node),
            Place::Nested(parts) => insert(&mut root, &parts, node),
        }
    }
    let mut lines = Vec::new();
    render_dir(
        &root,
        0,
        &issues_for,
        &incoming,
        now,
        path_prefix,
        &mut lines,
    );
    if !top_level.is_empty() {
        top_level.sort_by(|a, b| a.path.cmp(&b.path));
        lines.push("- (top level)".to_string());
        for node in top_level {
            let issues = issues_for(node);
            lines.push(row_at(
                node,
                &issues,
                incoming(&node.key),
                now,
                path_prefix,
                2,
            ));
        }
    }
    lines.join("\n")
}

#[derive(Default)]
struct Dir<'a> {
    folders: BTreeMap<String, Dir<'a>>,
    nodes: Vec<&'a Node>,
}

enum Place {
    Skip,
    Top,
    Direct,
    Nested(Vec<String>),
}

fn place(node: &Node, scope: Option<&str>) -> Place {
    let folder = node.folder_id.as_deref().unwrap_or("");
    let rest = match scope.map(|item| item.trim_matches('/')) {
        Some("") | None => folder,
        Some(scope) if folder == scope => "",
        Some(scope) => match folder.strip_prefix(&format!("{scope}/")) {
            Some(rest) => rest,
            None => return Place::Skip,
        },
    };
    if scope.is_none() && rest.is_empty() {
        Place::Top
    } else if rest.is_empty() {
        Place::Direct
    } else {
        Place::Nested(rest.split('/').map(str::to_string).collect())
    }
}

fn insert<'a>(dir: &mut Dir<'a>, parts: &[String], node: &'a Node) {
    if parts.is_empty() {
        dir.nodes.push(node);
        return;
    }
    insert(
        dir.folders.entry(parts[0].clone()).or_default(),
        &parts[1..],
        node,
    );
}

fn render_dir<'a>(
    dir: &Dir<'a>,
    indent: usize,
    issues_for: &impl Fn(&Node) -> Vec<Issue>,
    incoming: &impl Fn(&str) -> usize,
    now: SystemTime,
    path_prefix: &str,
    lines: &mut Vec<String>,
) {
    let pad = " ".repeat(indent);
    for (name, child) in &dir.folders {
        lines.push(format!("{pad}- {name}/"));
        render_dir(
            child,
            indent + 2,
            issues_for,
            incoming,
            now,
            path_prefix,
            lines,
        );
    }
    let mut nodes = dir.nodes.clone();
    nodes.sort_by(|a, b| a.path.cmp(&b.path));
    for node in nodes {
        let issues = issues_for(node);
        lines.push(row_at(
            node,
            &issues,
            incoming(&node.key),
            now,
            path_prefix,
            indent,
        ));
    }
}

fn row_at(
    node: &Node,
    issues: &[Issue],
    incoming: usize,
    now: SystemTime,
    path_prefix: &str,
    indent: usize,
) -> String {
    let when = relative_time(node.modified, now);
    let tail = if node.status == Status::Pending {
        "pending".to_string()
    } else if node.status == Status::Disabled {
        "disabled".to_string()
    } else if !issues.is_empty() {
        "issue".to_string()
    } else {
        format!("cites {} · cited by {incoming}", node.relations.len())
    };
    let title = if node.key.trim().is_empty() {
        "(no declaration)".to_string()
    } else {
        format!("**{}**", node.key)
    };
    let path = display_path(path_prefix, &node.path);
    let pad = " ".repeat(indent);
    let mut lines = vec![format!("{pad}- {title} · `{path}` · {when} · {tail}")];
    let cont = " ".repeat(indent + 2);
    for issue in issues {
        lines.push(format!(
            "{cont}- {}: {}",
            issue.severity.as_str(),
            issue.message
        ));
    }
    let summary = summary_line(node);
    if !summary.is_empty() {
        lines.push(format!("{cont}- {summary}"));
    }
    lines.join("\n")
}

fn summary_line(node: &Node) -> String {
    let raw = node.summary.trim();
    let text = if raw.is_empty() {
        knowledge_preview(&node.value, 1)
    } else {
        super::mentions::show_facts(raw)
    };
    clip(&text, SUMMARY_CHARS)
}

const STARTER: &str = "\
# Start

The knowledge base has no nodes yet. Draft 2 to 5 nodes and stop there.

- Do not interview the user. Infer the nodes from the workspace and the current conversation.
- Make 2 to 5 nodes, in 1 or 2 folders. Keep it small enough to read.
- Record principles, boundaries, and workflows. Do not record trivia, details that will change, or a catalog of the code.
- Use `knowledge create`. If there is no knowledge base yet, the first create creates `.litecode/knowledge`. Leave each node pending. Do not enable a node, and do not delete one.
- Then stop. Ask the user to look and correct. Do not add more until they have looked.
";

/// Empty library: how to start, plus the syntax and the commands.
pub fn missing_board() -> String {
    join_sections(&[
        STARTER.trim_end().to_string(),
        GUIDE.trim_end().to_string(),
        section("# Commands", BUTTONS),
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

fn root_note(root: Option<&str>) -> String {
    match root {
        Some(root) => format!("Current knowledge root: `{root}`."),
        None => String::new(),
    }
}

fn listed<'a>(
    mut nodes: Vec<&'a Node>,
    issues_for: &impl Fn(&Node) -> Vec<Issue>,
    incoming: &impl Fn(&str) -> usize,
    now: SystemTime,
    path_prefix: &str,
    mention_check: bool,
) -> String {
    nodes.sort_by(|a, b| b.modified.cmp(&a.modified));
    let hidden = nodes.len().saturating_sub(CARD_LIMIT);
    nodes.truncate(CARD_LIMIT);
    let body = folder_tree(nodes, None, issues_for, incoming, now, path_prefix);
    if hidden == 0 || body.is_empty() {
        body
    } else if mention_check {
        format!(
            "{body}\n\n{hidden} more omitted. Use `knowledge check` or `knowledge list` to see the rest."
        )
    } else {
        format!("{body}\n\n{hidden} more omitted. Use `knowledge list` to see the rest.")
    }
}

pub fn dashboard(
    nodes: &[Node],
    issues: &[Issue],
    incoming: impl Fn(&str) -> usize,
    reminder: Option<&str>,
    now: SystemTime,
    root: Option<&str>,
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
    let mention_check = stats.warning > 0 || stats.error > 0;
    let mut status = format!("{}\n\n{STATS_LEGEND}", stats_line(&stats));
    let note = root_note(root);
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
            "## Recent nodes",
            &listed(recent, &issues_for, &incoming, now, prefix, mention_check),
        ),
        section(
            "## Worth noting",
            &listed(noting, &issues_for, &incoming, now, prefix, mention_check),
        ),
        section("# Notice", reminder.unwrap_or("")),
        section("# Commands", BUTTONS),
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
        let healthy = node(
            "seq",
            Status::Enabled,
            &["session"],
            Duration::from_secs(3 * 3600),
        );
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
            &[],
        );
        assert!(board.contains("# Welcome"));
        assert!(board.contains("human-owned"));
        assert!(board.contains("Total 2 · Verified 1 · Isolated 1"));
        assert!(board.contains("Warning and Error count issues"));
        assert!(board.contains("Current knowledge root: `.litecode/knowledge`."));
        assert!(board.contains("## Recent nodes"));
        assert!(board.contains("**seq**"));
        assert!(board.contains("cites 1 · cited by 0"));
        assert!(board.contains(".litecode/knowledge/seq.md"));
        assert!(board.contains("3h"));
        assert!(board.contains("## Worth noting"));
        assert!(board.contains("**draft**"));
        assert!(board.contains("· pending"));
        assert!(board.contains("- (top level)"));
        assert!(board.contains("# Commands"));
        assert!(!board.contains("# Notice"));
        assert!(board.contains("Quote a key"));
        assert!(board.contains("cannot delete"));
    }

    #[test]
    fn fault_card_hides_citation_counts() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let node = node(
            "seq",
            Status::Enabled,
            &["missing"],
            Duration::from_secs(30),
        );
        let issues = vec![Issue {
            node_id: "seq".into(),
            severity: Severity::Error,
            code: "dangling_relation".into(),
            message: "Citation \"missing\" does not exist.".into(),
            reference: Some("missing".into()),
        }];
        let card = node_row(&node, &issues, 2, now, "");
        assert!(card.contains("error: Citation \"missing\" does not exist."));
        assert!(card.contains("· issue"));
        assert!(!card.contains("enabled"));
        assert!(!card.contains("cited by 2"));
        assert!(card.contains("now"));
    }

    #[test]
    fn empty_key_is_labeled_and_warning_nodes_leave_recent() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mut blank = node("seq", Status::Enabled, &[], Duration::from_secs(10));
        blank.key.clear();
        blank.id = "blank.md".into();
        let card = node_row(&blank, &[], 0, now, "knowledge");
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
        let board = dashboard(&nodes, &issues, incoming, None, now, Some("knowledge"), &[]);
        let recent = board.split("## Worth noting").next().unwrap();
        assert!(recent.contains("**seq**"));
        assert!(!recent.contains("**warn**"));
        assert!(board.contains("**warn**"));
        assert!(board.contains("· issue"));
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
        let board = dashboard(&nodes, &[], |_| 1, None, now, None, &[]);
        assert!(board.contains("2 more omitted. Use `knowledge list` to see the rest."));
        assert!(!board.contains("2 more omitted. Use `knowledge check`"));
        let warned = Issue {
            node_id: "n0".into(),
            severity: Severity::Warning,
            code: "filename_mismatch".into(),
            message: "Filename \"n0\" does not match declaration \"n0\".".into(),
            reference: Some("n0".into()),
        };
        let with_warning = dashboard(&nodes, &[warned], |_| 1, None, now, None, &[]);
        assert!(with_warning.contains(
            "1 more omitted. Use `knowledge check` or `knowledge list` to see the rest."
        ));
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
            &["notes.md".into(), "内核/scratch.md".into()],
        );
        assert!(board.contains("These files are not nodes and were left in place:"));
        assert!(board.contains("`notes.md`"));
        assert!(board.contains("`内核/scratch.md`"));
        assert!(board.contains("Total 0"));
        assert!(!board.contains("(no declaration)"));
        assert!(guide().contains("## Syntax"));
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
