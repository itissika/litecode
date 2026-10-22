Search file contents. Pass `pattern`, optional `path`, `glob`, `literal`, `ignore_case`, and `limit` (default 20, max 100). Resolves `{ hits, message }` where each hit is `{ path, line, text }`. Prefer `literal: true` when the string is not regex. Empty / hit-the-limit still resolve; invalid regex → `ok: false` `invalid_args`.
Example:
const found = await tools.grep({ pattern: "fn start", path: "src", literal: true });
if (found.hits.length === 0) return { ok: false, message: found.message };
return { hit: found.hits[0] };
