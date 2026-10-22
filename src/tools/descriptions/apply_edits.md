Apply several independent `{path, old_string, new_string}` replacements in one call. Runs serially in array order; later items on the same path see earlier successful writes. Failed items are skipped; the call still resolves `{ results }` (same length as `edits`)—filter `results` yourself. Exact-match / unique-hit / CRLF / BOM rules match `edit`. Empty `new_string` deletes that item's `old_string`.
Example:
const batch = await tools.apply_edits({
  edits: [
    { path: "src/a.rs", old_string: "fn start()", new_string: "fn main()" },
    { path: "src/b.rs", old_string: "old_api(", new_string: "new_api(" },
  ],
});
if (batch.results.some((r) => !r.ok)) return { ok: false, results: batch.results };
return { ok: true, results: batch.results };
