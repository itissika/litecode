Jump to the definition of the symbol at a file position inside `run_code`. Pass an object bag: `file_path` (workspace-relative source file), and **1-based** `line` + **1-based** UTF-16 `character` (end-of-line caret allowed: `1..=utf16_len+1`). There is no `action` bag, no row-inline `text` locator, and no Direct twin — this is Code-only. Resolves `{ locations, message }`. Each location guarantees `{ path, line, character }` (workspace-relative `path`, **1-based** start line and character). `end_line` / `end_character` may appear when the runtime supplies them — do not require them. No raw LSP JSON.

Empty `locations` with an explanatory `message` still **resolves** (nothing at that position). Bad bags (missing fields, `line`/`character` < 1, wrong types, character past the line) and paths outside the workspace / non-files resolve `ok: false` (`invalid_args` / `not_found` / `permission`); engine-not-ready / server failures reject `ToolError` (not-ready includes a short retry hint — no fixed wait seconds in this description). Cancellation rejects `cancelled`. Prefer a precise character on the token (from a prior `read` / `grep` line). At most **100** locations; if truncated, `message` says so (for example `Returning first 100 locations (more omitted).`). Example:
const found = await tools.grep({
  pattern: "fn start",
  path: "src",
  literal: true,
});
if (found.hits.length === 0) return { ok: false, message: found.message };
const hit = found.hits[0];
const col = hit.text.indexOf("start") + 1; // 1-based character on that line
const def = await tools.go_to_definition({
  file_path: hit.path,
  line: hit.line,
  character: col,
});
return { locations: def.locations, message: def.message };
