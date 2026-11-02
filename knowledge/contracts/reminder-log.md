```node
node : reminder-log
status : pending
summary : 提醒是带独立 seq 的日志事实；正文在写入时冻结，视图只渲染冻结文本，不重算现场状态。
x : 5094
y : 1962
domain : harness-core
```

**准入：** A1 / A3。提醒行是会话日志真源的一部分（每条一个 seq），不是 ephemeral 注入；seam 只按固定顺序追加，AgentView 渲染冻结 `text`。若视图按现场状态重算，compact 后恢复与 HumanView 隐藏纪律都会失真。

依据：[@ file="src/reminder/mod.rs" symbol="fn render_item"]、[@ file="src/reminder/engine.rs" symbol="fn sync"]、[@ file="src/reminder/engine.rs" symbol="fn view_from_spine"]、[@ file="src/reminder/kinds.rs" symbol="impl Reminder › fn text"]、[@ file="src/reminder/engine.rs" symbol="mod tests › fn restore_fires_once_after_compaction"]、[@ file="src/reminder/sources/tasks.rs" symbol="fn tasks"]。
上层：[@ key="context"]。

相关：[@ key="session-model"]。
