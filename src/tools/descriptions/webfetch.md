Fetch one http(s) URL you already know. Prefer `tools.websearch` to discover URLs. Pass `url` and optional `format` (`text`|`markdown`|`html`, default `markdown`). Success → `{ ok, url, status, format, body, message }`. Soft failures (HTTP/DNS/timeout) still resolve `ok: false` without `body`. SSRF / bad URL → `ok: false`; engine not ready rejects `ToolError`.
Example:
const page = await tools.webfetch({ url: "https://example.com/docs", format: "markdown" });
if (!page.ok) return { ok: false, status: page.status, message: page.message };
return { ok: true, chars: page.body.length, status: page.status };
