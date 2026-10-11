```node
node : stream-projection
status : pending
domain : harness-core
summary : 一流一投影：item 开行即得 seq；seal 用调用返回副本；流从不二次 append。
```

**准入：** A3。一次 `complete_with_stream_events` 拥有 provider `item_id`→日志 `seq` 的映射；流式增量只 update 已开行，终态用调用返回的 Item seal。若流与返回各自 append，读者会看到错序推理且 reload 无法修复。写失败必须失败整个 call，不得吞掉。

依据：[@ file="src/runtime/stream_projection.rs" symbol="struct StreamProjection"]、[@ file="src/runtime/stream_projection.rs" symbol="impl StreamProjection › fn settle"]、[@ file="src/types/death_list_gate.rs" symbol="fn stream_projection_uses_only_the_lifecycle_writes"]。
上层：[@ key="turn-lifecycle"]。
相关：[@ key="llm-stream-contract"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 一流一投影、开行即得 seq —— [@ file="src/runtime/stream_projection.rs" symbol="struct StreamProjection"]。
- seal 用调用返回副本、流不二次 append —— [@ file="src/runtime/stream_projection.rs" symbol="impl StreamProjection › fn settle"]；[@ file="src/types/death_list_gate.rs" symbol="fn stream_projection_uses_only_the_lifecycle_writes"]。
- 写失败失败整个 call —— `settle` 传播 `state.failed`，不吞错。
