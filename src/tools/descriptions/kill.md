Stop a shell command started with `tools.run` in this `run_code` program. Pass `id` (the `Command.id` from `run` or `wait`). Kills the process group. Resolves a `Command`: `id`, `exit_code`, `output` — same shape as `run` / `wait`.

`output` always includes a short note that the process was killed (plus any truncated logs or spill path). `exit_code` is an integer or `null` when no code is available. This consumes the job: do not expect a later natural-exit reminder for the same id.

An unknown `id`, or an id from another session / program, rejects `not_found`. Cancelling before the signal rejects `cancelled`. This does not stop the human terminal. Example:
const job = await tools.run({command:"npm test"});
if (job.exit_code === null) {
  const stopped = await tools.kill({id:job.id});
  return {exit_code:stopped.exit_code, output:stopped.output};
}
return {exit_code:job.exit_code, output:job.output};
