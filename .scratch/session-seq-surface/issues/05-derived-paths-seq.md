# 05: 边路只消费 seq / surface

**What to build:** search、revert、snapshot、subagent 与主路径读同一条日志。排除「当前模型窗口」= 排除当前 `surface.nodes`；被阴影的 append-origin 仍在 log 里所以仍可搜。Revert 在服务端落到 `anchor_seq`（RPC 仍可收第 k 条 user，但内部不再靠 `buffer_index` 或 `compact_checkpoint` kind 猜锚点）。本票是后端；不修 FoldCard。

**Blocked by:** 04: 线协议只说 seq

**Status:** resolved

02c 停写窗口指针之后，这些边路**已经在算错**，不是 05 才开始存在的活。本票必须把读者改到 surface，并把因换皮而红的测试收绿——不得再把「全量红着等 08」当完成。04 允许红的只有前端协议字段。

- [x] `session_search` 不以 `kept_from_seq` 为窗口权威；排除集 = 当前 surface 的 seq 集合（G6）
- [x] revert 内部是 `anchor_seq`；排除锚点不靠 `kind === compact_checkpoint`（C4）
- [x] 摘要 replace 行不得占一个 user 锚点 k；`user_detail_count` / `SQL_ANCHOR_SEQ` 与 compact 后的 k 合同一致
- [x] snapshot / file-revert 内部存 seq，不是 user 条数冒充身份
- [x] 回合投影不得把 `next_seq` 塞进仍叫 `committed_start` 的下标字段（删或改名为 seq）
- [x] subagent 列表行用 parent seq；`call_id` 只用于 tool 配对
- [x] G6：`kept_from_seq` / `checkpoint_seq` 作为窗口权威的用法清零（含 revert 末尾按 `kind=compact_checkpoint` 回写指针）
- [x] 本票结束 `cargo test` 必绿（至少）：`compacted_history_below_kept_from_seq_remains_searchable`、`revert_contract_three_states`；Windows 上 git CRLF / code-search warmup 不在范围

## Answer

Search exclude = `fold_surface` nodes via read-only SQLite (no `Session::resume`). Revert k counts only `surface_op=append` user rows; truncate by `anchor_seq`; no pointer UPDATE. Snapshot track/record uses `next_seq` as the file stem; file-revert maps user k → `anchor_seq + 1`. Restore no longer bounds stem against `user_detail_count`. `TurnCompleted.committed_next_seq` replaces `committed_start`. Subagent bind still `call_id`; parent row identity is seq via `find_function_call_event` (04). Schema `kept_from_seq` columns remain until 08.

## Comments

04 全量跑出的边路红：search 仍读停写的 `kept_from_seq`（整段当前 session 被当 live 排除）；revert 把 `kind=detail` 的摘要算进 k。这是 02c 切开写/读的预期活，当时没写进完成条件。

File-revert RPC 仍收第 k 条 user；磁盘 stem 是该条 user 落盘后的 `next_seq`。
