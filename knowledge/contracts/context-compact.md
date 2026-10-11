```node
node : context-compact
status : pending
domain : harness-core
summary : 唯一有损历史改写只经 SessionMutation::Compact；落库后从 fold 重载工作集，失败还原内存快照且不写检查点。
```

**准入：** A5 / A7。compact 是请求视图契约下唯一改写持久前缀的出口：只摘要已持久前缀，经 `SessionMutation::Compact` 落库后必须 `working_set_blocking` 重载，禁止内存拼 summary+kept；提交前失败还原快照。死亡清单钉住这条路径，第二套落库/拼装即架构违规。

依据：[@ file="src/context_pipeline/compact.rs" symbol="impl CompactPolicy › fn compact_transcript"]、[@ file="src/session/data/command.rs" symbol="enum SessionMutation"]、[@ file="src/types/death_list_gate.rs" symbol="fn session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept"]。
父节点：[@ key="context-view"]。
基线：075308b
相关：[@ key="session-model"]。

**核实结论：一致，无漂移。**（基线 075308b）
- 唯一落库：`SessionMutation::Compact`；失败还原 snapshot 且不写检查点 —— [@ file="src/context_pipeline/compact.rs" symbol="impl CompactPolicy › fn compact_transcript"]；[@ file="src/session/data/command.rs" symbol="enum SessionMutation"]。
- 落库后 fold 重载：`working_set_blocking` —— 同 `compact_transcript`；死亡清单 [@ file="src/types/death_list_gate.rs" symbol="fn session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept"]（子串门 → ISSUE-013 gate）。
- Andy 口径：auto（`compact_if_needed` / `CompactionTrigger::Auto`）失败 → `agent::run` `TurnOutcome::Error` 终止 turn；手动（`compact_now` / `start_manual_compact`）失败发 Failed、`drop(lease)` 后可再调 —— [@ file="src/agent/core.rs" symbol="fn run"]；[@ file="src/client_protocol/controller/session_lifecycle.rs" symbol="impl SessionController › fn start_manual_compact"]。
