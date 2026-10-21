List references to the symbol at a file position inside `run_code`. Pass `file_path` plus **1-based** `line` and **1-based** UTF-16 `character` (object bag; end-of-line caret allowed). Code-only; no `action`, no inline `text` locator. Resolves `{ locations, message }` — same flattened location shape as `go_to_definition` (guaranteed start `path` / `line` / `character`; optional `end_*`). No raw LSP JSON.

Empty `locations` + `message` still **resolves**. Invalid input, bad paths, and engine-not-ready / server failures reject `ToolError` (short retry hint when not ready; no fixed seconds). Cancel → `cancelled`. No `ok: false` on this tool. At most **100** locations; truncation notes go in `message`. Example:
const refs = await tools.find_references({
  file_path: "src/lib.rs",
  line: 40,
  character: 12,
});
if (refs.locations.length === 0) return { ok: false, message: refs.message };
const first = refs.locations[0];
const slice = await tools.read({ path: first.path, start_line: first.line, limit: 20 });
return { refs: refs.locations, preview: slice.lines, message: refs.message };
