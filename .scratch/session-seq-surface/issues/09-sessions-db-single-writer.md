# 09: sessions.db 只有一把写连接

**What to build:** 活着的 session 已经通过 `SessionGate` 握着 `sessions.db` 的写连接时，其它路径不得再 `Connection::open` + `ensure_session_schema` / FTS backfill 当第二写者。读者只读；schema 迁移只走那把活连接。本票修锁与连接纪律，不改 seq / surface 合同，不修 FoldCard。

**Blocked by:** 05: 边路只消费 seq / surface

**Status:** ready-for-agent

05 修的是 search **排除集**去 `Session::resume`（第二把写连接 + 迁 schema）。同一把库上的双写锁还在：

- `search_lexical` 仍 `SQLITE_OPEN_READ_WRITE`，请求路径上 `transcript_fts::ensure_schema` + `backfill_fts`
- `Session::list_sessions` / `list_child_session_ids` / `find_latest_by_project`、以及 registry 未命中时的 `is_child_session` / `is_session_empty`，仍 `Connection::open` + `ensure_session_schema`

和 live `SessionGate` 并发时会 `SQLITE_BUSY` / schema lock，不是 seq 身份问题。不要塞进 06（FE Map）或 08（围栏 + 死皮 + tsc）。

- [ ] search / 列表 / 「是否子 session」等读者不得为了读去 `resume` 或迁 schema
- [ ] FTS `ensure_schema` / backfill 不在与 live writer 并行的请求路径上另开写连接（走 gate，或启动时迁一次，或只读查询）
- [ ] 探针：活 session 正在 insert/commit 时 search 或 list 不得因第二写者失败；不得用加长 `busy_timeout` 当完成

## Comments

05 审查：`load_surface_seqs` 一度 `Session::resume`，与 search 其它只读打开不一致。那条已收。lexical FTS 仍是写打开，是本票地图。
