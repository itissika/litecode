# 01: SessionEvent 信封与内存 surface

**What to build:** 内核里可以追加带必有 `seq` 的事件，并用纯函数 `fold_surface` / `derive_messages` 得到模型可见的 `Item[]`。一次 `replace` 后，摘要出现在被阴影区间的位置，而不是日志尾巴。append-origin 转写仍按 seq 升序、不含 replace 副本。编译可以红。

**Blocked by:** None (can start immediately)

**Status:** resolved

- [x] `SessionEvent` / `SurfaceOp` 字段名与法一致：`seq`、`surface_op`、`source_seqs`；`seq` 不是 Option，也不是「没有就用 buffer_index」
- [x] `append` 校验 JSON 与 surface 转移；坏事件进不了日志
- [x] 内存测试：detail 0..4 + replace 摘要 `{start:0,end:1}` → `derive_messages` = `[摘要, seq2, seq3, seq4]`；转写仍为 seq0..4
- [x] G1 针已写入且本票必绿部分已绿（类型存在 + replace 投影）。A/B/C 全库针可以仍红
- [x] 无新函数签名把 `buffer_index` 当身份

## Answer

In-memory `EventLog::append` assigns `seq` from 0. `fold_surface` / `derive_messages` place a replace summary at the shadowed interval; `derive_transcript_items` stays append-origin seq order. G1 vocab test + replace probe green. Full A/B/C src scan still G3.
