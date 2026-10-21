Run one shell command inside `run_code`. The call resolves structured data with `kind` `exited`, `background`, or `promoted`. `stdout` and `stderr` are `{head, tail, bytes, truncated}`. Non-zero `exit_code` still resolves — branch on `exit_code` in the same program. cwd is always the workspace root (not a persistent session); chain with `&&` in one `command` when you need `cd` or env for that call only.

`kind: exited` means the process had already finished when the call decided. It includes `exit_code` (integer or null). A fast command is `exited` even when you pass `run_in_background: true`, and that result has no `job_id`. Small output omits `output_file` and `stderr_file`. When either is set, page it with `tools.read`.

`run_in_background` defaults to false. `true` ignores `timeout_sec`. If the process is still running, the call detaches first and returns `kind: background` with `job_id`, both log paths, and a snapshot of each stream. The process keeps running.

`timeout_sec` is an integer from 1 to 120 (default 30) and only limits the foreground wait. When it elapses and the process is still running, the call returns `kind: promoted` and leaves the process running. It does not kill the process. If the process has already exited at that check, the kind is `exited`.

Commands that may run longer than a few seconds should pass `run_in_background`. Do not rely on promote to beat the `run_code` budget: if that budget cancels the call before it returns, the process is killed and no job survives. Cancellation before a `job_id` is returned also kills the process and rejects `cancelled`. After `background` or `promoted`, stopping the turn does not kill the job. A later natural exit arrives as `reminder/bash_exit`.

Read or stop that job in the same program with the `job_id` from `background` or `promoted`. `tools.command_output` takes `job_id` and optional `wait_sec` (integer 0–120, default 0). `0` returns the current stdout and stderr windows plus `output_file` and `stderr_file` and does not wait. A positive `wait_sec` waits until the process exits or the time runs out. `kind: running` means it is still going; the job is not killed and is not promoted again. `kind: exited` includes `exit_code` and consumes that exit, so the same exit does not also arrive as `reminder/bash_exit`. Cancelling the wait rejects `cancelled` and does not kill the job.

`tools.command_kill` takes `job_id`, kills the process tree, and returns `kind: killed` with `exit_code`, `output_file`, and `stderr_file`. That consumes the exit, so it does not arrive as a user-stopped reminder. An unknown `job_id`, or a job from another session, rejects `not_found` from either tool.

Example (edit then test in one program):
const edited = await tools.edit({path:"src/a.rs", edits:[{old_string:"fn start()", new_string:"fn main()"}]});
if (edited.failed > 0) return {ok:false, edited};
const test = await tools.run_command({command:"cargo test -q --lib a"});
if (test.kind !== "exited" || test.exit_code !== 0) return {ok:false, test};
return {ok:true, hash:edited.file.hash, exit_code:test.exit_code};

Example (write then check):
await tools.write({path:"src/b.rs", content:"pub fn ok() -> bool { true }"});
const check = await tools.run_command({command:"cargo check -q", timeout_sec:60});
return {kind:check.kind, exit_code:check.exit_code ?? null};

Example (long job):
const job = await tools.run_command({command:"npm test", run_in_background:true});
if (job.kind === "exited") return {exit_code:job.exit_code};
const done = await tools.command_output({job_id:job.job_id, wait_sec:60});
if (done.kind === "exited") return {exit_code:done.exit_code, output_file:done.output_file};
await tools.command_kill({job_id:job.job_id});
return {stopped:job.job_id};
