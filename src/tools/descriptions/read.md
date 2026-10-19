Read one workspace file window. Text files return line-numbered text and structured `data.lines`; supported images return media plus image metadata. Prefer this over shell cat/type.

Use `path` for the file, and `start_line`/`end_line` for a 1-based inclusive window on large files. Past session transcripts are readable at `.litecode/sessions/<session_id>.md`. For discovery by name use glob; for content search use grep.

When `data.truncated` is true, continue from `data.next_start_line`. Structured lines contain raw text without the display line-number prefix.
