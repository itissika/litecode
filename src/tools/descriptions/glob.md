Find files by path glob. Pass `pattern`, optional `path` (search root), and `limit` (default 1000, max 5000). Resolves `{ paths, message }`—workspace-relative inside the workspace; absolute when the search root is outside. A pattern without `/` matches file names at any depth; with `/` it matches relative to `path`. Empty and hit-the-limit results still resolve.
Example:
const listed = await tools.glob({ pattern: "**/*.rs", path: "src" });
return { first: listed.paths[0] || null, n: listed.paths.length, message: listed.message };
