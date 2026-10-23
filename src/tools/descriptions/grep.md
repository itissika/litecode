Search file contents. Pass `pattern`, optional `path`, `glob`, `literal`, `ignore_case`, and `limit` (default 20, max 100). Resolves `{ hits, message }`; each hit is `{ path, line, text }`. Empty and hit-the-limit results still resolve; invalid regex resolves `ok: false` `invalid_args`.
Example:
const found = await tools.grep({ pattern: "fn start", path: "src", literal: true });
return { first: found.hits[0] || null, n: found.hits.length, message: found.message };
