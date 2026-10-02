```node
node : workspace-shared-substrate
status : pending
summary : 工作区基座：文件 / 终端 / 搜索 / Git 等共享基础设施，人类与 agent 入口分立、实例共享。
x : 34
y : 1188
```

**项目级基础设施属于工作区，不归任何 tool 所有；人类与 agent 等价消费。**

- 文件服务：sandbox / watcher / filter presets 唯一实现——安全边界不能多处校验，一个工作区不能有多个 watcher [@ file="src/workspace/sandbox.rs" label="sandbox.rs"]。
- Terminal：PTY 基座同时承载人类交互终端与 agent 命令执行 [@ file="src/terminal/pty.rs" label="pty.rs"]。
- 搜索：多后端（内容 / 语义 / 会话历史）；agent 按源粒度消费 tool，人类统一入口选源。
- Git：本地能力（status / diff / commit / log），人类面板与 agent tool 共享基座。
- 原则：共享基座、各自塑形——可复用能力拆为基座，两端入口各自特化；入口分立，实例共享。


