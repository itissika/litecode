```node
node : agent-harness-philosophy
status : pending
summary : agent harness 顶层：内核极薄、L0 冻结、二支柱扩展、日志为真源、治理收敛。
x : 34
y : 900
w : 470
h : 537
```

**内核极薄：agent 的能力不写在 loop 里，而由 L1 装配并注入。**

- **控制流冻结** —— loop 只编排 compact → model → tools | stop；新能力不改 loop [@ id="kernel-l0-frozen" label="L0 控制流冻结"]。
- **二支柱** —— Context 回答「模型看到什么」、Tool 回答「模型能做什么」；单点入口，管线不得旁路 [@ id="two-pillars" label="Context 与 Tool 二支柱"]。
- **日志为真源** —— 会话是一份 Item 日志；视图、投影、回退都从日志派生 [@ id="session-log-truth" label="日志为真源"] [@ id="context-is-view" label="上下文即视图"]。
- **工具同构** —— 内置 / Custom / MCP 对 loop 无差异 [@ id="tool-aci" label="Tool 统一抽象"]。
- **治理收敛** —— 权限单管线求值 [@ id="permission-asymmetric" label="权限模型"]；配置经唯一写入门 [@ id="config-single-gate" label="配置唯一写入口"]。
- **团队模型** —— 子 agent 是完整会话，不可嵌套 [@ id="subagent-session-parity" label="子 agent 即完整会话"]。
- **基座与位置** —— 项目能力落工作区基座中共享 [@ id="workspace-shared-substrate" label="工作区基座"]；内核独立进程、位置可切换 [@ id="process-separation" label="进程分离"]，引擎生命周期独立 [@ id="engine-independent-of-tool" label="引擎独立于 Tool"]。

**来源：** loop [@ file="src/agent/core.rs" label="core.rs"]、依赖注入 [@ file="src/agent/deps.rs" label="deps.rs"]、提醒 [@ file="src/reminder/mod.rs" label="reminder/mod.rs"]。
