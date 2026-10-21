Run one foreground shell command inside `run_code`. The call resolves structured data: `status`, `exit_code`, bounded `stdout`/`stderr` (`head`, `tail`, `bytes`, `truncated`), and optional `output_file` when output was truncated or large. Non-zero `exit_code` still resolves — treat it as a domain result and branch on `exit_code` in the same program. Cancellation rejects `ToolError` with `code` `cancelled`. Timeout rejects `timeout` (details may include `timeout_sec` / `output_file`); the process group is killed and the command is not backgrounded. There is no `bash_id`. cwd is always the workspace root (not a persistent session); chain with `&&` in one `command` when you need `cd` or env for that call only. Pass `timeout_sec` as an integer 1–120 (default 30).

This is the command path for builds, tests, git, and scripts next to `tools.edit` / `tools.write`. When `output_file` is set, page it with `tools.read` instead of re-running the command.

Example (edit then test in one program):
const edited = await tools.edit({path:"src/a.rs", edits:[{old_string:"fn start()", new_string:"fn main()"}]});
if (edited.failed > 0) return {ok:false, edited};
const test = await tools.run_command({command:"cargo test -q --lib a"});
if (test.exit_code !== 0) return {ok:false, exit_code:test.exit_code, stderr:test.stderr, output_file:test.output_file ?? null};
return {ok:true, hash:edited.file.hash, exit_code:test.exit_code};

Example (write then check):
await tools.write({path:"src/b.rs", content:"pub fn ok() -> bool { true }"});
const check = await tools.run_command({command:"cargo check -q", timeout_sec:60});
return {exit_code:check.exit_code, status:check.status};
