Read one workspace file. Pass `path`, optional `start_line` (1-based, default 1), and `limit` (default 2000, max 5000). Resolves `{ path, start_line, lines, message }`—`lines` is a string array; continue with a later `start_line`. Empty files, past-EOF pages, and hit-the-limit pages still resolve; `message` says which case. Missing path resolves `ok: false` `not_found`; non-text rejects `ToolError` `unsupported`.
Example:
const page = await tools.read({ path: "src/a.rs", start_line: 1 });
return { lines: page.lines.length, message: page.message };
