```node
node : agent-loop
status : pending
summary : L0 循环冻结：取消 > 步数 > 模型停止；先 persist 再执行工具；能力只经 AgentDeps 注入。
x : 5094
y : 612
domain : harness-core
```

**准入：** A1+A2。循环只编排「取消/步数 → 视图 → 模型 → 工具或停止」；终止优先级固定，新能力不得改控制流。模型输出先 `persist_new` 再 `execute_tools`；取消是封口不是抹掉。坏了会让 turn 生命周期、工具管线与会话真源同时失序。

依据：[run](src/agent/core.rs#run)、[AgentDeps](src/agent/deps.rs#AgentDeps)、[TurnOutcome](src/agent/outcome.rs#TurnOutcome)、[AgentRuntime.AgentDeps.compact_if_needed](src/runtime/exec.rs#AgentRuntime.AgentDeps.compact_if_needed)。
上层：[turn-lifecycle](knowledge/contracts/turn-lifecycle.md)。
相关：[context-compact](knowledge/contracts/context-compact.md)。
