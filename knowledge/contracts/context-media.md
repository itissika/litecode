```node
node : context-media
status : pending
summary : 媒体只在瞬时 LLM 视图上按 show-once 降级；持久日志不被 media 适配改写。
x : 5094
y : 954
domain : harness-core
```

**准入：** A1 / A3。媒体适配只动请求副本：工具媒体仅保留最新工具批、用户媒体仅保留当前回合；更早的变为短文本占位。持久 transcript 不是写者。若回写日志，历史真源与缓存前缀都会被污染。

依据：[@ file="src/context_pipeline/media_budget.rs" symbol="fn drop_stale_media"]、[@ file="src/context_pipeline/media_budget.rs" symbol="fn strip_media_for_summary"]。
父节点：[@ key="context-view"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- show-once 只改瞬时视图：`build_view` 对 `llm_items` 副本调用 `drop_stale_media`；模块注释钉「persisted transcript is never mutated」—— [@ file="src/context_pipeline/mod.rs" symbol="impl ContextPipeline › fn build_view"]；[@ file="src/context_pipeline/media_budget.rs" symbol="fn drop_stale_media"]。
- compact 摘要前剥媒体：`strip_media_for_summary` —— [@ file="src/context_pipeline/media_budget.rs" symbol="fn strip_media_for_summary"]。
