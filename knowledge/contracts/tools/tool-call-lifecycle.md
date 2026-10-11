```node
node : tool-call-lifecycle
status : pending
domain : tools-permission
summary : 分批执行加每调用一条结果；取消是信号，超时是止损，都不覆盖已完成结果。
x : 3258
y : 396
w : 470
h : 531
```

**主张：** 调用的生命周期以「分批执行 + 每调用一条结果」为单元；取消与超时是协作机制，不是对底层工作的保证。

- 开始：一个 step 里只要任一 FunctionCall 状态不完整（incomplete / in_progress）或已取消，整组调用都不执行，并以 Cancelled 收尾（[@ file="src/agent/core.rs" symbol="fn function_call_must_not_execute"]）；补位输出见 [@ key="tool-result-delivery"]。
- 分批：按并发安全与资源键切批后再执行；切批规则见 [@ key="tool-scheduling"]。
- 取消点：执行入口与批次边界检查令牌（[@ file="src/tool/executor.rs" symbol="fn run_tool"]）；并发批内检测到取消时逐个 join 剩余任务而非 abort 丢弃（[@ file="src/tool/pipeline.rs" symbol="impl ToolPipeline › fn join_tool_handle"]）。真实工作止于工具自身，`is_cancellable` 只是工具侧的能力声明（[@ file="src/tool/trait_.rs" symbol="trait Tool › fn is_cancellable"]）。
- 超时：`tool.timeout()` 是公共包装，超时以 Error 文本返回；丢掉的是 future 包装，不保证底层线程已停（[@ file="src/tool/executor.rs" symbol="fn run_tool"]）。
- 事实与观测：工具 future 一旦返回即结果，晚到取消不覆盖已完成结果（[@ file="src/tool/executor.rs" symbol="fn run_tool"]）；进入执行前发 TurnPhase::ExecutingTools（[@ file="src/runtime/exec.rs" symbol="impl AgentDeps for AgentRuntime › fn execute_tools"]），授权通过后相位恢复（[@ file="src/runtime/phase.rs" lines="47-53"]），这条管线不维护独立逐调用状态机。

父节点：[@ key="tool-call-contract"]。
相关：[@ key="tool-scheduling"]、[@ key="tool-result-delivery"]、[@ key="tool-execution-context"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b）
- 不完整/取消整组不执行 —— [@ file="src/agent/core.rs" symbol="fn function_call_must_not_execute"]、[@ file="src/agent/core.rs" symbol="fn interrupted_outputs"]。
- 入口取消与超时包装 —— [@ file="src/tool/executor.rs" symbol="fn run_tool"]；批内 join —— [@ file="src/tool/pipeline.rs" symbol="impl ToolPipeline › fn join_tool_handle"]。
