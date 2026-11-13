`run_code` is the high-density orchestration surface: think before calling, batch or parallelize tool work, compute in JavaScript, and return compact results instead of dumping raw output. It exists to reduce round trips and token cost.

The program is an async JavaScript function body. Top-level `await` and `return` are supported, and the returned value is the result.

Call tools as `tools.<suite>.<member>(args)`. Omit `args` for no-argument tools; otherwise pass exactly one object. Use tools for side effects and external IO. `fs` is a read-only probe: `ls` (one level, sorted), `stat` (type, size, mtime), `exists` (boolean), all synchronous and workspace-relative. `libs` are deterministic pure-JS helpers: `stats` (`sum`, `mean`, `median`, `variance`, `stddev`, `quantile`, `histogram`, `min`, `max`), `csv` (`parse`, `format`), `text` (`lines`, `words`, `count`, `normalizeNewlines`, `truncate`, `diffLines`), `data` (`groupBy`, `countBy`, `unique`, `sortBy`). There is no direct network or process access.

Tool failures resolve as `ToolFailure`; protocol and hard execution failures throw `ToolError`. Cancellation, the `run_code` budget, and hard limits end the program and cannot be caught. Non-finite numbers in a result become `null` with a warning.

Terminal work uses the Codex-style session pair. Prefer `tools.process.exec_command` for every shell command: it waits at most `yield_time_ms` and returns either an exit code or a live `session_id`. Drive that session with `tools.process.write_stdin`: empty `chars` polls output, ordinary bytes go to stdin, and `"\u0003"` sends SIGINT to the process group so cleanup runs like a real Ctrl-C. The legacy `process.run` / `process.wait` / `process.kill` trio remains for compatibility, but it is not the interactive path.
