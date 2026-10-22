Replace one exact string in one file. Must hit exactly one place; empty `new_string` deletes `old_string`. CRLF/LF compare equal; leading UTF-8 BOM is preserved. Soft failures (miss, multi-match, missing path, permission) resolve `ok: false` with `reason`/`message`—branch on `ok`. For several independent replacements use `tools.apply_edits`.
Example:
const edited = await tools.edit({
  path: "src/a.rs",
  old_string: "fn start() {}",
  new_string: "fn main() {}",
});
if (!edited.ok) return { ok: false, message: edited.message };
return { ok: true, path: edited.path };
