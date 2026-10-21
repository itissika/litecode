List diagnostics for one source file inside `run_code`. Pass `{ file_path }` only (object bag) — no `line` / `character` / `text` / `action`. Code-only. Resolves `{ items, message }`. Each item is `{ severity, path, line, character, message }` with **string** `severity` names (for example `Error`, `Warning`, `Information`, `Hint`) and **1-based** line and character when a range exists. Flattened only — no raw LSP JSON, no auto-injection into the `run_code` head (log or return `items` if needed).

A clean file **resolves** with empty `items` and a short `message`. Invalid path / non-file / engine-not-ready / server failure → `ToolError` (short retry hint when not ready; no fixed seconds); cancel → `cancelled`. No `ok: false` on this tool. At most **100** items; when truncated, `message` says so — no separate `truncated` field. Example:
const diags = await tools.diagnostics({ file_path: "src/a.rs" });
if (diags.items.length === 0) return { ok: true, message: diags.message };
const err = diags.items.find((d) => d.severity === "Error") ?? diags.items[0];
const slice = await tools.read({ path: err.path, start_line: err.line, limit: 30 });
return { diagnostic: err, preview: slice.lines, message: diags.message };
