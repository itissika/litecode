Search the public web inside `run_code` for current titles, URLs, and snippets when the answer is outside the workspace. Use this for discovery; follow a specific URL with `tools.webfetch`. Pass an object bag: required `query` (string). Code-only — there is no Direct `websearch`. Requires a configured search engine (Exa) that is warmed.

Resolves structured data (never a content-only string):
- success: `{ ok: true, query, text, message }` — `text` is the provider’s flattened body; `message` is the short fixed word `searched`. Do **not** expect a `results` array unless Runtime’s final dig stably exposes one (then teaching adds one sentence only).
- soft failure: `{ ok: false, query?, message }` — **still resolves**; does **not** reject `ToolError`

Soft failure (branch on `ok`; no try/catch required for these): provider HTTP/business errors, timeouts, unparseable provider responses. `message` is a short human reason.

Hard failure — rejects `ToolError`: empty/invalid `query`, engine not warmed / Exa not configured — short Exa/Settings-oriented retry hint, **no hardcoded seconds**. Malformed args / protocol / budget failures are program-level errors, not soft `ok:false`. This tool has no URL SSRF gate (outbound is to the search provider only).

Example (discover then fetch; soft-fail safe):
const hit = await tools.websearch({ query: "example.com docs webfetch" });
if (!hit.ok) return { ok: false, message: hit.message };
// hit.message === "searched"
const page = await tools.webfetch({ url: "https://example.com/docs", format: "markdown" });
if (!page.ok) return { ok: false, status: page.status, message: page.message };
return { ok: true, snippet: page.body.slice(0, 200), search_chars: hit.text.length };
