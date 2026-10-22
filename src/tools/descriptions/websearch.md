Search the public web (Exa). Pass `query` and optional `top_k` (default 5, max 10). Success → `{ ok, query, results: [{title,url,snippet}], message }`. Soft provider failures resolve `ok: false`; engine not configured rejects `ToolError`. Follow a hit with `tools.webfetch`.
Example:
const hit = await tools.websearch({ query: "example.com docs", top_k: 3 });
if (!hit.ok) return { ok: false, message: hit.message };
return { ok: true, hits: hit.results.length, first: hit.results[0] };
