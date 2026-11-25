```node
node : tool-call-lifecycle
status : pending
summary : 分批执行加每调用一条结果；取消是信号，超时是止损，都不覆盖已完成结果。
x : 2124
y : 918
w : 470
h : 531
domain : tools-permission
```

**主张：** 调用的生命周期以「分批执行 + 每调用一条结果」为单元；取消与超时是协作机制，不是对底层工作的保证。

- 开始：一个 step 里只要任一 FunctionCall 状态不完整（incomplete / in_progress）或已取消，整组调用都不执行，并以 Cancelled 收尾（[function_call_must_not_execute](src/agent/core.rs#function_call_must_not_execute)）；补位输出见 [tool-result-delivery](knowledge/contracts/tools/tool-result-delivery.md)。
- 分批：按并发安全与资源键切批后再执行；切批规则见 [tool-scheduling](knowledge/contracts/tools/tool-scheduling.md)。
- 取消点：执行入口与批次边界检查令牌（[run_tool](src/tool/executor.rs#run_tool)）；并发批内检测到取消时逐个 join 剩余任务而非 abort 丢弃（[ToolPipeline.join_tool_handle](src/tool/pipeline.rs#ToolPipeline.join_tool_handle)）。真实工作止于工具自身，`is_cancellable` 只是工具侧的能力声明（[Tool.is_cancellable](src/tool/trait_.rs#Tool.is_cancellable)）。
- 超时：`tool.timeout()` 是公共包装，超时以 Error 文本返回；丢掉的是 future 包装，不保证底层线程已停（[run_tool](src/tool/executor.rs#run_tool)）。
- 事实与观测：工具 future 一旦返回即结果，晚到取消不覆盖已完成结果（[run_tool](src/tool/executor.rs#run_tool)）；进入执行前发 TurnPhase::ExecutingTools（[AgentRuntime.AgentDeps.execute_tools](src/runtime/exec.rs#AgentRuntime.AgentDeps.execute_tools)），授权通过后相位恢复（[phase.rs:47-53](src/runtime/phase.rs#L47-L53)），这条管线不维护独立逐调用状态机。

父节点：[tool-call-contract](knowledge/contracts/tool-call-contract.md)。
相关：[tool-scheduling](knowledge/contracts/tools/tool-scheduling.md)、[tool-result-delivery](knowledge/contracts/tools/tool-result-delivery.md)、[tool-execution-context](knowledge/contracts/tools/tool-execution-context.md)。

依据：[interrupted_outputs](src/agent/core.rs#interrupted_outputs)。
