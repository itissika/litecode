```node
node : session-log-envelope
status : pending
summary : 日志行信封是 SessionEvent：seq 必有，surface_op/state 驱动折叠；禁止第二套对话信封。
x : 5094
y : 2736
domain : harness-core
```

**准入：** A1 / A7。业务日志行身份与折叠语义长在 `SessionEvent`（seq / surface_op / source_seqs / state）上，Surface 由 fold 算出，不是第二份存储。G1 死亡清单用文档词汇守门——文档与代码用词漂移会让身份约定失守。

依据：[@ file="src/session/event.rs" symbol="struct SessionEvent"]、[@ file="src/session/surface.rs" symbol="fn fold_surface"]、[@ file="src/session/surface.rs" symbol="enum SurfaceOp"]、[@ file="src/types/death_list_gate.rs" symbol="fn session_seq_g1_envelope_vocab_matches_mental_model"]、[@ file="src/session/DATA.md"]。
父节点：[@ key="session-model"]。
