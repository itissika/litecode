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
- 失败以提交边界为准：提交前任何失败不产生检查点、日志不变；COMMIT 之后的重载失败不回滚已落库的检查点。
- 回退使旧未提交尾部作废：后续提交不得把已丢弃的内容重新接回。

上层：[@ key="context"]。相关：[@ key="session-model"]。
契约展开：[@ key="context-budget"]、[@ key="context-compact"]、[@ key="context-keep-recent"]、[@ key="context-media"]。
依据：[@ file="src/context_pipeline/view.rs" symbol="struct PreparedView"]、[@ file="src/context_pipeline/mod.rs" symbol="impl ContextPipeline › fn build_view"]、[@ file="src/context_pipeline/mod.rs" symbol="impl ContextPipeline › fn commit_step"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 一次性视图：`build_view` 克隆 working → pad/project/drop_stale_media → 写入 `PreparedView`，不回写日志 —— [@ file="src/context_pipeline/mod.rs" symbol="impl ContextPipeline › fn build_view"]。
- 唯一有损出口 compact：`SessionMutation::Compact` 后 `working_set_blocking` 重载 —— [@ file="src/context_pipeline/compact.rs" symbol="impl CompactPolicy › fn compact_transcript"]；死亡清单 [@ file="src/types/death_list_gate.rs" symbol="fn session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept"]（gate 子串脆弱见 ISSUE-013）。
- 失败边界：compact 提交前失败还原 `snapshot`；agent 环 `compact_if_needed` Err 不继续 turn —— [@ file="src/context_pipeline/compact.rs" symbol="impl CompactPolicy › fn compact_transcript"]；[@ file="src/agent/core.rs" symbol="fn run"]。
- 提交：`commit_step_with_turn`；Idempotent 清空 prepared（丢弃路径）；成功路径由 `take_prepared_view` 先消费 —— [@ file="src/context_pipeline/mod.rs" symbol="impl ContextPipeline › fn commit_step_with_turn"]。
