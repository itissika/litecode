Wait on a `tools.run` job in this session. Pass `id` and required `sec` (clamped to 300). Resolves the same `Command` shape as `run`. Natural exit sets `exit_code`; a kill sets `killed: true` and a `[process was killed]` note. A job returns as soon as its process exits: a descendant that still holds the pipe only keeps `output` growing in the log, so `output` may be a snapshot and names the full log file. Budget elapsed with job still running → `exit_code: null` (job kept). Unknown id → `ok: false` `not_found`.
Example:
const job = await tools.run({ command: "npm test" });
if (job.exit_code !== null) return { exit_code: job.exit_code, output: job.output };
const done = await tools.wait({ id: job.id, sec: 60 });
return { exit_code: done.exit_code, output: done.output };
