# 05: 边路只消费 seq / surface

**What to build:** search、revert、snapshot、subagent 与主路径读同一条日志。排除「当前模型窗口」= 排除当前 `surface.nodes`；被阴影的 append-origin 仍在 log 里所以仍可搜。Revert 在服务端落到 `anchor_seq`（RPC 仍可收第 k 条 user，但内部不再靠 `buffer_index` 或 `compact_checkpoint` kind 猜锚点）。本票是后端；不修 FoldCard。

**Blocked by:** 04: 线协议只说 seq

**Status:** ready-for-agent

- [ ] `session_search` 不以 `kept_from_seq` 为窗口权威；排除集 = 当前 surface 的 seq 集合（G6）
- [ ] revert 内部是 `anchor_seq`；排除锚点不靠 `kind === compact_checkpoint`（C4）
- [ ] snapshot / file-revert 内部存 seq，不是 user 条数冒充身份
- [ ] subagent 列表行用 parent seq；`call_id` 只用于 tool 配对
- [ ] G6：`kept_from_seq` / `checkpoint_seq` 作为窗口权威的用法清零
