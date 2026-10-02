```node
node : context-is-view
status : pending
summary : 上下文即视图：单入口 prepare/commit；有损只在显式 compact；失败即停、日志不变。
x : 510
y : 900
```

**回答「模型这次看到什么」——视图是计算，不是存储。**

- ContextPipeline 单一 prepare / commit 入口：compact → build_view → persist；视图（PreparedView）是临时的，不属于 Session [@ file="src/context_pipeline/mod.rs" label="mod.rs"]。
- token 预算与媒体预算在管线内决策；媒体只裁视图、不动日志真源。
- **有损边界**：对话历史的有损只允许显式 compact（摘要替代早期历史）；禁止静默少送、禁止机械截断。
- **提醒即日志事实**：系统提醒写进日志的独立 seq，视图去重后渲染 [@ id="reminder-as-log-fact" label="提醒即日志事实"]。
- **失败即停**：compact 任一步失败 → abort，无 partial commit，日志不变。


