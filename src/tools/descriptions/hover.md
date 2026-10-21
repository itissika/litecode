Fetch hover docs / type info at a file position inside `run_code`. Pass `file_path` plus **1-based** `line` and **1-based** UTF-16 `character` (object bag; end-of-line caret allowed). Code-only; no `action`, no inline `text` locator. Resolves `{ markdown, message }` — flattened hover text in the **`markdown`** field (not `content`, not raw LSP `contents` JSON). There is no `locations` array on this tool.

Unavailable hover still **resolves** with an explanatory `message` and empty / short `markdown`. Invalid input, bad paths, and engine-not-ready / server failures reject `ToolError` (short retry hint when not ready; no fixed seconds). Cancel → `cancelled`. No `ok: false` on this tool. Example:
const info = await tools.hover({
  file_path: "src/main.rs",
  line: 10,
  character: 5,
});
return { markdown: info.markdown, message: info.message };
