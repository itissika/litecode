Search file contents. The result is `data.base`, `data.matches`, `data.page`, `data.searched`, and sometimes `data.reason`. Matches are `{path, start_line, end_line, line_text, line_truncated}`. Read the file for surrounding lines.

`pattern` is a regular expression unless `literal` is true. An invalid pattern fails with `error.code` `"invalid_regex"` and `retryable: true`. `details` include `pattern`, `regex_error`, `position`, `suggested_literal`, and `suggested_pattern`. The tool does not rewrite the pattern. Set `literal` true to search for the pattern as plain text.

The default corpus applies workspace search exclusions and `.gitignore`. Set `include_ignored` true to search gitignored and workspace-excluded paths. `limit` is 1-100 and defaults to 20. When `data.page.has_more` is true, pass `data.page.next` back as `cursor`. The cursor is bound to this pattern, path, glob, filters, and a corpus version. A different query, or the same query after the files change, returns `error.code` `"stale_cursor"`. A capped match line adds diagnostic `line_truncated`. An empty result sets `data.reason` to `no_match`, `corpus_empty`, `glob_empty`, `path_excluded`, `binary_file`, or `file_not_searched`.

Example: {"pattern":"fn main","path":"src"}
