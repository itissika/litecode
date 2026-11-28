```node
node : process-lifetime
status : pending
summary : 引擎子进程不得活过宿主：spawn 后绑定寿命；Stopped/崩溃后不应残留 worker 或语言服务。
x : 5094
y : 1836
domain : ide-substrate
```

**准入：** A5/A2。code_search worker 与语言服务在 spawn 后经 `bind_child_to_parent` 绑定（Windows Job kill-on-close；Unix 靠 stdin EOF）；优雅 stop 走引擎 `stop`/`stop_all`，非优雅死亡靠绑定与启动时 `sweep_orphan_lsp_processes`。Stopped 必须意味着无运行实体；残留即违反引擎寿命契约。

依据：[bind_child_to_parent](src/proc_lifetime.rs#bind_child_to_parent)、[sweep_orphan_lsp_processes](src/proc_lifetime.rs#sweep_orphan_lsp_processes)、[WorkspaceEngines.stop](src/engines/mod.rs#WorkspaceEngines.stop)、[CodeSearchEngine.stop](src/engines/code_search_engine.rs#CodeSearchEngine.stop)。
上层：[engine-lifecycle](knowledge/contracts/workspace/engine-lifecycle.md)。
