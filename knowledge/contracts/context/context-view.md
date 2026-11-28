```node
node : context-view
status : enabled
summary : 请求视图：库里的历史回答发生了什么，模型视图回答这一眼能看到什么；视图每步算一次，不回写。
x : 748
y : 556
w : 336
h : 434
domain : harness-core
```

**目标：** 库里的历史回答「发生了什么」，模型视图回答「这一眼能看到什么」——视图每步算一次，绝不回写历史。

**必须做到：**

- 每步从权威工作集派生一次性视图；补齐、适配、媒体裁剪只改请求副本。
- 对话历史不静默截短；媒体超出预算按明确占位降级，最新用户消息受保护。
- 唯一有损出口是压缩：摘要替换已持久的前缀（半开区间），旧行仍留在日志；只摘要已持久的部分，没提交的尾部不被吞。
- 失败以提交边界为准：提交前失败不产生检查点、日志不变；提交之后重载失败，不回滚已经落库的检查点。
- 读回失败叫「未验证」，不叫成功；每步取 surface 失败必须报错停，不许拿旧视图继续。
- 回退让旧的未提交尾部作废，之后的提交不能把丢弃的内容接回来。

**绝对不做：**

- 把视图当第二份存储，或把适配结果写回日志。
- 静默截短对话历史。
- 把没提交的尾部压进摘要。
- 读回失败当成功，或用旧视图继续跑。
- 回退之后把已丢弃的内容重新接回。

依据：[PreparedView](src/context_pipeline/view.rs#PreparedView)、[ContextPipeline.build_view](src/context_pipeline/mod.rs#ContextPipeline.build_view)、[ContextPipeline.commit_step](src/context_pipeline/mod.rs#ContextPipeline.commit_step)、[ContextPipeline.commit_step_with_turn](src/context_pipeline/mod.rs#ContextPipeline.commit_step_with_turn)、[ContextPipeline.sync_turn_working](src/context_pipeline/mod.rs#ContextPipeline.sync_turn_working)、[rebuild_committed_surface](src/context_pipeline/compact.rs#rebuild_committed_surface)、[readback_failure_action](src/context_pipeline/compact.rs#readback_failure_action)、[AgentRuntime.compact_if_needed](src/runtime/exec.rs#AgentRuntime.compact_if_needed)、[session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept](src/types/death_list_gate.rs#session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept)。

上层：[context](knowledge/framework/context.md)。
相关：[session-model](knowledge/contracts/session/session-model.md)、[context-budget](knowledge/contracts/context/context-budget.md)、[context-compact](knowledge/contracts/context/context-compact.md)、[context-media](knowledge/contracts/context/context-media.md)。
