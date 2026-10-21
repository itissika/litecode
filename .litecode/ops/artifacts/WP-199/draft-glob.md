Find files by path glob inside `run_code`. Pass an object bag: `pattern` (required), optional `path`, and `limit`. Resolves `{ paths, message }`. `paths` is a string array of **workspace-relative** file paths only (sorted by depth, then parent, then name) — never absolute. There is no `base` workspace|absolute dual mode, no `page`/`cursor`/`stale_cursor`, no `include_ignored`, and no enum `reason` — “no files”, “hit the limit”, or a short why go in `message` only.

`pattern` is matched relative to `path` exactly as given. With `path` `src`, use `pattern` `**/*.rs`, not `src/**/*.rs`. A pattern that walks outside the search directory (including `..`) rejects `ToolError` `invalid_input`. Omit `path` to search from the workspace root. `limit` has a **default and an upper bound** (one bounded call); exact numbers are locked from the Runtime dig at Tools closeout. When more would match, `paths` stops at the bound and `message` says the call hit the limit. Empty `paths` with an explanatory `message` means nothing matched (including under default ignore rules) — do not look for a `reason` enum, a `base` flag, or `include_ignored`.

**Hard vs soft:** a bad bag (missing `pattern`, wrong types) or an out-of-root pattern rejects `ToolError` `invalid_input`. Cancellation rejects `cancelled`. Empty search and hit-the-limit still **resolve** with `paths` + `message`. Example (list then read in one program):
const listed = await tools.glob({ pattern: "**/*.rs", path: "src" });
if (listed.paths.length === 0) return { ok: false, message: listed.message };
const first = listed.paths[0];
const page = await tools.read({ path: first, start_line: 1 });
return {
  ok: true,
  path: first,
  start_line: page.start_line,
  lines: page.lines,
  glob_message: listed.message,
  read_message: page.message,
};
