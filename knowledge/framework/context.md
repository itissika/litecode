```node
node : context
status : enabled
summary : context：请求视图是计算不是第二存储；历史摘要只走 compact，媒体适配不回写日志。
x : 288
y : 350
w : 452
h : 499
domain : harness-core
```

- 视图不是第二存储：每步从日志派生的工作集计算请求副本，不持久化另一份对话历史。
- 历史摘要只走 compact 契约（自动预算触发或手动）；媒体预算与能力适配只裁请求副本，不改历史真源。
- 准备视图与提交增量各有单入口；失败即停，是否已落库以提交边界为准，不把提交后失败说成回滚。
- 提醒是日志事实（独立 seq），不做 ephemeral 注入。
- 窗口稀缺：注入必须值得它的 token。

契约展开：[context-view](knowledge/contracts/context/context-view.md)、[reminder-log](knowledge/contracts/session/reminder-log.md)。
上层：[agent-design](knowledge/design/agent-design.md)。
依据：[mod.rs](src/context_pipeline/mod.rs)、[mod.rs](src/reminder/mod.rs)、[ContextPipeline.build_view](src/context_pipeline/mod.rs#ContextPipeline.build_view)、[AgentRuntime.AgentDeps.call_model](src/runtime/exec.rs#AgentRuntime.AgentDeps.call_model)、[compact_input](src/context_pipeline/compact.rs#compact_input)、[drop_stale_media](src/context_pipeline/media_budget.rs#drop_stale_media)、[run](src/agent/core.rs#run)、[render_item](src/reminder/mod.rs#render_item)。
