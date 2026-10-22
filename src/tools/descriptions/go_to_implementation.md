Jump to implementations of the symbol at a file position inside `run_code` (traits / interfaces → concrete impls when the language server supports it). Pass `file_path` plus **1-based** `line` and **1-based** UTF-16 `character` (object bag; end-of-line caret allowed). Code-only; no `action`, no inline `text` locator. Resolves `{ locations, message }` — same flattened location shape as `go_to_definition` (guaranteed start fields; optional `end_*`).

Empty `locations` + `message` still **resolves**. Invalid input and bad paths resolve `ok: false` with `reason` `invalid_args` / `not_found` (they do not reject); engine-not-ready / server failures reject `ToolError` (short retry hint when not ready; no fixed seconds). Cancel → `cancelled`. At most **100** locations; truncation notes go in `message`. Prefer `go_to_definition` for the type itself; use this for implementors. Example:
const impls = await tools.go_to_implementation({
  file_path: "src/trait_foo.rs",
  line: 3,
  character: 10,
});
return { locations: impls.locations, message: impls.message };
