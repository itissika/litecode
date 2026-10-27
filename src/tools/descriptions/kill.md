Stop a `tools.run` job in this session. Pass `id`. Terminates the whole process tree, including descendants that outlived the shell (Windows: a per-job Job Object, falling back to `taskkill /T`). Only after the process is confirmed dead is `ok` true—`output` includes `[process was killed]`. A job that had already exited is reported with its exit code and `job had already exited`; when only its leftover descendants were stopped, the note says so. If still alive after the reap wait -> `ok: false` `still_alive`, and its later natural exit is still reported by `wait`. Unknown id -> `not_found`. The job record remains observable through `wait`.
Example:
const job = await tools.run({ command: "npm test" });
if (job.exit_code === null) {
  const stopped = await tools.kill({ id: job.id });
  return { exit_code: stopped.exit_code, output: stopped.output };
}
return { exit_code: job.exit_code };
