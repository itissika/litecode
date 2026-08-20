# 02: 持久化权威是事件日志

**What to build:** 磁盘上的 session 加载后就是连续 `SessionEvent` 序列：按 seq ASC，不在 SQL 里重排。压缩成功写入 replace 事件，而不是改 `checkpoint_seq` / `kept_from_seq` 当模型窗口，也不是 `kind=compact_checkpoint` 插在 `MAX(seq)+1` 再当 cut。砸库允许。此时还不要求 agent loop 或前端改完。

**Blocked by:** 01: SessionEvent 信封与内存 surface

**Status:** ready-for-agent

- [ ] 加载路径不再使用 `ORDER BY CASE WHEN kind=compact_checkpoint` 把摘要提到前面（B1）
- [ ] `checkpoint_seq` + `kept_from_seq` 不再是加载/派生模型窗口的权威（B2 存储侧）；派生只 fold 已加载事件
- [ ] 旧 `apply_compact_checkpoint*` 生产路径删除；compact = `append` 带 `surface_op: replace`
- [ ] 不变量：加载后 `event.seq` 等于磁盘 seq 列，且连续（无「enumerate(history) 冒充 seq」）
- [ ] 旧 compact SQL 测试改为断言 surface，或删除；不用 `#[ignore]` 假装完成
