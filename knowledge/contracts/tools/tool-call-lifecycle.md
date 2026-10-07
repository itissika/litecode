```node
node : tool-call-lifecycle
status : pending
summary : 分批执行加每调用一条结果；取消是信号，超时是止损，都不覆盖已完成结果。
x : 3258
y : 396
w : 470
h : 531
```

**主张：** 调用的生命周期以「分批执行 + 每调用一条结果」为单元；取消与超时是协作机制，不是对底层工作的保证。

- 开始：一个 step 里只要任一 FunctionCall 状态不完整（incomplete / in_progress）或已取消，整组调用都不执行，补中断输出并以 Cancelled 收尾（[@ file="src/agent/core.rs" symbol="fn run" lines="122-135"]；判定谓词 [@ file="src/agent/core.rs" symbol="fn function_call_must_not_execute"]）。
- 分批：`partition_tool_calls` 按并发安全与资源键切批，安全批内 tokio::spawn 并行，其余按批串行（[@ file="src/tool/executor.rs" symbol="fn partition_tool_calls"]）。
- 取消点：执行入口、批次边界与授权/写锁后检查令牌（[@ file="src/tool/executor.rs" symbol="fn run_tool" lines="299-301"]、[@ file="src/tool/pipeline.rs" symbol="impl ToolPipeline › fn execute_batch_cancellable" lines="69-90"]、[@ file="src/tool/executor.rs" symbol="fn run_tool" lines="337-339,370-372"]）；并发批内检测到取消时逐个 join 剩余任务而非 abort 丢弃（[@ file="src/tool/pipeline.rs" symbol="impl ToolPipeline › fn join_tool_handle" lines="245-267"]）。真实工作止于工具自身，`is_cancellable` 只是工具侧的能力声明（[@ file="src/tool/trait_.rs" symbol="trait Tool › fn is_cancellable"]）。
- 超时：`tool.timeout()` 是公共包装，超时以 Error 文本返回；丢掉的是 future 包装，不保证底层线程已停（[@ file="src/tool/executor.rs" symbol="fn run_tool" lines="390-396"]）。
- 事实与观测：工具 future 一旦返回即结果，晚到取消不覆盖已完成结果（[@ file="src/tool/executor.rs" symbol="fn run_tool" lines="401-405"]）；进入执行前发 TurnPhase::ExecutingTools（[@ file="src/runtime/exec.rs" symbol="impl AgentDeps for AgentRuntime › fn execute_tools" lines="51-57"]），授权通过后相位恢复（[@ file="src/runtime/phase.rs" lines="47-53"]），这条管线不维护独立逐调用状态机。

父节点：[@ key="tool-call-contract"]。
