```node
node : tool-result-delivery
status : pending
summary : 结果按 call_id 配对、按调用顺序回放；缺失与编码失败有兜底文本，持久化丢弃孤儿。
x : 3492
y : 396
w : 416
h : 537
```

**主张：** 每个调用都以 call_id 配对拿到一条结果，并按模型给出的调用顺序交付；模型看不到悬空调用。

- 配对顺序：结果先入 `call_id → ToolCallResult` 映射，再按原始调用顺序回放——不是完成先后（[@ file="src/tool/executor.rs" symbol="fn outputs_from_tool_results"]）。
- 补位两层：step 级对取消/不完整调用整组补中断输出（[@ file="src/agent/core.rs" symbol="fn run" lines="122-135"]、[@ file="src/agent/core.rs" symbol="fn interrupted_outputs" lines="202-229"]）；编码级把缺失结果降级为错误文本（[@ file="src/tool/executor.rs" symbol="fn outputs_from_tool_results" lines="463-479"]）。
- 媒体兜底：媒体编码失败在此降级为错误文本，fail closed（[@ file="src/tool/executor.rs" symbol="fn function_call_output_item" lines="243-265"]）。
- 落地：FunctionCall 已由模型输出在 transcript，管线只追加 FunctionCallOutput，由 agent 循环持久化（[@ file="src/agent/core.rs" symbol="fn run"]）。
- 持久化防线：提交时丢弃无对应 FunctionCall 的输出（[@ file="src/session/data/sqlite/session.rs" symbol="impl Session › fn commit_turn_delta_with_orphan_cleanup"]）；会话恢复的 `pad_unanswered_calls` 只补临时 LLM 视图、不写磁盘（[@ file="src/session/data/sqlite/session.rs" symbol="impl Session › fn pad_unanswered_calls" lines="2543-2550"]）。

父节点：[@ key="tool-call-contract"]。
