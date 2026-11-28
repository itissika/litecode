```node
node : tool-result-delivery
status : enabled
summary : 结果按 call_id 配对、按调用顺序回放；缺失与编码失败有兜底，持久化丢弃孤儿、启动封口尾部未回答调用。
x : 2394
y : 378
w : 416
h : 537
domain : tools-permission
```

**目标：** 每个调用都拿到一条结果、按模型给出的顺序回去；模型永远看不到悬空调用。

**必须做到：**

- 结果先按 call_id 配对，再按原始调用顺序回放，不按完成先后。
- 取消或没跑完的调用整组补中断输出，别留悬空。
- 单个结果编码失败就降级成错误文本，不静默丢。
- 落库前丢掉没有对应调用的孤儿结果，顺手把尾部没回答的调用封口。
- 恢复时补的「中断」先只进临时视图不写盘；只有日志尾部真的收不了尾，才追加落盘封口，且文案一致。

**绝对不做：**

- 按完成先后发结果。
- 让模型看到没有结果的调用。
- 让孤儿结果进库。

依据：[outputs_from_tool_results](src/tool/executor.rs#outputs_from_tool_results)、[interrupted_outputs](src/agent/core.rs#interrupted_outputs)、[function_call_output_item](src/tool/executor.rs#function_call_output_item)、[Session.commit_turn_delta_with_orphan_cleanup](src/session/data/sqlite/session.rs#Session.commit_turn_delta_with_orphan_cleanup)、[Session.pad_unanswered_calls](src/session/data/sqlite/session.rs#Session.pad_unanswered_calls)、[Session.seal_tail_unanswered_calls](src/session/data/sqlite/session.rs#Session.seal_tail_unanswered_calls)、[seal_tail_unanswered_calls](src/session/data/writer.rs#seal_tail_unanswered_calls)。

上层：[tool-call-contract](knowledge/contracts/tools/tool-call-contract.md)。
相关：[tool-call-lifecycle](knowledge/contracts/tools/tool-call-lifecycle.md)、[tool-identity](knowledge/contracts/tools/tool-identity.md)。
