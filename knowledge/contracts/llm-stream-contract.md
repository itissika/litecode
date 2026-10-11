```node
node : llm-stream-contract
status : pending
domain : model-config
summary : 流契约闸：delta 前必须有 output_item.added；唯一转发 choke；中断 seal incomplete。
```

**准入：** A1。所有 codec 的权威流事件必须经 `forward_stream_event`；缺 early `added` 时闸门合成。用户中止停止读 SSE 并用同一 `item_id` seal incomplete，不得用 raw delta 另拼一套方言。闸门坏了会让工具名迟到、投影开行失败或留下悬空调用。

依据：[@ file="src/llm/codec/stream_contract.rs" symbol="fn forward_stream_event"]、[@ file="src/llm/codec/stream_contract.rs" symbol="struct StreamItemAccumulator"]、[@ file="src/llm/codec/stream_contract.rs" symbol="fn mark_items_incomplete"]。
上层：[@ key="llm-provider-surface"]。
相关：[@ key="stream-projection"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 唯一转发 choke forward_stream_event —— [@ file="src/llm/codec/stream_contract.rs" symbol="fn forward_stream_event"]。
- 缺 early added 时累加器合成 —— [@ file="src/llm/codec/stream_contract.rs" symbol="struct StreamItemAccumulator"]。
- 中断 seal incomplete —— [@ file="src/llm/codec/stream_contract.rs" symbol="fn mark_items_incomplete"]。
- 压缩传输 Failed→Cleared —— [@ file="src/llm/reconnect.rs" symbol="fn scope_compaction"]（同文件单测 `compaction_scope_turns_a_terminal_failure_into_cleared`；cfg(test) 未入符号索引；防 agent/retry 误醒）。
