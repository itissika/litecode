`run_code` is the high-density orchestration surface: think before calling, batch or parallelize tool work, compute in JavaScript, and return compact results instead of dumping raw output. It exists to reduce round trips and token cost.

The program is an async JavaScript function body. Top-level `await` and `return` are supported, and the returned value is the result.

Call tools as `tools.<suite>.<member>(args)`. Omit `args` for no-argument tools; otherwise pass exactly one object. Use tools for side effects and external IO. `fs` is a read-only probe, `libs` are deterministic pure-JS helpers, and there is no direct network or process access.

Tool failures resolve as `ToolFailure`; protocol and hard execution failures throw `ToolError`. Cancellation, the `run_code` budget, and hard limits end the program and cannot be caught. Non-finite numbers in a result become `null` with a warning.