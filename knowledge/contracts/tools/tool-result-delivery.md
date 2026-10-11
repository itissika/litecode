```node
node : tool-result-delivery
status : pending
summary : 结果按 call_id 配对、按调用顺序回放；缺失与编码失败有兜底，持久化丢弃孤儿、启动封口尾部未回答调用。
x : 2394
y : 378
w : 416
h : 537
domain : tools-permission
```

**主张：** 每个调用都以 call_id 配对拿到一条结果，并按模型给出的调用顺序交付；模型看不到悬空调用。

- 配对顺序：结果先入 `call_id → ToolCallResult` 映射，再按原始调用顺序回放——不是完成先后（[@ file="src/tool/executor.rs" symbol="fn outputs_from_tool_results"]）。
- 补位两层：step 级对取消/不完整调用整组补中断输出（[@ file="src/agent/core.rs" symbol="fn interrupted_outputs"]）；编码级把缺失结果降级为错误文本（[@ file="src/tool/executor.rs" symbol="fn outputs_from_tool_results"]）。
- 媒体兜底：媒体编码失败在此降级为错误文本，fail closed（[@ file="src/tool/executor.rs" symbol="fn function_call_output_item"]）。
- 落地：FunctionCall 已由模型输出在 transcript，管线只追加 FunctionCallOutput，由 agent 循环持久化（持久化路径不在本节点展开）。
- 持久化防线：提交时丢弃无对应 FunctionCall 的输出（[@ file="src/session/data/sqlite/session.rs" symbol="impl Session › fn commit_turn_delta_with_orphan_cleanup"]）；会话恢复分两层——临时视图由 `pad_unanswered_calls` 补齐（不写盘）[@ file="src/session/data/sqlite/session.rs" symbol="impl Session › fn pad_unanswered_calls"]；未回答调用仍是日志尾部时，启动由 `seal_tail_unanswered_calls` 追加落盘封口（同文案、append-only）[@ file="src/session/data/sqlite/session.rs" symbol="impl Session › fn seal_tail_unanswered_calls"]、[@ file="src/session/data/writer.rs" symbol="fn seal_tail_unanswered_calls"]。

父节点：[@ key="tool-call-contract"]。
相关：[@ key="tool-call-lifecycle"]、[@ key="tool-identity"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b）
- call_id 映射再按序回放 —— [@ file="src/tool/executor.rs" symbol="fn outputs_from_tool_results"]。
- 中断补位 —— [@ file="src/agent/core.rs" symbol="fn interrupted_outputs"]。
- 孤儿丢弃 / 尾部封口 —— [@ file="src/session/data/sqlite/session.rs" symbol="impl Session › fn commit_turn_delta_with_orphan_cleanup"]、[@ file="src/session/data/sqlite/session.rs" symbol="impl Session › fn seal_tail_unanswered_calls"]。
