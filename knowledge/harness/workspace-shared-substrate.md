```node
node : workspace-shared-substrate
status : enabled
domain : ide-substrate
summary : 工作区基座：文件 / 终端 / 搜索 / Git 等共享基础设施，人类与 agent 入口分立、实例共享。
x : 252
y : 890
w : 398
h : 408
```

**主张：** 项目级基础设施属于工作区，不归任何工具所有；人类与 agent 入口分立、实例共享。

- 基座清单：文件（唯一安全边界与唯一 watcher）、终端（同一 PTY 基座）、搜索（多源引擎）、Git。
- 唯一实现：安全校验只在一处、一个工作区只有一个文件监听者；多个实例就是多份语义。
- 引擎也是基座：LSP 与检索是长生命周期服务，人类界面与 agent 工具消费同一实例。
- 入口分立、实例共享：同一能力为人类与 agent 各自塑形入口，底层同一实现；Git 由人类面板与 agent 终端共同消费。
- 进程与位置：内核独立进程（桌面 sidecar / 远程 SSH）、单端口收敛服务；一个进程一个工作区，跨进程独占锁保证同一工作区唯一持有者。

向下展开：[@ key="workspace-sandbox"]、[@ key="workspace-watcher"]、[@ key="terminal-pty"]。
上层：[@ key="product-brief"]、[@ key="ux-design"]。
依据：[@ file="src/workspace/sandbox.rs"]、[@ file="src/workspace/watcher.rs"]、[@ file="src/session/workspace_lock.rs"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 唯一安全门与唯一 watcher —— [@ file="src/workspace/sandbox.rs" symbol="impl Sandbox › fn resolve"]、[@ file="src/workspace/watcher.rs" symbol="impl WorkspaceWatcher › fn start"]；跨进程独占锁 [@ file="src/session/workspace_lock.rs" symbol="impl WorkspaceLock › fn acquire"]。
- PTY / 引擎基座清单与子节点一致；终端死亡清单通过（`terminal_bypass_death_list`）。
- 产品 bug（不否定唯一门主张）—— 顶层 symlink 经 resolve 跟随后被 copy，见 ISSUE-010（不越界，非 B3）。热切换死亡清单误报见 ISSUE-007。
