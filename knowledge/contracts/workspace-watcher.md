```node
node : workspace-watcher
status : pending
summary : 一个工作区只有一个 notify 所有者；排除在源端硬切，禁止第二套 OS watcher。
x : 5094
y : 2286
domain : ide-substrate
```

**准入：** A1/A7。`WorkspaceWatcher` 是工作区唯一的 `notify` 所有者（DESIGN §2.9）：多实例即多份变更总线语义。排除规则在源端硬切，消费者只做二次过滤；排除清单重载不是工作区根热切换。坏了会导致引擎/UI 各听各的、或误把排除重载当成根切换。

依据：[@ file="src/workspace/watcher.rs" symbol="impl WorkspaceWatcher › fn start"]、[@ file="src/workspace/watcher.rs" symbol="fn spawn_watcher"]、[@ file="tests/workspace_hotswitch_death_list.rs" symbol="fn death_list_forbidden_hot_switch_symbols"]。
上层：[@ key="workspace-shared-substrate"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 工作区唯一 notify 所有者 —— [@ file="src/workspace/watcher.rs" symbol="impl WorkspaceWatcher › fn start"]、[@ file="src/workspace/watcher.rs" symbol="fn spawn_watcher"]；排除规则在源端硬切后调用 `reload_workspace_excludes_from_disk`（不是根热切换）。
- 死亡清单 `death_list_forbidden_hot_switch_symbols` 因子串误报失败 —— ISSUE-007（建议收窄 FORBIDDEN，禁止静默删测）。
