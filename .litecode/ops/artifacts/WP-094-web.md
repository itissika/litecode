
---
## WP-097 实施记录（前端-web；2026-10-13 HKT）
口径：GM 定稿——只改 Workers 名单；英文 `cancelled · by you`，仅 `last_turn_reason==='cancelled'` 且 `last_turn_cancelled_by==='user'` 显示；parent / null / 缺失 / 其他原因不显示。tool card 不动。
**GM 裁定**：stop_and_send 内部触发的停止记为 `parent`，不显示标记。前端只信字段，不自行推断。
**更正（前端中层）**：`last_turn_cancelled_by` 不进快照。已回退方案里"快照类型加字段 + applySnapshot 合并"两处；字段只来自 `session/list` 的 SessionInfo 和 lifecycle `turn_finished`，重连靠 connected 时那次 list 拿回。测试第 5 条改为按 list 测。

### 改动（只改 web/）
- `api/types.ts`：`CancelledBy = "user" | "parent" | null`；`SessionInfo.last_turn_cancelled_by?`（紧邻 last_turn_reason）；`SessionLifecycle.last_turn_cancelled_by?`。
- `stores/sessionStore.ts`：lifecycle turn_finished 与 reason 同步写入（带 key 用其值含 null；不带 key 但有新 reason → 清空，防继承；都不带 → 保留）；未知会话 upsert 同样带上；turn_started 清空。session/list 整行替换，字段自然带入。
- `components/SubagentRosterPanel.tsx`：`RosterEntry.stoppedBy`（两处构建赋值）；状态静态分支满足条件时显示 `cancelled · by you`（`data-testid="subagent-roster-cancelled-by-user"`），其余文字/颜色/停止/删除/计数不变。
- 新测试 `components/SubagentRosterPanel.cancelledBy.test.tsx`：第 1–7 条（7 例）。

### 证据（/exec-daemon/node v22.14.0）
- 新测：7 passed。
- web 全量：Test Files 174 passed (174)，Tests 1287 passed (1287)。（比 WP-093 后的 1274 多 13：本包 7 + 其它并行改动的 6，非本包新增。）
- `tsc -b` exit 0。lint 不适用。desktop 未改。未 commit。

### WP-097 返工（验收退回，2026-10-13 HKT）
- 原因：lifecycle `turn_finished` 的 wire 键名是 `cancelled_by`（与 `reason` 并列），不是 `last_turn_cancelled_by`。
- `api/types.ts`：`SessionLifecycle.cancelled_by?: CancelledBy`（原 last_turn_cancelled_by 移除）；`SessionInfo.last_turn_cancelled_by` 不动。
- `stores/sessionStore.ts`：读 `params.cancelled_by`（`"cancelled_by" in params` 判断显式 null；缺失容忍逻辑不变），写入会话行 `last_turn_cancelled_by`。
- 测试：事件改用 `cancelled_by`；第 6 条补"显式 null 清掉"和"事件上的旧键名 last_turn_cancelled_by 不被读取"。
- rg 核对：web/src 非测试代码里 `last_turn_cancelled_by` 只剩 SessionInfo 类型与写入会话行处，事件路径无旧键名。
- 证据（node v22.14.0）：新测 7 passed；全量 Test Files 174 / Tests 1287 passed；`tsc -b` exit 0。未 commit。
