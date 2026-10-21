Wait on a shell command started with `tools.run` in this `run_code` program. Pass `id` (the `Command.id` from `run`) and `sec` (required positive number of seconds to block). Resolves a `Command`: `id`, `exit_code`, `output` — same shape as `run`.

`sec` is required. Values above the built-in maximum of **300** are silently clamped to 300 (no error). After clamping, this call blocks until the process exits or the wait budget elapses. If it exits, `exit_code` is set and `output` holds the accumulated text (plus any truncation / spill notes). If the budget elapses and the process is still running, the call still resolves: `exit_code` stays `null` and `output` is the current snapshot — the job is not killed and is not promoted. Call `wait` again or `tools.kill({id})`.

An unknown `id`, or an id from another session / program, rejects `not_found`. Cancelling this wait rejects `cancelled` and does not kill the job. Example:
const job = await tools.run({command:"npm test"});
if (job.exit_code !== null) return {exit_code:job.exit_code, output:job.output};
const done = await tools.wait({id:job.id, sec:60});
return {exit_code:done.exit_code, output:done.output};
