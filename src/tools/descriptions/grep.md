Search file contents with a regular expression, case-sensitive by default. The default corpus applies workspace search exclusions and `.gitignore`. Set include_ignored=true to search gitignored and workspace-excluded paths; the tool never lifts those filters implicitly after a zero-hit search. Returns one bounded page of structured matches plus a text view. The text view is presentation only; it never changes which matches belong to the page.

Views: `lines` (default), `context`, and `expanded`. Use `expanded` only when enclosing code is worth the extra output.

Pagination: `limit` is 1-100 and defaults to 20. When `has_more` is true, pass `next_cursor` back as `cursor` for the next page. The cursor is opaque.

No spill file is created. For full context or untruncated lines, use read on the matched file.

Example: {"pattern":"fn main","path":"src","view":"context"}
