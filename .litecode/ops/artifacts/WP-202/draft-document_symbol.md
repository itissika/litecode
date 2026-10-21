List symbols in one source file inside `run_code`. Pass `{ file_path }` (object bag). Code-only; no position fields, no `action`, no `text`. Resolves `{ symbols, message }`. Each symbol guarantees `{ name, kind, line, character }` — **string** `kind` (for example `Function`), **1-based** start line and character. Optional `end_line` / `end_character` may appear; **`path` is omitted** (use the same `file_path` you passed). Flattened — no raw LSP JSON, no `name_path` / Serena dialects.

Empty `symbols` + `message` still **resolves**. Bad path / engine-not-ready / server failure → `ToolError` (short retry hint when not ready; no fixed seconds); cancel → `cancelled`. No `ok: false` on this tool. Results are bounded; truncation notes go in `message`. Example:
const outline = await tools.document_symbol({ file_path: "src/lib.rs" });
const target = outline.symbols.find((s) => s.name === "start");
if (!target) return { ok: false, message: outline.message };
const def = await tools.go_to_definition({
  file_path: "src/lib.rs",
  line: target.line,
  character: target.character,
});
return { outline: outline.symbols, def: def.locations, message: def.message };
