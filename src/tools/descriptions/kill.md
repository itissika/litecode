Stop a shell command started with `tools.run` in this session (this or an earlier `run_code` program). Pass `id` (the `Command.id` from `run` or `wait`). Kills the process group (Windows: `taskkill /T`). Resolves a `Command`: `id`, `exit_code`, `output`, `stderr`, `shell` — same shape as `run` / `wait`.

Only after the process is confirmed dead is `ok` left true (or filled true): `output` includes a `[process was killed]` note and `exit_code` is the reaped code or `null` when none is available. If the reap wait expires and the process is still alive, resolves `ok: false` with `reason: "still_alive"`, `exit_code: null`, and a message — never a false success. This consumes the job: do not expect a later natural-exit reminder for the same id.

An `id` stays valid across later `run_code` calls in the same session while the job is registered (still running, or finished but not yet collected); a job that already exited inside `run`'s foreground window may be gone. An unknown `id`, or an id from another session, resolves `ok: false` with `reason: "not_found"`. Cancelling before the signal rejects `cancelled`. This does not stop the human terminal. Example:
const job = await tools.run({command:"npm test"});
if (job.exit_code === null) {
  const stopped = await tools.kill({id:job.id});
  return {exit_code:stopped.exit_code, output:stopped.output};
}
return {exit_code:job.exit_code, output:job.output};
