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

契约展开：[@ key="context-view"]、[@ key="reminder-log"]。
上层：[@ key="agent-design"]。
依据：[@ file="src/context_pipeline/mod.rs"]、[@ file="src/reminder/mod.rs"]、[@ file="src/context_pipeline/mod.rs" symbol="impl ContextPipeline › fn build_view"]、[@ file="src/runtime/exec.rs" symbol="impl AgentDeps for AgentRuntime › fn call_model"]、[@ file="src/context_pipeline/compact.rs" symbol="fn compact_input"]、[@ file="src/context_pipeline/media_budget.rs" symbol="fn drop_stale_media"]、[@ file="src/agent/core.rs" symbol="fn run"]、[@ file="src/reminder/mod.rs" symbol="fn render_item"]。
