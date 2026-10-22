Run one shell command. Resolves `id`, `exit_code` (int or `null` if still running), `output` (stdout), `stderr`, `shell`. Foreground ~30s then backgrounds with `exit_code: null`—continue with `tools.wait` / `tools.kill`. cwd is always workspace root; chain with `&&` when needed. Non-zero `exit_code` still resolves—branch in-program. Parallel `run` via `Promise.all` is truly concurrent. Under Git Bash, POSIX-looking args like `/c` can be rewritten by MSYS—use `//c` or `MSYS_NO_PATHCONV=1`.
Example:
const test = await tools.run({ command: "cargo test -q --lib a" });
const done = test.exit_code === null ? await tools.wait({ id: test.id, sec: 60 }) : test;
return { exit_code: done.exit_code, output: done.output };
