Search file contents. The result is `base`, `matches`, `page`, `searched`, and sometimes `reason`. Matches are `{path, start_line, end_line, line_text, line_truncated}`.

`pattern` is a regular expression unless `literal` is true. An invalid pattern rejects `invalid_regex`. Pass `literal: true` to search for the pattern as plain text. A blank `glob` rejects `invalid_input`.

The default corpus applies workspace search exclusions and `.gitignore`. Set `include_ignored` true to search gitignored and workspace-excluded paths. `limit` is 1-100 and defaults to 20. When `page.has_more` is true, pass `page.next` back as `cursor`. The cursor is bound to this pattern, path, glob, filters, and a corpus version. A different query, or the same query after the files change, rejects `ToolError` with `code` `stale_cursor`. A capped match line sets `line_truncated`. An empty result sets `reason` to `no_match`, `corpus_empty`, `glob_empty`, `path_excluded`, `binary_file`, or `file_not_searched`.

Example: {"pattern":"fn main","path":"src"}
