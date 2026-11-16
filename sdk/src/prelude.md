`run_code` executes one async JavaScript function body. Top-level `await` and `return` are supported. The returned value is the result; `console.log` writes progress notes, not the result.

Call tools as `tools.<suite>.<member>(args)`. A no-argument tool takes no argument; every other tool takes exactly one object matching its declared input schema. Each call returns a promise that resolves to the tool's declared data or rejects with an ordinary JavaScript `Error`; `try`/`catch` and `Promise.allSettled` see that rejection as usual. The declarations below are the source of truth for tool paths, argument shapes, and result fields.

`fs` is a read-only, synchronous, workspace-relative probe. `libs` are pure-JS helpers. Both are declared below. There is no direct network, process, or module access, and no `setTimeout`/`setInterval`. Side effects and external IO go through the bound tools.

Unknown tool bindings, output-contract violations, cancellation, the fixed 600000 ms run cap, and hard limits stop the cell and cannot be caught. Non-finite numbers in a result become `null` with a warning.
