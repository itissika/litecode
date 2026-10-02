```node
node : context
status : enabled
summary : context：请求视图是计算不是第二存储；历史摘要只走 compact，媒体适配不回写日志。
x : 288
y : 350
w : 416
h : 337
```

- 视图不是第二存储：每步从日志派生的工作集计算请求副本，不持久化另一份对话历史。
- 历史摘要只走 compact 契约（自动预算触发或手动）；媒体预算与能力适配只裁请求副本，不改历史真源。
- 准备视图与提交增量各有单入口；失败即停，是否已落库以提交边界为准，不把提交后失败说成回滚。
- 提醒是日志事实（独立 seq），不做 ephemeral 注入。
- 窗口稀缺：注入必须值得它的 token。

契约展开：[@ id="context-view" label="上下文视图"]。
上层：[@ id="agent-design" label="agent 设计"]。
