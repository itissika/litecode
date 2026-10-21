Run one shell command inside `run_code`. Resolves a `Command`: `id`, `exit_code`, and `output`. `id` is the handle for later `tools.wait` / `tools.kill` in the same program. `exit_code` is an integer when the process has already exited, or `null` when it is still running. `output` is always present (may be empty): combined process text plus any truncation note, on-disk spill path, or status line the runtime appends. There is no `kind`, no separate stdout/stderr windows, and no wait-budget or background-mode parameters on this call.

cwd is always the workspace root (not a persistent session). Chain with `&&` in one `command` when you need `cd` or env for that call only. A non-zero `exit_code` still resolves — branch on `exit_code` in the same program; do not treat non-zero as `ToolError`.

If the process is still running when this call returns, `exit_code` is `null` and `output` holds whatever is available so far (or a short status note). Keep waiting with `tools.wait({id, sec})`. Stop it with `tools.kill({id})`. Example (edit then test in one program):
const edited = await tools.edit({path:"src/a.rs", old_string:"fn start()", new_string:"fn main()"});
if (!edited.ok) return {ok:false, edited};
const test = await tools.run({command:"cargo test -q --lib a"});
if (test.exit_code === null) {
  const done = await tools.wait({id:test.id, sec:60});
  if (done.exit_code !== 0) return {ok:false, done};
  return {ok:true, path:edited.path, exit_code:done.exit_code, output:done.output};
}
if (test.exit_code !== 0) return {ok:false, test};
return {ok:true, path:edited.path, exit_code:test.exit_code, output:test.output};

Example (write then check):
await tools.write({path:"src/b.rs", content:"pub fn ok() -> bool { true }"});
const check = await tools.run({command:"cargo check -q"});
const done = check.exit_code === null
  ? await tools.wait({id:check.id, sec:120})
  : check;
return {exit_code:done.exit_code, output:done.output};
