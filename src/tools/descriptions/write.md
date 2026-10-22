Create or overwrite one file. Resolves `file` (`path`, `hash`, `bytes`, …) and `created`. Optional `expected_hash` (sha256 of current bytes)—mismatch → `ok: false` `stale`; missing file with `expected_hash` → `not_found` (does not create). `create_only` on an existing file → `conflict`. Prefer `tools.edit` / `tools.apply_edits` for surgical changes.
Example:
const w = await tools.write({ path: "src/b.rs", content: "pub fn ok() -> bool { true }\n" });
return { created: w.created, path: w.file.path };
