Search file contents inside `run_code`. Pass an object bag: `pattern` (required), optional `path`, `glob`, `literal`, `ignore_case`, and `limit`. Resolves `{ hits, message }`. Each hit is `{ path, line, text }`: workspace-relative `path`, **1-based** `line`, and `text` (that one line). If a single matching line is too long, shorten it **inside `text` itself** — there is no `line_truncated` / `truncated` field. There is no `matches` tree, no `start_line`/`end_line`/`line_text` on a hit, no `base`/`searched`/`page`/`cursor`/`stale_cursor`, no `include_ignored`, and no enum `reason` — put “no hits”, “hit the limit”, or a short why into `message` only.

`pattern` is a regular expression unless `literal` is true (plain text). Prefer `literal: true` when the string is not meant as regex. `path` narrows the search root (workspace-relative); omit it to search from the workspace root. `glob` further filters file names (for example `**/*.rs`). `ignore_case` makes matching case-insensitive when true. `limit` defaults to **20** and is at most **100** (one bounded call). When more would match, `hits` stops at the bound and `message` says the call hit the limit — raise `limit` to see more. Empty `hits` with an explanatory `message` means nothing matched under the current filters (including default ignore rules) — do not look for a `reason` enum or an `include_ignored` knob.

**Result contract:** a bad bag (missing `pattern`, wrong types, empty/invalid `glob` when provided) or an invalid regex resolves `ok: false` with `reason: "invalid_args"` and the cause in `message`. Cancellation rejects `cancelled`. Empty search and hit-the-limit still **resolve** with `hits` + `message`. Example (find then edit in one program):
const found = await tools.grep({
  pattern: "fn start",
  path: "src",
  literal: true,
});
if (found.hits.length === 0) return { ok: false, message: found.message };
const hit = found.hits[0];
const edited = await tools.edit({
  path: hit.path,
  old_string: hit.text,
  new_string: hit.text.replace("fn start", "fn main"),
});
return { ok: edited.ok, hit, edited, message: found.message };
