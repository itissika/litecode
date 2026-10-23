List references at a file position. Pass `path` plus 1-based `line` and UTF-16 `character`. Resolves `{ locations, message }` (same location shape as `go_to_definition`). Cap 100. Engine-not-ready rejects `ToolError`.
Example:
const refs = await tools.find_references({ path: "src/lib.rs", line: 40, character: 12 });
return { n: refs.locations.length, first: refs.locations[0] ?? null };
