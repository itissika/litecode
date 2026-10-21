Read one workspace file inside `run_code`. Pass an object bag: `path` (required), optional `start_line`, and `limit`. Resolves `{ path, start_line, lines, message }`. `path` echoes the file (**workspace-relative**). `start_line` is the **1-based** line where this window began (same name on the way in and out — not `offset`). `lines` is a string array of that window’s line contents (plain strings). If one line is too long, shorten it **inside that `lines[]` string** — there is no `line_truncated` / `truncated` field and no `{line_number, …}` objects. Put “end of file”, “hit the limit”, missing/unsupported file, or a short why into `message` only. There is no `end_line`, no `file.hash` / `expected_hash` (write’s `expected_hash` is out of this package), no `page` / `cursor`, and no `reason` enum.

Omit `start_line` (or pass `1`) to start at the first line. `limit` has a **default and an upper bound** (one bounded call); exact numbers are locked from the Runtime dig at Tools closeout. To continue, call again with a larger `start_line` (typically previous `start_line + lines.length`); do **not** pass a cursor. When the window reaches the end of the file, `lines` may be shorter than `limit` and `message` can note that. When the call hits the line bound with more remaining, say so in `message` (not a separate truncated flag).

**Hard vs soft:** a bad bag (missing `path`, wrong types, non-integer `start_line` / `limit`) rejects `ToolError` `invalid_input`. Cancellation rejects `cancelled`. A **missing path** or **unsupported/non-text** file still **resolves**: empty `lines` and an explanatory `message` (soft fail — branch on `lines.length` / `message`, not `ToolError`). Past-EOF empty windows and hit-the-limit likewise resolve. Example (paged read then edit in one program):
const page1 = await tools.read({ path: "src/a.rs", start_line: 1 });
if (page1.lines.length === 0) return { ok: false, message: page1.message };
const next = page1.start_line + page1.lines.length;
const page2 = await tools.read({ path: "src/a.rs", start_line: next });
const needle = "fn start() {}";
const blob = page1.lines.join("\n");
if (!blob.includes(needle)) return { ok: false, message: "needle not in first page", page1 };
const edited = await tools.edit({
  path: "src/a.rs",
  old_string: needle,
  new_string: "fn main() {}",
});
return { ok: edited.ok, edited, page1_message: page1.message, page2_len: page2.lines.length };
