Search symbols across the workspace inside `run_code`. Pass a required `query` string only (object bag `{ query }`). Do **not** pass `path`, `file_path`, `line`, `character`, `text`, or `action` — this is not a position tool and has no path scope filter. Code-only. Resolves `{ symbols, message }`. Each symbol guarantees `{ name, kind, path, line, character }` — **string** `kind`, workspace-relative **`path` (required on every hit)**, **1-based** start line and character; optional `end_*`. Flattened — no raw LSP JSON.

Empty `symbols` + `message` still **resolves**. Missing / empty `query`, bad types, or engine-not-ready / server failure → `ToolError` (short retry hint when not ready; no fixed seconds); cancel → `cancelled`. No `ok: false` on this tool. At most **200** symbols; truncation notes go in `message`. Example:
const hits = await tools.workspace_symbol({ query: "AgentDeps" });
if (hits.symbols.length === 0) return { ok: false, message: hits.message };
const s = hits.symbols[0];
const def = await tools.go_to_definition({
  file_path: s.path,
  line: s.line,
  character: s.character,
});
return { hit: s, locations: def.locations, message: def.message };
