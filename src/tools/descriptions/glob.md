Find files by path glob. Pass `pattern`, optional `path` (search root), and `limit` (default 1000, max 5000). Resolves `{ paths, message }`—workspace-relative paths only. A pattern with no `/` matches the file name at any depth; with `/` it matches the path relative to `path`. Empty / hit-the-limit still resolve.
Example:
const listed = await tools.glob({ pattern: "**/*.rs", path: "src" });
if (listed.paths.length === 0) return { ok: false, message: listed.message };
return { first: listed.paths[0], n: listed.paths.length };
