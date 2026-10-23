Jump to definition at a file position. Pass `path` plus 1-based `line` and UTF-16 `character` (end-of-line caret allowed). Resolves `{ locations, message }`—each `{ path, line, character }` (optional `end_*`). Empty still resolves. Cap 100. Engine-not-ready rejects `ToolError`.
Example:
const def = await tools.go_to_definition({ path: "src/lib.rs", line: 10, character: 5 });
return { locations: def.locations, message: def.message };
