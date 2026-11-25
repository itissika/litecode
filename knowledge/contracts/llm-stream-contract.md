```node
node : llm-stream-contract
status : pending
summary : 流契约闸：delta 前必须有 output_item.added；唯一转发 choke；中断 seal incomplete。
x : 5094
y : 1512
domain : model-config
```

**准入：** A1。所有 codec 的权威流事件必须经 `forward_stream_event`；缺 early `added` 时闸门合成。用户中止停止读 SSE 并用同一 `item_id` seal incomplete，不得用 raw delta 另拼一套方言。闸门坏了会让工具名迟到、投影开行失败或留下悬空调用。

依据：[forward_stream_event](src/llm/codec/stream_contract.rs#forward_stream_event)、[StreamItemAccumulator](src/llm/codec/stream_contract.rs#StreamItemAccumulator)、[mark_items_incomplete](src/llm/codec/stream_contract.rs#mark_items_incomplete)、[scope_compaction](src/llm/reconnect.rs#scope_compaction)。
上层：[llm-provider-surface](knowledge/contracts/llm-provider-surface.md)。
相关：[stream-projection](knowledge/contracts/stream-projection.md)。
