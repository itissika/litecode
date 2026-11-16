`run_code` executes one async JavaScript function body with top-level `await` and `return`. Always `return` the data you need to inspect or report; do not discard needed tool results.

Call tools as `tools.<suite>.<member>(args)`. A no-argument tool takes no argument; every other tool takes one object matching its declared input schema. Each call returns a promise that resolves to its declared data or rejects with an ordinary JavaScript `Error`; `try`/`catch` and `Promise.allSettled` handle rejections, and `Promise.all` runs independent reads/searches in parallel. For strings containing backslashes or quotes (such as regex edits), use `String.raw` or template literals. The declarations below are the source of truth for tool paths, argument shapes, and result fields.

`fs` is a synchronous, read-only, workspace-relative probe (`ls`/`stat` throw `Error` on missing or out-of-workspace paths). `libs` are pure-JS helpers. Both are declared below. No direct network, process, module access, or timers (`setTimeout`/`setInterval`) exist; side effects and external IO go through the bound tools.

Unknown tool bindings, output-contract violations, cancellation, the fixed 600000 ms run cap, and hard limits stop the cell and cannot be caught. Non-finite numbers in a result become `null` with a warning.
