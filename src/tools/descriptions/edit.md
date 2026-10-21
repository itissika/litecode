Replace one exact string in one file inside `run_code`. Pass `path`, `old_string`, and `new_string` (object bag). Resolves an `EditResult`: `ok` (boolean), `path` (string), and `message` (string). There is no `replace_all`, no `expected_hash`, no `edits` array on this tool, and no `blocks` / `applied` / `failed` / `details` / `file.hash` report.

Matching is exact and must hit exactly one place in the file. Empty `new_string` deletes `old_string`. CRLF and LF compare equal; a leading UTF-8 BOM is preserved. On success, `ok` is true and `message` is a short confirmation such as `replaced` (never empty, never a structured payload). When the path is missing, permission is denied, the string is missing, it matches more than once, or the write otherwise cannot proceed, this call still **resolves** with `ok: false` and a short `message` that says what to fix (add surrounding lines, re-read, check the path, etc.) — it does **not** reject `ToolError` for those cases, including when this single replacement fully fails, and it does not partially write. Branch on `ok` in the same program; do not look for a structured details tree.

Only malformed input (missing fields, wrong types) rejects `invalid_input`, and cancellation rejects `cancelled`. Multiple independent replacements: use `tools.apply_edits`, or call `edit` again. Example (edit then test in one program):
const edited = await tools.edit({
  path: "src/a.rs",
  old_string: "fn start() {}",
  new_string: "fn main() {}",
});
if (!edited.ok) return { ok: false, path: edited.path, message: edited.message };
// edited.message === "replaced" (short confirm)
const test = await tools.run({ command: "cargo test -q --lib a" });
const done =
  test.exit_code === null
    ? await tools.wait({ id: test.id, sec: 60 })
    : test;
if (done.exit_code !== 0) return { ok: false, edited, done };
return { ok: true, path: edited.path, message: edited.message, exit_code: done.exit_code };
