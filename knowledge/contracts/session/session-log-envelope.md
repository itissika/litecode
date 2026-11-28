```node
node : session-log-envelope
status : pending
summary : 日志行信封是 SessionEvent：seq 必有，surface_op/state 驱动折叠；禁止第二套对话信封。
x : 5094
y : 2736
domain : harness-core
```

**准入：** A1 / A7。业务日志行身份与折叠语义长在 `SessionEvent`（seq / surface_op / source_seqs / state）上，Surface 由 fold 算出，不是第二份存储。G1 死亡清单用文档词汇守门——文档与代码用词漂移会让身份约定失守。

依据：[SessionEvent](src/session/event.rs#SessionEvent)、[fold_surface](src/session/surface.rs#fold_surface)、[SurfaceOp](src/session/surface.rs#SurfaceOp)、[session_seq_g1_envelope_vocab_matches_mental_model](src/types/death_list_gate.rs#session_seq_g1_envelope_vocab_matches_mental_model)、[DATA.md](src/session/DATA.md)。
父节点：[session-model](knowledge/contracts/session/session-model.md)。
