```node
node : tool-call-lifecycle
status : enabled
summary : 分批执行加每调用一条结果；取消是信号，超时是止损，都不覆盖已完成结果。
x : 2124
y : 918
w : 470
h : 531
domain : tools-permission
```

**目标：** 调用的生命周期以「分批执行 + 每调用一条结果」为单元；取消和超时是信号与止损，不是对底层工作的保证。

**必须做到：**

- 一个 step 里任一调用状态不完整，或整轮已取消，整组都不执行：补中断输出，按取消收尾。
- 分批按并发安全和资源键切；切法见 [tool-scheduling](knowledge/contracts/tools/tool-scheduling.md)。
- 取消是合作式的：框架只发信号并等落地，能不能真停看工具自己响应令牌，不按工具类型分流；批内发现取消就逐个 join 剩余任务，不 abort 丢弃。
- 超时是止损：到点先发取消让工具自己收，等它落地；已提交的结果赢过超时，没提交才按超时返回错误文本。
- 工具 future 返回就是结果，晚到的取消不覆盖已完成结果。
- 进执行前发 ExecutingTools 相位；权限回答后回到执行相位（拒绝也算），中止才转取消；不另维护逐调用状态机。

**绝对不做：**

- 把取消或超时当成「底层工作已停」的保证。
- 用晚到的取消覆盖已完成结果。
- 为调用再造一份逐调用状态机。

依据：[function_call_must_not_execute](src/agent/core.rs#function_call_must_not_execute)、[interrupted_outputs](src/agent/core.rs#interrupted_outputs)、[run_tool](src/tool/executor.rs#run_tool)、[effective_tool_timeout](src/tool/executor.rs#effective_tool_timeout)、[ToolPipeline.join_tool_handle](src/tool/pipeline.rs#ToolPipeline.join_tool_handle)、[execute_tools](src/runtime/exec.rs#execute_tools)、[PhasePermissionSink.leave_phase](src/runtime/phase.rs#PhasePermissionSink.leave_phase)、[committed_result_wins_over_tool_timeout](src/tool/executor.rs#committed_result_wins_over_tool_timeout)、[blocking_tool_is_interrupted_by_pipeline_timeout](src/tool/executor.rs#blocking_tool_is_interrupted_by_pipeline_timeout)。

父节点：[tool-call-contract](knowledge/contracts/tools/tool-call-contract.md)。
相关：[tool-scheduling](knowledge/contracts/tools/tool-scheduling.md)、[tool-result-delivery](knowledge/contracts/tools/tool-result-delivery.md)、[tool-execution-context](knowledge/contracts/tools/tool-execution-context.md)。
