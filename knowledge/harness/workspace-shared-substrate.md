```node
node : workspace-shared-substrate
status : enabled
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

上层：[@ key="product-brief"]、[@ key="ux-design"]。
依据：[@ file="src/workspace/sandbox.rs"]、[@ file="src/workspace/watcher.rs"]、[@ file="src/session/workspace_lock.rs"]。
