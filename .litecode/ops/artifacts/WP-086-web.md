# WP-089 方案 v2（前端-web；按 WP-086-backend.md §6 定稿补齐；只改 web/，不 commit，未批不 start）
口径：WP-086-frontend.md + WP-086-backend.md §5/§6 + GM 从简。子会话只看/停/删；子视图不加输入框（AgentChatInput subagent 分支保持现状）。新字段一律容忍缺失。

## A. 停止 / 删除 / 删除后 / 重连 / 状态（与 v1 同，按定稿微调）
1. 停止：AgentChatInput.tsx subagent 底栏 + SubagentRosterPanel.tsx 每行（不破坏点行跳转）；复用 turnStore.ts `agent/cancel {session_id: child}`；仅 running/running_with_subagent/stopping 可用；`{cancelled:false}` → 中性 toast「没有在运行的回合」，不改状态。
2. 删除：Roster 每行；子会话自身 status 非 idle（running / running_with_subagent / stopping）时置灰（P4 定稿保留置灰）；先确认框，确认后 `session/delete {id: child}`（复用 sessionStore.ts 删除逻辑或抽公共函数）。
3. deleted 事件 `session/lifecycle {event:"deleted", session_id, parent_session_id}`：sessionStore.ts deleted 分支关 `agent-<id>` + `subagent-<id>`（sessionPanelNav.ts）；按 parent_session_id 记父会话已删集合；级联时每条各自处理；Roster 兜底行、SessionStatusLine 计数排除；subagentUi.ts childStatusWord→「已删除」；SubagentInlineLine.tsx 不可点。
4. 重连：connectionStore.ts 进入 connected 时统一一次 `session/list`（含子会话、原始 status）；SessionList/Roster 挂载刷新保留并去重。
5. 状态：sessionStore.ts lifecycle `turn_started`/`turn_finished` 有 `status` 直接用（含 running_with_subagent、stopping），无则回退原逻辑；**新增 `event:"status_changed"` 分支**：只更新该会话 status，不动其它回合状态；缺 status 忽略。cancelling→stopping 原逻辑不变。

## B. 原阻塞项（定稿补齐）
6. 墓碑（只此一种形状，不做新旧兼容）：
   - types.ts：`subagent_bindings` 保持 `Record<string,string>` 不变；新增 `subagent_binding_states?: Record<string /*call_id*/, { child_session_id: string; deleted: boolean }>`（snapshot 与 buffer load 两处类型都加）。
   - messageStore.ts：现有 subagent_bindings 合并处同时合并 binding_states（snapshot 覆盖、buffer load 合并）；派生 deletedChildIds；字段缺失 / 某 call_id 无条目 = 未删除。
   - 与 A3 的 deleted 事件集合并集使用：卡片「已删除」不可点、名单/计数排除。
7. 错误码映射：types.ts 新增常量 `subagent_session_readonly`、`agent_already_running`、`descendant_running`（wire 字符串，-32000）；映射集中放在一个小 helper（如 lib/subagentUi.ts 或新 lib/sessionErrors.ts），sessionStore.ts 删除失败处调用：
   - agent_already_running →「子会话正在运行，先停止再删除」
   - descendant_running →「有后代子会话在运行，无法删除」（data.running_session_ids 存在则附数量，缺失照常提示）
   - 其它 → 通用失败提示；列表不做乐观删除/或回滚。
   - subagent_session_readonly：前端无入口；若 run/enqueue/retry 意外收到，映射为只读提示（不新增入口）。

## C. 测试（vitest /exec-daemon/node v22）+ tsc -b
T1 子视图无输入框/发送；停止可用态；点击发 agent/cancel{child}。
T2 cancelled:false → 中性 toast，状态不变。
T3 Roster 停/删；running/running_with_subagent/stopping 删除置灰；确认取消不发 RPC，确认发 session/delete。
T4 deleted 事件（带 parent_session_id，含级联多条）：两面板关；卡片「已删除」不可点；名单计数排除。
T5 重连 connected 时 session/list 只调一次，不依赖 SessionList 挂载；running_with_subagent 不被覆盖。
T6 turn_finished 带 status:running_with_subagent 保持；带 stopping 显示 stopping；无 status 回退 idle。
T7 回归：主会话取消/删除不变；AskCard/PermissionModal 不受影响。
新增：
T8 墓碑：binding_states deleted=true → 已删除（snapshot 与 buffer load 各一）；deleted=false → 正常；字段缺失 / 无对应 call_id → 视为未删除。
T9 拒绝码：agent_already_running、descendant_running（有/无 running_session_ids）各自明确提示；未知码 → 通用提示；列表状态不变。
T10 status_changed：更新父会话 status（如 running_with_subagent → idle）；缺 status 忽略。
T11 readonly 码 → 只读提示（helper 单测即可）。

产物：本文件（实施记录后续追加）。禁区：desktop / 后端 / 引擎 UI / symlink / client_protocol。

---
## 实施记录（WP-089，2026-10-13 HKT；前端中层已批 → start）
按定稿 WP-086-backend.md §5/§6 落地。只改 web/，未 commit。

### 改动文件
- `api/types.ts`：`SessionStatus` 类型；`SubagentBindingState`；`BufferLoaded` / `SessionSnapshot` 加 `subagent_binding_states?`；`SessionLifecycle.status?`；事件加 `status_changed`；错误码常量 `RPC_ERR_SUBAGENT_SESSION_READONLY / AGENT_ALREADY_RUNNING / DESCENDANT_RUNNING`。
- `api/agentWs.ts`：RPC 错误 reject 时保留 `rpcCode` 和 `data`（data.code / running_session_ids），message 不变。
- `lib/sessionErrors.ts`（新）：`rpcErrorCode`、`deleteRejectedMessage`（两种拒绝码各自提示，其余通用）、`readonlyRejectedMessage`。
- `lib/subagentUi.ts`：`deletedChildIds`（binding_states deleted=true ∪ deleted 事件；缺失=未删）、`sessionBusy`、`childStatusWord(..., deleted)`→「已删除」。
- `lib/useDeletedChildIds.ts`（新）：按父会话取已删集合的 hook。
- `stores/messageStore.ts`：slice 加 `subagentBindingStates?`；buffer load 合并；新 `applySubagentBindingStates`（snapshot 用）。
- `stores/sessionStore.ts`：snapshot 合并 binding_states；deleted 分支加关 `subagent-<id>` 并记 `deletedSessionIds`；`status_changed` 只改 status；turn_started/turn_finished 优先原始 status、缺失回退；删除被拒按码提示（无码保持原英文通用文案）；新 `stopSubagent`（cancelled:false → info toast「没有在运行的回合」）；`listSessions` 加 in-flight 去重。
- `stores/connectionStore.ts`：进入 connected 统一调一次 `listSessions`。SessionList / Roster 挂载刷新保留，由去重防重复请求。
- `components/AgentChatInput.tsx`：subagent 分支底栏加「停止」（在跑/stopping 才可用），仍无输入框。
- `components/SubagentRosterPanel.tsx`：每行加「停止」「删除」（在跑含 running_with_subagent/stopping 时删除置灰，删除先 `window.confirm`）；排除已删除子会话。点行跳转不变。
- `components/SessionStatusLine.tsx`：total/running 排除已删除。
- `components/toolviews/SubagentLaunchToolView.tsx`、`SubagentSendToolView.tsx`：已删除 → 「已删除」，不可点（不传 childId）。
- 测试：新增 `components/SubagentOps.wp089.test.tsx`（22 例，T1–T11）；`SubagentRosterPanel.test.tsx`、`SessionStatusLine.test.tsx` 各 1 处按钮查询收窄到 `name: /^Subagent /`（新增行内按钮所致，断言意图不变）。

### 与 v2 方案的差异 / 说明
- readonly 码只提供映射 helper + 单测，未接到 run/enqueue/retry 的现有错误路径（前端无子会话发消息入口）。
- 删除被拒：有 `data.code` 才走中文映射；无码（如断线）保留原 "Failed to delete session"。
- T5 直接测的是 listSessions 去重 + running_with_subagent 不丢；connected 触发是一行接线，未单独做 connectionStore 集成测。

### 证据（/exec-daemon/node v22.14.0）
- 相关：`vitest run src/components/SubagentOps.wp089.test.tsx` → 22 passed。
- 全量：`vitest run` → Test Files 171 passed (171)，Tests 1266 passed (1266)（基线 1244 + 新增 22）。
- `tsc -b` → exit 0，0 错误。lint：web 未配置，不适用。
- desktop / 后端 / 引擎 UI / symlink / client_protocol 未改；未 commit。

---
## WP-093（listSessions 补拉一次 + 重连接线测试；2026-10-13 HKT）
### 改动
- `web/src/stores/sessionStore.ts`：新增模块级 `listPending`。进行中再来的请求只置 `listPending=true`（多次合并为一次）；请求 `.finally(settle)` 时清 in-flight，若 pending 则清标志并补拉一次（失败后同样补一次，不重试循环）。`resetListSessionsInFlight` 同时清 `listPending`。调用点未改。
- 新增 `web/src/stores/sessionStore.listDedupe.test.ts`：L1–L5（5 例）。
- 新增 `web/src/stores/connectionStore.reconnect.test.ts`：R1–R3（3 例）。mock `../api/agentWs`（捕获 `onConnectionChange`）和 `../api/auth`，`attachSiblingStores` 注入假 session store，`afterEach destroy()`。mock 链不长、不脆，R1–R3 全留，无需降级。
### 证据（/exec-daemon/node v22.14.0）
- 相关：listDedupe + reconnect → 8 passed；连同 SubagentOps.wp089 / connectionStore.test → 4 files 37 passed。
- 全量：Test Files 173 passed (173)，Tests 1274 passed (1274)（1266 + 8）。
- `tsc -b` exit 0。未 commit；只动 web/ 与 ops 记录。
