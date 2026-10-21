Jump to implementations of the symbol at a file position inside `run_code` (traits / interfaces → concrete impls when the language server supports it). Pass `file_path` plus **1-based** `line` and **1-based** `character` (object bag). Code-only; no `action`, no inline `text` locator. Resolves `{ locations, message }` — same flattened location shape as `go_to_definition` (guaranteed start fields; optional `end_*`).

Empty `locations` + `message` still **resolves**. Invalid input, bad paths, and engine-not-ready / server failures reject `ToolError` (short retry hint when not ready; no fixed seconds). Cancel → `cancelled`. No `ok: false` on this tool. Results are bounded; truncation notes go in `message`. Prefer `go_to_definition` for the type itself; use this for implementors. Example:
const impls = await tools.go_to_implementation({
  file_path: "src/trait_foo.rs",
  line: 3,
  character: 10,
});
return { locations: impls.locations, message: impls.message };
