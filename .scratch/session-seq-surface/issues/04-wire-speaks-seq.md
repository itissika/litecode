# 04: 线协议只说 seq

**What to build:** 前后端之间指一行只用 `seq`。加载窗口是 seq 区间，不是「从第 40 条」。新追加的事件逐条投影出去，不是用长度差冒充新下标。方法名可以暂时仍叫 `buffer/*`，字段语义必须是 seq；把旧字段名映射成下标视为没死。本票结束后前端允许编译红——红着的引用就是下一张的地图。

**Blocked by:** 03: Loop / prepare 只读 surface

**Status:** ready-for-agent

- [ ] 线上事件 payload 含 `seq` + 事件类型 + 可选 `surface_op`；无 `buffer_index` 身份字段（E1、E2、E4）
- [ ] 加载：`from_seq` / `to_seq`（seq 半开区间），禁止平行 `indices[]` / `kinds[]` 与条数窗口（A8、E3）
- [ ] snapshot 用 `last_seq` / `next_seq`，不用 `buffer.len` 当下一条下标（A9、E5）
- [ ] compact 通知不再 `bump` 一个位置窗口，也不再 `buffer/compacted` 重载 last-40（C3、C5）
- [ ] `src/` identifier 扫描：`buffer_index` / `bufferIndex` / `kept_from_seq` / `checkpoint_seq` / `compact_checkpoint` 生产路径为零（门禁文件豁免）
- [ ] 线协议测试断言 replace 的 `surface_op` 与 cut 范围，而不是 `buffer_index == 3`
- [ ] P3 探针：cut 插在 shadowed 边界（纯函数即可）
