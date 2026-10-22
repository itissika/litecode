Fetch hover docs / type info at a file position. Pass `file_path` plus 1-based `line` and UTF-16 `character`. Resolves `{ markdown, message }` (flattened text in `markdown`, not raw LSP JSON). Unavailable hover still resolves with explanatory `message`.
Example:
const info = await tools.hover({ file_path: "src/main.rs", line: 10, character: 5 });
return { markdown: info.markdown, message: info.message };
