```node
node : terminal-pty
status : pending
domain : ide-substrate
summary : 终端只许一套 PTY 基座：TerminalHub + PtySession；人类与 agent 入口分立、实例共享。
```

**准入：** A1/A7。PTY 实现只存在于 `src/terminal`；死亡清单禁止 desktop/web 再拉 `node-pty` 或在别处出现 ConPTY/winpty。`TerminalHub` 聚合会话，`PtySession` Drop/kill 必须回收子进程与 reader，避免线程与进程泄漏。坏了会出现第二套终端语义或关闭路径挂死。

依据：[@ file="src/terminal/pty.rs" symbol="impl PtySession › fn spawn_interactive"]、[@ file="src/terminal/pty.rs" symbol="impl Drop for PtySession"]、[@ file="src/terminal/mod.rs" symbol="fn install_hub"]、[@ file="tests/terminal_bypass_death_list.rs" symbol="fn death_list_src_pty_symbols_only_in_terminal_module"]。
上层：[@ key="workspace-shared-substrate"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 唯一 PTY 基座 —— [@ file="src/terminal/pty.rs" symbol="impl PtySession › fn spawn_interactive"]、[@ file="src/terminal/mod.rs" symbol="fn install_hub"]；Drop 回收见 [@ file="src/terminal/pty.rs" symbol="impl Drop for PtySession"]。
- 死亡清单三测全过 —— `death_list_no_node_pty_*` / `death_list_src_pty_symbols_only_in_terminal_module`。
