```node
node : session-log-envelope
status : pending
domain : harness-core
summary : 日志行信封是 SessionEvent：seq 必有，surface_op/state 驱动折叠；禁止第二套对话信封。
```

**准入：** A1 / A7。业务日志行身份与折叠语义长在 `SessionEvent`（seq / surface_op / source_seqs / state）上，Surface 由 fold 算出，不是第二份存储。G1 死亡清单用文档词汇守门——文档与代码用词漂移会让身份约定失守（见 ISSUE-003）。

依据：[@ file="src/session/event.rs" symbol="struct SessionEvent"]、[@ file="src/session/surface.rs" symbol="fn fold_surface"]、[@ file="src/session/surface.rs" symbol="enum SurfaceOp"]、[@ file="src/types/death_list_gate.rs" symbol="fn session_seq_g1_envelope_vocab_matches_mental_model"]。
父节点：[@ key="session-model"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b；Batch3 已消 ISSUE-003 文档噪声）
- 信封是 `SessionEvent`（seq / type / surface_op / source_seqs / state）—— [@ file="src/session/event.rs" symbol="struct SessionEvent"]。
- Surface 由 fold 派生 —— [@ file="src/session/surface.rs" symbol="fn fold_surface"]、[@ file="src/session/surface.rs" symbol="enum SurfaceOp"]。
- G1 词汇门 —— [@ file="src/types/death_list_gate.rs" symbol="fn session_seq_g1_envelope_vocab_matches_mental_model"]；文档侧已改 [@ file="src/session/DATA.md"] 对齐 `SessionEvent`（见 ISSUE-003 closed）。
