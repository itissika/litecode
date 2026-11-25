```node
node : reminder-log
status : pending
summary : 提醒是带独立 seq 的日志事实；正文在写入时冻结，视图只渲染冻结文本，不重算现场状态。
x : 5094
y : 1962
domain : harness-core
```

**准入：** A1 / A3。提醒行是会话日志真源的一部分（每条一个 seq），不是 ephemeral 注入；seam 只按固定顺序追加，AgentView 渲染冻结 `text`。若视图按现场状态重算，compact 后恢复与 HumanView 隐藏纪律都会失真。

依据：[render_item](src/reminder/mod.rs#render_item)、[sync](src/reminder/engine.rs#sync)、[view_from_spine](src/reminder/engine.rs#view_from_spine)、[Reminder.text](src/reminder/kinds.rs#Reminder.text)、[tests.restore_fires_once_after_compaction](src/reminder/engine.rs#tests.restore_fires_once_after_compaction)、[tasks](src/reminder/sources/tasks.rs#tasks)。
上层：[context](knowledge/framework/context.md)。

相关：[session-model](knowledge/contracts/session-model.md)。
