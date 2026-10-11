```node
node : process-lifetime
status : pending
domain : ide-substrate
summary : 引擎子进程不得活过宿主：spawn 后绑定寿命；Stopped/崩溃后不应残留 worker 或语言服务。
```

**准入：** A5/A2。code_search worker 与语言服务在 spawn 后经 `bind_child_to_parent` 绑定（Windows Job kill-on-close；Unix 靠 stdin EOF）；优雅 stop 走引擎 `stop`/`stop_all`，非优雅死亡靠绑定与启动时 `sweep_orphan_lsp_processes`。Stopped 必须意味着无运行实体；残留即违反引擎寿命契约（见 ISSUE-009）。

依据：[@ file="src/proc_lifetime.rs" symbol="fn bind_child_to_parent"]、[@ file="src/proc_lifetime.rs" symbol="fn sweep_orphan_lsp_processes"]、[@ file="src/engines/mod.rs" symbol="impl WorkspaceEngines › fn stop"]、[@ file="src/engines/code_search_engine.rs" symbol="impl CodeSearchEngine › fn stop"]。
上层：[@ key="engine-lifecycle"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 子进程绑定宿主 —— [@ file="src/proc_lifetime.rs" symbol="fn bind_child_to_parent"]（Windows Job；Unix 靠 stdin EOF，函数体为 no-op）；孤儿清扫 [@ file="src/proc_lifetime.rs" symbol="fn sweep_orphan_lsp_processes"]。
- stop 杀 worker —— [@ file="src/engines/code_search_engine.rs" symbol="impl CodeSearchEngine › fn stop"] → `kill_client` → `kill_process`；`stop_during_warmup_kills_worker` 通过。
- ISSUE-009：原 suspect 实为 Unix 僵尸导致的门禁误报；已修测试收割顺序并关闭。产品 SIGKILL 路径成立。
