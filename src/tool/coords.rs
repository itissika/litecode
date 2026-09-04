//! Agent-visible file-line and result-offset coordinates.
//!
//! Two spaces, kept distinct:
//! - **File lines** are 1-based inclusive. Same number in `read` `start_line` /
//!   `end_line`, `lsp` `line`, and `L12` / `L12-15` labels.
//! - **Hit offset** is a 0-based index into an agent result list (`grep`,
//!   `session_search`). It is never a file line.
//!
//! Columns are not part of the agent view. File body is the only place that
//! prints a line number next to source text (`     N: text`). Hits and headings
//! use `L12`. Locations with a path use `{path}:L12`.

/// How `read` prefixes each source line. Edit must not copy this into old_string.
pub const FILE_LINE_PREFIX_HINT: &str = "read's line-number prefix";

/// Numbered source line: the only file-body form the agent sees.
pub fn format_file_line(num: u32, text: &str) -> String {
    format!("{:6}: {}\n", num, text)
}

/// Hit or heading: `L12` or `L12-15` (inclusive, one `L`).
pub fn format_line_label(start: u32, end: u32) -> String {
    if start == end {
        format!("L{start}")
    } else {
        format!("L{start}-{end}")
    }
}

/// Location with path: `{path}:L12` or `{path}:L12-15`.
pub fn format_path_lines(path: &str, start: u32, end: u32) -> String {
    format!("{path}:{}", format_line_label(start, end))
}

/// Comma-separated hit labels: `L1, L3`. Caps at 8 then `(and N more)`.
pub fn format_line_list(lines: &[u32]) -> String {
    const MAX: usize = 8;
    if lines.len() <= MAX {
        return lines
            .iter()
            .map(|n| format!("L{n}"))
            .collect::<Vec<_>>()
            .join(", ");
    }
    let shown: Vec<String> = lines[..MAX].iter().map(|n| format!("L{n}")).collect();
    format!("{} (and {} more)", shown.join(", "), lines.len() - MAX)
}

/// Continuation token when more hits remain. `next` is the 0-based offset to pass.
pub fn format_offset_more(next: usize) -> String {
    format!("(more hits; offset: {next})")
}

/// Footer when this page started after offset 0 and nothing remains.
pub fn format_offset_done(offset: usize) -> String {
    format!("(showing hits from offset {offset}; no further pages)")
}

/// File-window continuation for `read`. `next` is the next `start_line`.
pub fn format_file_window_footer(
    first: u32,
    last: u32,
    total: u32,
    next: u32,
    output_cap: bool,
) -> String {
    if output_cap {
        format!(
            "[showing lines {first}-{last} of {total} — output cap. Use start_line={next} to continue]"
        )
    } else {
        format!("[showing lines {first}-{last} of {total}. Use start_line={next} to continue]")
    }
}

/// Append the offset footer to an already-built page body.
///
/// `shown` is how many hits are in this page; `total` is the full hit count.
pub fn attach_offset_footer(body: &str, offset: usize, shown: usize, total: usize) -> String {
    let remaining = total.saturating_sub(offset);
    let footer = if shown < remaining {
        Some(format_offset_more(offset + shown))
    } else if offset > 0 {
        Some(format_offset_done(offset))
    } else {
        None
    };
    match footer {
        None => body.to_string(),
        Some(footer) if body.is_empty() => footer,
        Some(footer) if body.ends_with('\n') => format!("{body}{footer}"),
        Some(footer) => format!("{body}\n{footer}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_line_matches_read_body() {
        assert_eq!(format_file_line(1, "a"), "     1: a\n");
        assert_eq!(format_file_line(12, "x"), "    12: x\n");
        assert_eq!(format_file_line(1500, "z"), "  1500: z\n");
    }

    #[test]
    fn line_label_single_and_range() {
        assert_eq!(format_line_label(12, 12), "L12");
        assert_eq!(format_line_label(12, 15), "L12-15");
    }

    #[test]
    fn path_lines_single_and_range() {
        assert_eq!(format_path_lines("src/a.rs", 12, 12), "src/a.rs:L12");
        assert_eq!(format_path_lines("src/a.rs", 12, 15), "src/a.rs:L12-15");
    }

    #[test]
    fn line_list_labels_and_caps() {
        assert_eq!(format_line_list(&[1, 3]), "L1, L3");
        let many: Vec<u32> = (1..=10).collect();
        assert_eq!(
            format_line_list(&many),
            "L1, L2, L3, L4, L5, L6, L7, L8 (and 2 more)"
        );
    }

    #[test]
    fn offset_footers_are_exact() {
        assert_eq!(format_offset_more(10), "(more hits; offset: 10)");
        assert_eq!(
            format_offset_done(10),
            "(showing hits from offset 10; no further pages)"
        );
    }

    #[test]
    fn file_window_footer_is_exact() {
        assert_eq!(
            format_file_window_footer(1, 1500, 1600, 1501, false),
            "[showing lines 1-1500 of 1600. Use start_line=1501 to continue]"
        );
        assert_eq!(
            format_file_window_footer(1, 1, 3, 2, true),
            "[showing lines 1-1 of 3 — output cap. Use start_line=2 to continue]"
        );
    }

    #[test]
    fn attach_offset_footer_first_complete_has_none() {
        assert_eq!(attach_offset_footer("body\n", 0, 3, 3), "body\n");
    }

    #[test]
    fn attach_offset_footer_more_and_last_page() {
        assert_eq!(
            attach_offset_footer("body", 0, 2, 5),
            "body\n(more hits; offset: 2)"
        );
        assert_eq!(
            attach_offset_footer("body\n", 2, 3, 5),
            "body\n(showing hits from offset 2; no further pages)"
        );
    }
}
