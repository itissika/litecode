`run_code` is the high-density orchestration surface: think before calling, batch or parallelize tool work, compute in JavaScript. It exists to reduce round trips and token cost.

The program is an async JavaScript function body. Top-level `await` and `return` are supported. The returned value is the result; `console.log` is for progress notes.

Call tools as `tools.<suite>.<member>(args)`. Omit `args` for no-argument tools; otherwise pass exactly one object. Use tools for side effects and external IO. `fs` is a read-only probe: `ls` (one level, sorted), `stat` (type, size, mtime), `exists` (boolean), all synchronous and workspace-relative. `libs` are deterministic pure-JS helpers: `stats` (`sum`, `mean`, `median`, `variance`, `stddev`, `quantile`, `histogram`, `min`, `max`), `csv` (`parse`, `format`), `text` (`lines`, `words`, `count`, `normalizeNewlines`, `truncate`, `diffLines`), `data` (`groupBy`, `countBy`, `unique`, `sortBy`). There is no direct network or process access.

Tool failures resolve as `ToolFailure`; protocol and hard execution failures throw `ToolError`. Cancellation, the fixed run cap, and hard limits end the program and cannot be caught. Non-finite numbers in a result become `null` with a warning.
