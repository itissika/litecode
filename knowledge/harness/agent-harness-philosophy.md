```node
node : agent-harness-philosophy
status : enabled
summary : agent harness 顶层：内核极薄、L0 冻结、二支柱扩展、日志为真源、治理收敛。
x : 34
y : 890
w : 470
h : 537
```

**主张：** 内核极薄——agent 的能力不写在循环里，由上层装配并注入。

- 控制流冻结：循环只编排「取消/步数 → 视图 → 模型 → 工具或停止」；终止优先级固定（取消 > 步数 > 模型停止），新能力不改控制流。
- 二支柱：Context 回答「模型看到什么」、Tool 回答「模型能做什么」；各有单点入口，管线不得旁路。
- 日志为真源：会话是一份业务日志；视图、投影、回退都从日志派生；提醒是日志事实（独立 seq），不做隐形注入。
- 工具同构：内置 / Custom / MCP 对循环无差异——同合约、同出口、同门闸；能不加工具就不加，工程兜底在管线。
- 治理收敛：权限单管线求值（只收紧、不放宽），配置经唯一写入门。
- 团队与基座：子 agent 是完整会话、不可嵌套；能力落工作区基座共享；内核独立进程，位置只改变连接方式。

框架展开：[@ key="session"]、[@ key="context"]、[@ key="tools"]、[@ key="config"]。
上层：[@ key="product-brief"]、[@ key="agent-design"]。
依据：[@ file="src/agent/core.rs"]、[@ file="src/agent/deps.rs"]、[@ file="src/tool/pipeline.rs"]。
