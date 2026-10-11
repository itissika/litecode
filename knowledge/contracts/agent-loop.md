```node
node : agent-loop
status : pending
summary : L0 循环冻结：取消 > 步数 > 模型停止；先 persist 再执行工具；能力只经 AgentDeps 注入。
x : 5094
y : 612
domain : harness-core
```

**准入：** A1+A2。循环只编排「取消/步数 → 视图 → 模型 → 工具或停止」；终止优先级固定，新能力不得改控制流。模型输出先 `persist_new` 再 `execute_tools`；取消是封口不是抹掉。坏了会让 turn 生命周期、工具管线与会话真源同时失序。

依据：[@ file="src/agent/core.rs" symbol="fn run"]、[@ file="src/agent/deps.rs" symbol="trait AgentDeps"]、[@ file="src/agent/outcome.rs" symbol="enum TurnOutcome"]。
上层：[@ key="turn-lifecycle"]。
相关：[@ key="context-compact"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- L0 终止优先级取消 > 步数 > 模型停止 —— [@ file="src/agent/core.rs" symbol="fn run"]；[@ file="src/agent/outcome.rs" symbol="enum TurnOutcome"]。
- 先 persist 再执行工具 —— `persist_or_stop` 在 `execute_tools` 之前；不完整 FunctionCall 不执行 —— [@ file="src/agent/core.rs" symbol="fn run"]。
- 能力只经 AgentDeps —— [@ file="src/agent/deps.rs" symbol="trait AgentDeps"]；Runtime 接入 [@ file="src/runtime/exec.rs" symbol="impl AgentDeps for AgentRuntime › fn compact_if_needed"]。
- auto compact 失败终止 turn —— `compact_if_needed` 的 `Err` 映射为 `TurnOutcome::Error`，不继续模型调用（Andy 口径核实）。
