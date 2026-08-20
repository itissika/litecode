# 06: 前端唯一 Map&lt;seq, Event&gt;

**What to build:** 面板上每一行的身份是 `seq`。加载与 live 写入同一张 map，按 seq 排序做转写投影。流式 delta 打到 `output_item.added` 已分配的那个 seq，不得用 provider `item_id` 另开 `live-*` 行，也不得 `finalizeTurn` 删未封口行。没有 seq 的行不得进 store。本票不要求 FoldCard 活度正确。

**Blocked by:** 04: 线协议只说 seq

**Status:** ready-for-agent

04 留下的前端红是本票地图，不是意外：`buffer_index` / `buffer.len` / `committed_end` / `start`–`end` 加载窗。不要在 06 之前用 overlay 特例「修」compact 乱序。

- [ ] `ChatRow` 必有 `seq`；删除 `bufferIndex` 与 `live-` / `ord-` / `user-*` 身份协议（A1、A2）
- [ ] 删除 `orderProjection`、`findRowByItemId` / `findRowForSeal` / `vacateIndex` / `sealProjectionRow`（A3、A4）
- [ ] 删除 `finalizeTurn` 丢 `live-*`，以及 overlay 生命周期补洞（A5、A6）
- [ ] 乐观 user：拿到 seq 前最多一个 pending 槽，且不进入排序键空间
- [ ] 探针：已封口 seq 再来相同 `item_id` 的 delta，不得变异该 seq（G4）
- [ ] FE 死亡清单 A 类（D 除外）清零；禁止「没 seq 再 fallback item_id」
