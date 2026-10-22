List diagnostics for one source file. Pass `{ file_path }`. Resolves `{ items, message }`—each item `{ severity, path, line, character, message }` (string severity, 1-based). Clean file → empty `items`. Cap 100; truncation noted in `message`. Engine-not-ready rejects `ToolError`.
Example:
const diags = await tools.diagnostics({ file_path: "src/a.rs" });
return { n: diags.items.length, first: diags.items[0] ?? null, message: diags.message };
