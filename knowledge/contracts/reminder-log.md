```node
node : reminder-log
status : pending
summary : 提醒是带独立 seq 的日志事实；正文在写入时冻结，视图只渲染冻结文本，不重算现场状态。
x : 5094
y : 1962
domain : harness-core
```

**准入：** A1 / A3。提醒行是会话日志真源的一部分（每条一个 seq），不是 ephemeral 注入；seam 只按固定顺序追加，AgentView 渲染冻结 `text`。若视图按现场状态重算，compact 后恢复与 HumanView 隐藏纪律都会失真。

依据：[@ file="src/reminder/mod.rs" symbol="fn render_item"]、[@ file="src/reminder/engine.rs" symbol="fn sync"]、[@ file="src/reminder/engine.rs" symbol="fn view_from_spine"]、[@ file="src/reminder/kinds.rs" symbol="impl Reminder › fn text"]、[@ file="src/reminder/engine.rs" symbol="mod tests › fn restore_fires_once_after_compaction"]。
上层：[@ key="context"]。
基线：075308b

相关：[@ key="session-model"]。

**核实结论：一致，无漂移。**（基线 075308b）
- 冻结正文：`Reminder::text()` 为写入时字段；`render_item` 只包装该 text —— [@ file="src/reminder/kinds.rs" symbol="impl Reminder › fn text"]；[@ file="src/reminder/mod.rs" symbol="fn render_item"]。
- seam 只追加：`sync` 固定顺序 sources，不重算已落库行 —— [@ file="src/reminder/engine.rs" symbol="fn sync"]。
- Restore 仅 Tasks/Background（`compacted_head`）；Mentions 注释「compaction 后不恢复」与实现一致（不入 sync Restore；经 `append_mentions_for` 一次性附着）—— [@ file="src/reminder/sources/tasks.rs" symbol="fn tasks"]；[@ file="src/reminder/kinds.rs" lines="25-26"]；测试见 [@ file="src/reminder/engine.rs" symbol="mod tests › fn restore_fires_once_after_compaction"]。DATA.md 种表曾缺 Mentions/KnowledgeStatus/CustomToolSettled → ISSUE-014（已于 2026-10-12 GM 授权补齐并关闭；不改节点主张）。
