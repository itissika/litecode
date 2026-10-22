List symbols in one source file. Pass `{ file_path }`. Resolves `{ symbols, message }`—each `{ name, kind, line, character }` (string kind, 1-based; optional `end_*`; no `path`). Cap 200. Engine-not-ready rejects `ToolError`.
Example:
const outline = await tools.document_symbol({ file_path: "src/lib.rs" });
const target = outline.symbols.find((s) => s.name === "start");
return { target, n: outline.symbols.length };
