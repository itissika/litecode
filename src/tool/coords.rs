//! Agent-visible file-line and result-offset coordinates.
//!
//! Two spaces, kept distinct:
//! - **File lines** are 1-based inclusive. Same number in `read` `start_line` /
//!   `end_line`, LSP tools' `line`/`character`, and `L12` / `L12-15` labels.
//! - **Hit offset** is a 0-based index into an agent result list (`grep`,
//!   `session_search`). It is never a file line.
//!
//! Columns are not part of the agent view. File body is the only place that
//! prints a line number next to source text (`     N: text`). Hits and headings
//! use `L12`. Locations with a path use `{path}:L12`.

/// Transcript / read-output file-body form that prefixes each source line (`     N: text`).
/// Code-surface `read` resolves bare `lines[].text` with no prefix; do not paste prefixed
/// transcript or read output into edit `old_string`.
pub const FILE_LINE_PREFIX_HINT: &str =
    "line-number prefixes pasted from transcript or read output";

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
    format!("More hits: pass offset={next}.")
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
    

}
