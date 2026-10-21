Apply several independent `{path, old_string, new_string}` replacements in one call inside `run_code`. Pass an object bag with `edits`: a non-empty array. Runs **truly serially** in array order: each successful item is written before the next item runs, so later items on the **same `path` see earlier successful `new_string` values**. Resolves `{ results: EditResult[] }` — `results.length` equals `edits.length`. Each `EditResult` is `ok`, `path`, and `message` (same shape as `tools.edit`).

A failed item is **skipped**; later items still run. Even when every item fails, the call still **resolves** with `results` full of `ok: false` — it does **not** reject `ToolError` / `edit_failed`. There is no `replace_all`, no `expected_hash`, no shared multi-block same-file plan, and no aggregate `applied` / `failed` / `blocks` / `details` — count or filter `results` yourself (`results.some(r => !r.ok)`). Empty `new_string` deletes that item's `old_string`. Exact match, unique-hit, CRLF/LF, and BOM rules match `edit`. Path missing, permission denied, miss, or multiple matches for an item → that item's `ok: false` with the reason in `message` only. On success, that item's `message` is a short confirmation such as `applied` (never empty, never structured).

If a later `old_string` depends on an earlier change that failed (`ok: false`), that later item will typically miss and resolve `ok: false` with a not-found style `message` — fix the earlier failure or split into another call. Only a malformed bag / empty `edits` / bad item types reject `invalid_input`; cancellation rejects `cancelled`. Example:
const batch = await tools.apply_edits({
  edits: [
    { path: "src/a.rs", old_string: "fn start()", new_string: "fn main()" },
    { path: "src/b.rs", old_string: "old_api(", new_string: "new_api(" },
  ],
});
if (batch.results.some((r) => !r.ok)) {
  return { ok: false, results: batch.results };
}
// each ok item: message === "applied" (short confirm)
await tools.write({
  path: "src/c.rs",
  content: "pub fn ok() -> bool { true }\n",
});
const check = await tools.run({ command: "cargo check -q" });
const done =
  check.exit_code === null
    ? await tools.wait({ id: check.id, sec: 120 })
    : check;
return {
  ok: done.exit_code === 0,
  results: batch.results,
  exit_code: done.exit_code,
  output: done.output,
};
