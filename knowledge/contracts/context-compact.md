```node
node : context-compact
status : pending
summary : 压缩只改模型看到的历史，不改库里的事实：库里追加一行，旧行原样留着，读的时候才折叠成摘要 + 保留段。
x : 5364
y : 612
domain : harness-core
```

**准入：** A5 / A7。压缩要解决的是「模型看不下」，不是「库里存不下」：库里只追加，不改写任何旧行；模型可见的历史在读取时折叠——压缩点之前换成一段摘要，之后原样保留。落地只有两种动作，都是追加：摘要替换（新增一行 `compacted`）和机械裁剪（超长工具结果、过期媒体各追加一条替换）。除此之外没有第三条改历史的路径。落库后必须从 fold 重载工作集，禁止在内存里拼 summary+kept；提交前失败还原快照。

0.8 触发边界先做一次机械裁剪决策（只有单独解除压力才落库，否则不写），再无裁剪地冻结一个 plan：同一模型/指令/工具/前缀/cut/`kept_from_seq`/输出上限。摘要是独立分支，按同一请求重试两次；分支自己经 `SessionMutation::Compact` 提交并按 `kept_from_seq` 重锚，后续行原样保留。提交不再做 0.8 gate、不再重判裁剪；只有 surface 真变（checkpoint/cut 消失）或 plan 没有 cut（NoCut）才允许硬线同步重选。正常 turn 结束不取消分支，用户取消才取消。硬发送线为 `min(selected_context_window, prompt_limit - margin)`，standard 在 100% 产品窗口停，max 由物理 prompt 上限封顶。

依据：[@ file="src/context_pipeline/compact.rs" symbol="impl CompactPolicy › fn compact_transcript"]、[@ file="src/session/data/command.rs" symbol="enum SessionMutation"]、[@ file="src/types/death_list_gate.rs" symbol="fn session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept"]。
父节点：[@ key="context-view"]。
基线：666540c
相关：[@ key="session-model"]。

**核实结论：按实现订正。**（基线 666540c）
- 两条追加动作：`SessionMutation::Compact`（摘要，新增一行 `compacted`，fold 时替换 `[from, to)`，被换行仍在库里）与 `SessionMutation::ReplaceSurface`（机械裁剪，追加替换行、原行不删）—— [@ file="src/session/data/command.rs" symbol="enum SessionMutation"]；[@ file="src/context_pipeline/compact.rs" symbol="impl CompactPolicy › fn compact_transcript"]。
- 落库后 fold 重载：`working_set_blocking` —— 同 `compact_transcript`；死亡清单 [@ file="src/types/death_list_gate.rs" symbol="fn session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept"]（子串门 → ISSUE-013 gate）。
- Andy 口径：auto（`compact_if_needed` / `CompactionTrigger::Auto`）失败 → `agent::run` `TurnOutcome::Error` 终止 turn；手动（`compact_now` / `start_manual_compact`）失败发 Failed、`drop(lease)` 后可再调 —— [@ file="src/agent/core.rs" symbol="fn run"]；[@ file="src/client_protocol/controller/session_lifecycle.rs" symbol="impl SessionController › fn start_manual_compact"]。
