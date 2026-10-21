Fetch one http(s) URL inside `run_code` when you already know the address (docs page, raw file, API HTML). Prefer `tools.websearch` when you still need to discover URLs. Pass an object bag: required `url` (string); optional `format` as `text` | `markdown` | `html` (default `markdown`). Code-only — there is no Direct `webfetch`.

Resolves structured data (never a content-only string):
- success: `{ ok: true, url, status, format, body, message }` — `message` is the short fixed confirmation `Fetched.`
- soft failure: `{ ok: false, url?, status?, format?, message }` — **still resolves**; does **not** reject `ToolError`. Soft failure **tries to return** the request `url` and `format`; `status` is present **only when** the server answered with an HTTP response. Treat those fields as optional when reading. Soft failures have no `body`.

Soft failure (branch on `ok` in the same program; no try/catch required for these): HTTP 4xx/5xx, DNS failure, request/body/redirect read failure, timeout. `message` is a short human reason. Long bodies are truncated inside `body` at **50000** bytes with a size note (no separate truncated field). If Runtime changes that cap, follow its dig.

Hard failure — rejects `ToolError` (catchable; uncaught fails the whole `run_code`): empty/invalid `url`, non-http(s) scheme, SSRF block for true private/non-public targets (RFC1918, loopback, link-local, unique-local; CGNAT `100.64/10` still blocked), engine not warmed / unavailable — short Settings-oriented retry hint, **no hardcoded seconds**. Malformed args / protocol / budget failures are program-level errors, not soft `ok:false`. Fake-IP / TUN ranges such as `198.18.0.0/15` are **allowed** through the public gate when the hostname resolves there; teaching does not ask you to use literal IPs to bypass TUN.

Example (search then fetch; soft-fail safe):
const found = await tools.websearch({ query: "litecode webfetch SSRF" });
if (!found.ok) return { ok: false, step: "search", message: found.message };
const page = await tools.webfetch({ url: "https://example.com/docs", format: "markdown" });
if (!page.ok) return { ok: false, step: "fetch", status: page.status, message: page.message };
// page.message === "Fetched."
return { ok: true, chars: page.body.length, status: page.status };
