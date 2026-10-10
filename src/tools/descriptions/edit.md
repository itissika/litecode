Replace text in one file. Prefer this over sed or awk for edits.
Multiple files: multiple calls (one file_path each). Pass file_path and a non-empty edits array; each item is old_string, new_string, and optional replace_all (default false). Empty new_string deletes old_string.
Matching tries exact first (CRLF/LF and common typography such as smart quotes and dashes are equivalent), then a unique high-confidence line-aligned fuzzy match. replace_all applies every exact match and never auto-replaces multiple fuzzy candidates.
{{SNAPSHOT_RULE}}
Example (one edit): {"file_path":"src/a.rs","edits":[{"old_string":"fn start() {}","new_string":"fn main() {}"}]}
Example (several): {"file_path":"src/a.rs","edits":[{"old_string":"foo","new_string":"bar"},{"old_string":"old_api(","new_string":"new_api(","replace_all":true}]}
