# 02a: 砸库加上事件信封列

**What to build:** `transcript_items` 能放下 `SessionEvent` 信封（`event_type`、`surface_op`、`source_seqs`），旧库形状 fail-closed。本票不改 compact 语义、不改模型窗口。新列必须有默认或由现有 insert 写上，避免半截 schema。

**Blocked by:** 01: SessionEvent 信封与内存 surface

**Status:** resolved

- [x] CREATE 含信封列；缺列的旧 `sessions.db` 拒绝打开（砸库政策，无 ALTER 兼容）
- [x] `insert_detail_rows`（及现有 append-origin 写入）给新列写入合法值：表面行至少 `event_type` + `surface_op=append`，seq 仍是磁盘列
- [x] compact 仍可走现有 checkpoint 路径（本票不删 `apply_compact_checkpoint*`）
- [x] schema 测试断言新列存在，且半旧库仍 fail-closed

## Answer

Schema fail-closed on missing `event_type` / `surface_op` / `source_seqs`. Detail and checkpoint inserts write `item/*` + JSON `append`. Compact pointers unchanged.
