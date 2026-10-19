Find files by path glob when you need names/paths, not contents. Prefer workspace-relative path scopes; with path set, keep pattern relative to that directory (e.g. path=src and pattern=**/*.rs). Default filters hide gitignored and workspace-excluded paths; set include_ignored=true to include them. For content matches use grep.

Results are available as structured data: `paths`, `total`, `truncated`, `effective_pattern`, and `searched_path`.
