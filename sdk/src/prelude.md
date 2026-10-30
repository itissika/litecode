"code" is an async JavaScript function body. Top-level await and return are supported. The top-level return value becomes result; the run header and logs are diagnostics.

The TypeScript declarations below are documentation only; do not paste type annotations into code. Call bundled tools only through tools.name(args), with exactly one argument object. A code suite keeps its namespace even with one member: tools.suite.member(args). Other LiteCode tools may remain directly callable outside this SDK.

Every resolved call has a boolean ok. When ok is false, reason and message describe a domain failure, so one bad item need not end a batch. Unknown tools and extra JavaScript arguments throw TypeError; invalid tool arguments resolve ok:false invalid_args, and unknown fields are never silently dropped. Non-finite result numbers (NaN / Infinity) become null with a header warning. For command tools, ok means the call completed; inspect exit_code for the process status. Start a job with run_in_background:true, then use its id with wait/kill while exit_code is null. Cancellation, the run_code budget, and hard limits end the program and cannot be caught.

libs are deterministic pure-JS helpers (no filesystem, network, or process). fs.ls / fs.stat / fs.exists are synchronous, read-only workspace probes; mutate files only through declared tools. The sandbox has no setTimeout / setInterval.

Unhandled Promise rejections do not end the program; a [warn] notes them after settle.