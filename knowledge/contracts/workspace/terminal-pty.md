```node
node : terminal-pty
status : pending
summary : 终端只许一套 PTY 基座：TerminalHub + PtySession；人类与 agent 入口分立、实例共享。
x : 5094
y : 2070
domain : ide-substrate
```

**准入：** A1/A7。PTY 实现只存在于 `src/terminal`；死亡清单禁止 desktop/web 再拉 `node-pty` 或在别处出现 ConPTY/winpty。`TerminalHub` 聚合会话，`PtySession` Drop/kill 必须回收子进程与 reader，避免线程与进程泄漏。坏了会出现第二套终端语义或关闭路径挂死。

依据：[PtySession.spawn_interactive](src/terminal/pty.rs#PtySession.spawn_interactive)、[PtySession.Drop](src/terminal/pty.rs#PtySession.Drop)、[install_hub](src/terminal/mod.rs#install_hub)、[death_list_src_pty_symbols_only_in_terminal_module](tests/terminal_bypass_death_list.rs#death_list_src_pty_symbols_only_in_terminal_module)。
上层：[workspace-shared-substrate](knowledge/harness/workspace-shared-substrate.md)。
