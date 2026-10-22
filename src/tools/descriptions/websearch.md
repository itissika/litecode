Search the public web inside `run_code` for current titles, URLs, and snippets when the answer is outside the workspace. Use this for discovery; follow a specific URL with `tools.webfetch`. Pass an object bag: required `query` (string), optional `top_k` (integer, default 5, max 10). Code-only — there is no Direct `websearch`. Requires a configured search engine (Exa) that is warmed.

Resolves structured data (never a content-only string):
- success: `{ ok: true, query, results, message }` — `results` is at most `top_k` hits `{title, url, snippet}` (snippet ≈300 chars), best first; `message` is a short count such as `5 results.` (or `No results.`).
- soft failure: `{ ok: false, query?, results: [], message }` — **still resolves**; does **not** reject `ToolError`

Soft failure (branch on `ok`; no try/catch required for these): provider HTTP/business errors, timeouts, unparseable provider responses. `message` is a short human reason.

Empty/invalid `query` or `top_k` resolves `ok: false` (`invalid_args`). Hard failure — rejects `ToolError`: engine not warmed / Exa not configured — short Exa/Settings-oriented retry hint, **no hardcoded seconds**. Malformed args / protocol / budget failures are program-level errors, not soft `ok:false`. This tool has no URL SSRF gate (outbound is to the search provider only).

Example (discover then fetch; soft-fail safe):
const hit = await tools.websearch({ query: "example.com docs webfetch", top_k: 3 });
if (!hit.ok) return { ok: false, message: hit.message };
const page = await tools.webfetch({ url: hit.results[0]?.url ?? "https://example.com/docs", format: "markdown" });
if (!page.ok) return { ok: false, status: page.status, message: page.message };
return { ok: true, snippet: page.body.slice(0, 200), hits: hit.results.length };
