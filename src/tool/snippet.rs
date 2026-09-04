//! Agent-visible code snippet page (grep `content` / code_search).
//!
//! One shape: file sections, `###` headings with `L` labels, fenced source.
//! Callers decide the window (ancestor, ±context, or indexed chunk).

use super::format_line_label;

/// Display cap per snippet line (chars).
pub(crate) const SNIPPET_LINE_MAX_CHARS: usize = 240;

#[derive(Debug, Clone)]
pub struct SnippetSection {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub breadcrumb: Option<String>,
    pub text: String,
    pub remaining_lines: u32,
}

/// Group consecutive same-path sections: `## Matches in {path}` then headings.
pub fn format_snippet_sections(sections: &[SnippetSection]) -> String {
    let mut body = String::new();
    let mut current_path: Option<&str> = None;
    for section in sections {
        if current_path != Some(section.path.as_str()) {
            body.push_str(&format!("\n## Matches in {}\n", section.path));
            current_path = Some(section.path.as_str());
        }
        let line_label = format_line_label(section.start_line, section.end_line);
        match section.breadcrumb.as_deref() {
            Some(crumb) => body.push_str(&format!("\n### {crumb} › {line_label}\n")),
            None => body.push_str(&format!("\n### {line_label}\n")),
        }
        body.push_str(&fence_block(&section.text));
        if section.remaining_lines > 0 {
            body.push_str(&format!(
                "\n{} lines remaining in ancestor node. Read the file to see all.\n",
                section.remaining_lines
            ));
        }
    }
    body
}

pub(crate) fn fence_block(snippet: &str) -> String {
    let snippet = truncate_snippet_lines(snippet);
    let mut longest = 2usize;
    let mut run = 0usize;
    for ch in snippet.chars() {
        if ch == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let ticks = "`".repeat(longest + 1);
    format!("{ticks}\n{snippet}\n{ticks}\n")
}

pub(crate) fn truncate_snippet_lines(snippet: &str) -> String {
    snippet
        .lines()
        .map(|line| {
            if line.chars().count() <= SNIPPET_LINE_MAX_CHARS {
                line.to_string()
            } else {
                let kept: String = line.chars().take(SNIPPET_LINE_MAX_CHARS).collect();
                format!("{kept}… (line truncated)")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_match_grep_expanded_shape() {
        let out = format_snippet_sections(&[
            SnippetSection {
                path: "a.rs".into(),
                start_line: 1,
                end_line: 4,
                breadcrumb: Some("fn save".into()),
                text: "fn save() {\n    ok\n}".into(),
                remaining_lines: 0,
            },
            SnippetSection {
                path: "b.rs".into(),
                start_line: 12,
                end_line: 12,
                breadcrumb: None,
                text: "let x = 1;".into(),
                remaining_lines: 3,
            },
        ]);
        assert!(out.contains("## Matches in a.rs"), "{out}");
        assert!(out.contains("### fn save › L1-4"), "{out}");
        assert!(out.contains("## Matches in b.rs"), "{out}");
        assert!(out.contains("### L12"), "{out}");
        assert!(
            out.contains("3 lines remaining in ancestor node. Read the file to see all."),
            "{out}"
        );
        assert!(out.contains("```\nfn save()"), "{out}");
    }
}
