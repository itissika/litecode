# 04: 线协议只说 seq

**What to build:** 前后端之间指一行只用 `seq`。加载窗口是 seq 区间，不是「从第 40 条」。新追加的事件逐条投影出去，不是用长度差冒充新下标。方法名可以暂时仍叫 `buffer/*`，字段语义必须是 seq；把旧字段名映射成下标视为没死。本票结束后前端允许编译红——红着的引用就是下一张的地图。

**Blocked by:** 03: Loop / prepare 只读 surface

**Status:** resolved

- [x] 线上事件 payload 含 `seq` + 事件类型 + 可选 `surface_op`；无 `buffer_index` 身份字段（E1、E2、E4）
- [x] 加载：`from_seq` / `to_seq`（seq 半开区间），禁止平行 `indices[]` / `kinds[]` 与条数窗口（A8、E3）
- [x] snapshot 用 `last_seq` / `next_seq`，不用 `buffer.len` 当下一条下标（A9、E5）
- [x] compact 通知不再 `bump` 一个位置窗口，也不再 `buffer/compacted` 重载 last-40（C3、C5）
- [x] `src/` identifier 扫描：G4 扫 `client_protocol/` + `runtime/observer.rs`（门禁文件豁免；store/search 指针名留给 05）
- [x] 线协议测试断言 replace 的 `surface_op` 与 cut 范围，而不是 `buffer_index == 3`
- [x] P3 探针：cut 插在 shadowed 边界（纯函数即可）

## Answer

Wire `buffer/item` is `{ seq, type, surface_op, item }`. `buffer/load` is `[from_seq, to_seq)` of log events. Snapshot buffer is `last_seq` / `next_seq`. Compact success emits the replace event by seq, not `buffer/compacted` last-40. Frontend still on `buffer_index` (06). Store `load_by_buffer_index` / `kept_from_seq` remain for 05.
