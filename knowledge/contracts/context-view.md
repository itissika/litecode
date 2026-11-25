```node
node : context-view
status : enabled
domain : harness-core
summary : 请求视图：从权威工作集派生一次性 PreparedView；适配、补齐与媒体裁剪不回写历史。
x : 748
y : 556
w : 336
h : 434
```

**主张：** 持久历史回答发生了什么，模型视图回答这一次模型能看到什么。

- 视图是每步计算：从日志工作集派生一次性视图，补齐、适配与媒体裁剪只作用于请求副本，不回写日志。
- 保真承诺：对话历史不做静默截短；媒体超出预算按明确占位降级，最新用户消息受保护。
- 唯一有损出口是 compact：摘要替换已持久的历史前缀（半开区间），旧行仍在日志；自动或手动触发，只摘要已持久前缀，未提交尾部不被吞掉。
- 失败以提交边界为准：提交前任何失败不产生检查点、日志不变；COMMIT 之后的重载失败不回滚已落库的检查点。读回失败是 unverified，不是成功；session 每步取 surface 失败必须报错停，不能用旧 fold 继续。
- 回退使旧未提交尾部作废：后续提交不得把已丢弃的内容重新接回。

上层：[context](knowledge/framework/context.md)。相关：[session-model](knowledge/contracts/session-model.md)。
契约展开：[context-budget](knowledge/contracts/context-budget.md)、[context-compact](knowledge/contracts/context-compact.md)、[context-keep-recent](knowledge/contracts/context-keep-recent.md)、[context-media](knowledge/contracts/context-media.md)。
依据：[PreparedView](src/context_pipeline/view.rs#PreparedView)、[ContextPipeline.build_view](src/context_pipeline/mod.rs#ContextPipeline.build_view)、[ContextPipeline.commit_step](src/context_pipeline/mod.rs#ContextPipeline.commit_step)、[commit_plan](src/context_pipeline/compact.rs#commit_plan)、[rebuild_committed_surface](src/context_pipeline/compact.rs#rebuild_committed_surface)、[session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept](src/types/death_list_gate.rs#session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept)、[readback_failure_action](src/context_pipeline/compact.rs#readback_failure_action)、[ContextPipeline.sync_turn_working](src/context_pipeline/mod.rs#ContextPipeline.sync_turn_working)、[AgentRuntime.compact_if_needed](src/runtime/exec.rs#AgentRuntime.compact_if_needed)、[ContextPipeline.commit_step_with_turn](src/context_pipeline/mod.rs#ContextPipeline.commit_step_with_turn)。
