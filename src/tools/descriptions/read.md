Read one workspace file. Pass `path`, optional `start_line` (1-based, default 1), and `limit` (default 2000, max 5000). Resolves `{ path, start_line, lines, message }`—`lines` is a string array; continue with a larger `start_line`, not a cursor. Past-EOF and hit-the-limit still resolve with `lines` + `message`. Missing path → `ok: false` `not_found`; non-text rejects `ToolError` `unsupported`.
Example:
const page = await tools.read({ path: "src/a.rs", start_line: 1 });
if (page.lines.length === 0) return { ok: false, message: page.message };
return { lines: page.lines.length, message: page.message };
