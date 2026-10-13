# WP-096 方案（frontend_mid · split）

- 父包：WP-094（GM，Andy 已批）。并行包：WP-095（backend_mid）。
- 审批：gm。产物：本文件 + 子包产物 `WP-094-web.md`。
- **GM 未批 plan 不 start；不 commit。协议定稿前不实现，不预设字段位置；只支持一种格式，不做新旧兼容。**

## Goal（只做一项）
Workers 名单（SubagentRosterPanel）里，**被用户停止**的子会话在状态文字上加标记。

## 现状落点（只读核实，2026-10-13）
- **名单条目数据**：
  - `web/src/components/SubagentRosterPanel.tsx:31-33` 定义 `RosterEntry.runState`、`lastTurnReason`。
  - 赋值有两处：正常条目在 `:160-167`，取 `s.last_turn_reason`；兜底条目在 `:186-193`。
- **名单结束态文字**：`SubagentRosterPanel.tsx:338-344`。
  - 直接显示原始的 `lastTurnReason`（例如英文 `cancelled`），没有就显示 failed / idle。
  - 运行中文字在 `:324-334`（running / cancelling）。
  - 图标状态在 `:90-100`：cancelled 和 max_steps 显示 warning。
- **cancelled 的来源**：
  - 写入 `SessionInfo.last_turn_reason`：lifecycle `turn_finished` 的 `reason` 在 `stores/sessionStore.ts`（turn_finished 分支），`session/list` 里也带这个字段。
  - 后端目前没有"谁停的"信息。
- **tool card 用的推导函数不同**：
  - tool card 走 `web/src/lib/subagentUi.ts:67-88` 的 `childStatusWord`，结束后也返回原始 `last_turn_reason`。
  - 使用处：`toolviews/SubagentLaunchToolView.tsx:30`，以及 `SubagentSendToolView.tsx:32` 经 `sendStatusWord`（`subagentUi.ts:90-96`）。
  - **名单不经过 `childStatusWord`**，有自己的文字逻辑。所以只改名单时，tool card 自然保持不变。
- **现有文案语言**：
  - 状态文字是英文原始值（running / cancelling / cancelled / idle / failed）。
  - 行内按钮是中文：「停止」「删除」（`:289-309`）。
  - tool card 有中文「已删除」。

## 协议占位（等 WP-095 定稿）
- **字段名和来源都待定**。可能的来源有三个：
  1. lifecycle `turn_finished` 带 `by`；
  2. `session/list` 的 `SessionInfo` 带上一次停止的来源；
  3. binding state 带。
- 前端只按定稿的那**一种**格式实现：
  - `by:"user"` 才显示标记；
  - `by:"parent"` 不显示；
  - 字段缺失不显示；
  - 只在 reason 为 cancelled 时才考虑显示。
- **重连 / 刷新后标记是否还在，取决于后端持久化**。如果 `by` 只出现在实时通知里，重连后拉 `session/list` 时就会丢。请 backend_mid 确认是否持久化，以及是否进入 `session/list` 或快照。

## 文案建议
- 和名单现有的英文状态文字保持一致：**`cancelled · by you`**。
- 如果 GM 想让名单状态改用中文，那就整列一起改成中文（**另开一项，不在本包**），届时用 `已取消 · 由你停止`。
- 本包默认用英文。

## 子包
| 子包 | owner | 域 | 产物 |
|------|-------|----|------|
| **WP-097** | `frontend`（前端-web） | client-ux-web | `WP-094-web.md` |

## web 落点（等协议定稿后实施）
- `api/types.ts`：按定稿加 `by` 字段（只加一种）。
- 数据进入 store：
  - 如果来源是 lifecycle 或 list，改 `stores/sessionStore.ts`，写入 `SessionInfo`；
  - 如果来源是 binding state，改 `stores/messageStore.ts`。
- `SubagentRosterPanel.tsx`：
  - `RosterEntry` 加 `stoppedBy`，在 `:160-167` 和 `:186-193` 两处赋值；
  - `:338-344` 在 `lastTurnReason==="cancelled" && stoppedBy==="user"` 时显示 `cancelled · by you`，其余逻辑不变；
  - 加 testid / data 属性方便测试。
- 不改 `subagentUi.ts` 的 `childStatusWord`，也不改 tool card（除非 GM 裁定 tool card 也要跟进）。

## 测试清单（vitest）
1. `cancelled` + `by:user`：名单显示 `cancelled · by you`。
2. `cancelled` + `by:parent`：显示 `cancelled`，无标记。
3. `cancelled` + 字段缺失：显示 `cancelled`，无标记。
4. 非 cancelled（completed / error / max_steps）+ `by:user`：无标记。
5. 重连后（session/list 或快照重新加载）标记仍在（前提是后端持久化，由协议决定）。
6. 新一轮 turn 开始或结束后，旧标记清掉，不残留。
7. 回归：tool card 文字不变；名单的停止 / 删除 / 计数行为不变。

## desktop
desktop 内嵌 web，预计不用改。只做静态核对（`git status desktop` + 加载同一套 web/dist），真机由 Andy 验。

## 不在范围
- tool card 的标记（待 GM 定）。
- 名单状态整列改中文。
- 父会话自己停止时的标记，以及其他来源的显示。
- 后端（WP-095）。
- 不 commit。

## 验收标准
- 测试 1–7 全绿；`tsc -b` 0 错误；lint 不适用（web 未配置 lint 工具）。
- 和 WP-095 静态 e2e 一致：字段名、位置、取值逐项对上。
- desktop 静态核对通过；未 commit。

## 待 GM / backend_mid 确认
1. **GM**：tool card（launch / send 那一行）是否也要显示「by you」？默认不跟进。
2. **GM**：文案用英文 `cancelled · by you`（默认，和现有状态一致），还是 `已取消 · 由你停止`？
3. **backend_mid**：`by` 放在哪里（lifecycle `turn_finished` / `SessionInfo` / binding state），具体字段名是什么。
4. **backend_mid**：`by` 是否持久化，是否出现在 `session/list` 或快照里，以保证重连后还在。
5. **backend_mid**：新一轮 turn 时 `by` 是否清空（或者只随 `last_turn_reason` 一起更新）。

---
## 中层交叉审（frontend_mid · 2026-10-13 HKT）
- 子包 WP-097 done；web 落地与 GM 口径一致（名单-only / `cancelled · by you` / 只信字段）。
- 证据：vitest **7/7**（`SubagentRosterPanel.cancelledBy.test.tsx`，`/exec-daemon/node` v22）；`tsc -b` **0**；lint N/A。
- 协议对齐 WP-095：`session/list` → `SessionInfo.last_turn_cancelled_by`；lifecycle `turn_finished` → `cancelled_by`（显式 null）；**不进 snapshot**；重连靠 list。静态对照 `protocol.rs` / `status_track.rs` / `connection.rs` 字段名一致。
- desktop：源码未改（`git status desktop` clean）；sidecar 加载同一 web/dist；真机 Andy 验。
- **未 commit**。准备 deliver 交 GM 验收。
