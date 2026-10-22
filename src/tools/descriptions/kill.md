Stop a `tools.run` job in this session. Pass `id`. Kills the process group (Windows: `taskkill /T`). Only after the process is confirmed dead is `ok` true—`output` includes `[process was killed]`. If still alive after the reap wait → `ok: false` `still_alive`. Unknown id → `not_found`. Consumes the job.
Example:
const job = await tools.run({ command: "npm test" });
if (job.exit_code === null) {
  const stopped = await tools.kill({ id: job.id });
  return { exit_code: stopped.exit_code, output: stopped.output };
}
return { exit_code: job.exit_code };
