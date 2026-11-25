```node
node : stream-projection
status : pending
summary : 一流一投影：item 开行即得 seq；seal 用调用返回副本；流从不二次 append。
x : 5364
y : 1458
domain : harness-core
```

**准入：** A3。一次 `complete_with_stream_events` 拥有 provider `item_id`→日志 `seq` 的映射；流式增量只 update 已开行，终态用调用返回的 Item seal。若流与返回各自 append，读者会看到错序推理且 reload 无法修复。写失败必须失败整个 call，不得吞掉。

依据：[StreamProjection](src/runtime/stream_projection.rs#StreamProjection)、[StreamProjection.settle](src/runtime/stream_projection.rs#StreamProjection.settle)、[death_list_gate.rs](src/types/death_list_gate.rs)。
上层：[turn-lifecycle](knowledge/contracts/turn-lifecycle.md)。
相关：[llm-stream-contract](knowledge/contracts/llm-stream-contract.md)。
