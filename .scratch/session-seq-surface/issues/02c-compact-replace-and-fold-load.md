# 02c: compact 只写 replace，turn 加载只 fold

**What to build:** 压缩成功 = 日志里多一条 `item/user` + `surface_op: replace`。Turn 工作集 = `derive_messages(fold_surface(load_events))`。禁止再 `INSERT compact_checkpoint`、禁止再用 `checkpoint_seq`/`kept_from_seq` 当窗口、禁止 SQL `ORDER BY CASE WHEN kind=compact_checkpoint`。这两步必须同一张票：只停指针、不改加载，窗口会空；只改加载、不写 replace，窗口会变成未压缩的全历史。

**Blocked by:** 02b: append-origin 事件按 seq 读写

**Status:** resolved

- [x] 删除生产路径 `apply_compact_checkpoint*`；调用方改为 append replace（`source_seqs` 覆盖被阴影 surface 节点）
- [x] 不再 UPDATE `checkpoint_seq` / `kept_from_seq` 当作模型窗口（B2 存储侧）
- [x] `SQL_LOAD_TURN_TRANSCRIPT` 的 CASE WHEN 重排删除（B1）；turn 加载按 seq ASC 读事件再 fold
- [x] 探针：磁盘上 detail 0..4 + replace `{start:0,end:1}` → `derive_messages` = `[摘要, seq2, seq3, seq4]`；append-origin 转写仍为 seq0..4
- [x] 旧 compact SQL 测试改断言 surface 或删除；不用 `#[ignore]`
- [x] 允许残破：agent loop 内存里仍可能握着派生 `Item[]`（03 再拔第二工作集）；线协议仍可说 `buffer_index`

## Answer

`load_transcript` is `derive_messages(load_events)`. Compact appends `surface_op: replace` and does not write window pointers. `apply_compact_checkpoint*` names remain as a thin persist wrapper until callers are renamed.
