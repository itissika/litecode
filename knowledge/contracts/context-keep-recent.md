```node
node : context-keep-recent
status : pending
domain : harness-core
summary : keep-recent 切割永不拆开 tool 调用/结果对或 assistant 段；整段落在窗口内则跳过 compact。
```

**准入：** A1。compact 的可丢弃前缀由 keep-recent 切割决定：切割必须保持 FunctionCall 与其 Output 同窗、assistant 段完整，且不得切到 leading summary 之前；切不出可丢弃前缀则本次不 compact。切坏会使模型侧工具配对断裂或摘要吞掉未提交语义。

依据：[@ file="src/context_pipeline/keep_recent.rs" symbol="fn find_keep_recent_cut"]、[@ file="src/context_pipeline/keep_recent.rs" symbol="mod tests › fn tool_pair_not_split"]。
父节点：[@ key="context-view"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 切割不拆 tool 对 / assistant 段；`cut==0` → None 跳过 compact —— [@ file="src/context_pipeline/keep_recent.rs" symbol="fn find_keep_recent_cut"]；测试见 [@ file="src/context_pipeline/keep_recent.rs" symbol="mod tests › fn tool_pair_not_split"]。
