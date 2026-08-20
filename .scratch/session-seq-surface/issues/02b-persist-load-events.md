# 02b: append-origin 事件按 seq 读写

**What to build:** store 能把一条 append 事件原样读回：加载顺序是 seq ASC，`event.seq` 等于磁盘 `seq` 且连续。新增 `load_events`（或等价）作为日志 API。`load_transcript` 若仍存在，只能从事件派生，不得另排。本票仍不切换 compact，也不删 SQL 重排——turn 窗口可以暂时继续旧查询，避免 02b 单独合入时模型看到「全历史无摘要」。

**Blocked by:** 02a: 砸库加上事件信封列

**Status:** resolved

- [x] 写入走分配器给出的 seq（与磁盘列同一套），不是事后 `enumerate(history)`
- [x] `load_events`：seq ASC，连续，往返后 `derive_transcript_items` 与插入的 append-origin 一致
- [x] 不变量测试：加载后 `event.seq == 行.seq`，无空洞
- [x] 不在本票删除 `SQL_LOAD_TURN_TRANSCRIPT` 的 CASE WHEN 重排（留给 02c）
- [x] 不在本票把 compact 改成 replace

## Answer

`Session::load_events` reads seq ASC and rejects holes. `load_transcript` still uses the old turn SQL view so compact windows stay intact until 02c.
