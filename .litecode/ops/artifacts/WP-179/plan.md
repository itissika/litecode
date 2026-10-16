# WP-179 方案（backend_mid）— ask_user / 权限类「用户门闩」等待态（仅方案，禁止实现）

**口径（Andy / GM 2026-10-16）：** ask_user 语义明确，**与重连无关**——调用后 session 进入「等待用户回答」。本包只论证与出方案，**禁止改 `src/` / web / desktop**。实现另立修复包，须 GM 批本方案后再开。

**产物路径：** `.litecode/ops/artifacts/WP-179/`

---

## 1. 现状 as-is（只读调研，file:line）

### 1.1 ask_user 与权限共用 PermissionSink

| 路径 | 行为 |
|------|------|
| `src/tools/ask_user.rs:287-296` | `AskPrompt::ask_user` → `sink.ask`；`Deny` → 文案 `"User skipped the question(s) (no answer)."`；`Aborted` → error `"ask_user aborted (turn cancelled)"` |
| `src/tool/authorize.rs:80-104` | 工具策略 `Ask` → `sink.ask_permission`；`Deny` / `Aborted` / `Allow{always}` |
| `src/permission/grants.rs:41-43,128-138` | `AskKind::{Permission, Approval, AskUser}` 已同型挂在 `AskPrompt`；`ask_user` 带 `questions[]` |
| `src/client_protocol/permission_bridge.rs:49-94` | `WsPermissionBridge::ask`：无 WS → **Deny**（L82-83）；cancel token → **Aborted**（L85-86）；`request_id` 仅内存 UUID |
| `src/runtime/phase.rs:32-50` | `PhasePermissionSink` 发 `PermissionAwaiting{awaiting:true/false}`；Allow 后 phase→`ExecutingTools` |

### 1.2 挂起态只在连接内存，不进 session 落盘

| 路径 | 行为 |
|------|------|
| `src/client_protocol/connection.rs:1649` | `pending_asks: HashMap<String, PendingAsk>` — **进程内、连接作用域** |
| `src/client_protocol/connection.rs:1558-1570` | `reject_all_pending_asks`：Cancel / Quit / disconnect 一律 `AskOutcome::Deny`（假「用户跳过」） |
| `src/runtime/observer.rs:64-68` | `TurnPhase::AwaitingPermission { tool, rule_id, summary }` — **无 request_id / questions** |
| `src/client_protocol/protocol.rs:102-106,364` | Wire 有 `AwaitingPermission` phase + `awaiting_permission: bool`；**无挂起列表** |
| `src/client_protocol/project.rs:400` | `PermissionAwaiting` **不投影**到客户端事件（`=> None`） |

### 1.3 工具调用在物理 seq 上的封口现状

| 路径 | 行为 |
|------|------|
| `src/session/event.rs` / `model.rs` | `item/tool_call` / `item/tool_result` 成对写入 transcript |
| `src/session/data/writer.rs:310-358` | 进程死后启动：`seal_orphaned_in_progress` 把 `in_progress` 行标 Incomplete |
| `src/session/data/writer.rs:360-408` | `seal_tail_unanswered_calls`：尾部未答 `tool_call` **补 interrupt 结果封口**（诚实关闭，**非**「继续等待」） |

**结论（as-is）：** 权限等待是**内存 oneshot + HashMap**；进程退出 / 断线后 **不会**保持「等待回答」。启动修复反而会把尾部未答 tool_call **封成 interrupt**。`TurnPhase::AwaitingPermission` 存在，但不足以驱动「可回答的 UI 卡」重挂。

---

## 2. 目标语义（推荐）

### 2.1 显式 session / turn 态：`waiting_for_user`

推荐在权威状态机增加（或充实现有 `AwaitingPermission`）一档：

- **名称：** `waiting_for_user`（对外 Wire / snapshot 字段建议同名；对内可继续用 `TurnPhase::AwaitingPermission` 并扩展载荷）。
- **含义：** 工具调用（ask_user **或** 工具授权 Ask）已发出、**故意不封口**，session 停在等用户；与 `running`（模型/工具执行中）和 `idle`（无回合）区分。
- **进入：** PermissionSink 发出挂起请求且尚未收到用户回执。
- **离开：** 用户 Allow / Deny（跳过）→ 写 tool_result 封口 → 回 `running`/`ExecutingTools`；或用户 Cancel 回合 → `Aborted` + `turn_finished`（见 §5）。

**弃选：** 仅布尔 `awaiting_permission`、无挂起列表——现状已证明重连丢卡。

### 2.2 「权限类 / 用户门闩」同型

| 维度 | 方案 |
|------|------|
| 权威模型 | ask_user 与工具授权 / Approval 同属 **用户门闩**：物理 seq 上均为「已发 tool_call、故意不封口、决定 session∈waiting_for_user」的调用 |
| AskKind | 保留 `Permission \| Approval \| AskUser`（展示可两套卡） |
| 回执通道 | 同一 `agent/permission`（或现有 grant）回执；同一 pending 队列 |
| 封口 | **唯一**故意延迟封口的 tool_call 族；其余工具仍即时执行并封口 |

---

## 3. 物理落盘：推荐 **是**（带边界）

### 3.1 推荐：等待态应物理可恢复

**理由（呼应 Andy）：**

1. 「等待回答」是 **session 事实**，不是连接事实；仅内存挂起 → quit/crash/断线后语义塌缩为 idle 或假 skipped（`reject_all`→Deny）。
2. 现有 startup `seal_tail_unanswered_calls` 把尾部未答 call **封成 interrupt**，与「继续等用户」目标相反；若采纳等待态，须改为：**识别权限类未封口 call → 恢复为 waiting_for_user，而不是 seal interrupt**。
3. FE 书面意见：若落盘等待态，**snapshot/attach 必须带挂起 ask**，否则 UI 无卡可答。

### 3.2 落盘最小形状（设计草案）

- transcript：权限类 `item/tool_call` **正常写入**（与其它 tool_call 同形），**暂不写** `item/tool_result`。
- 旁路或同表元数据：`request_id`、`AskKind`、questions/options/summary/rule_id（足够 FE 重挂卡）。
- 进程重启：检测到尾部为权限类未封口 → session/turn → `waiting_for_user`，**不**跑 seal interrupt；等用户回答后再写 result 封口。
- **非**权限类未封口（真崩在 bash/grep 执行中）：仍走现有 seal Incomplete / interrupt（诚实关闭）。

### 3.3 弃选：纯内存挂起

| 代价 | 说明 |
|------|------|
| 假 skipped | 断线/`reject_all`→Deny → ask_user 文案「User skipped…」（WP-176 lock 已指此） |
| 残卡 / 丢卡 | snapshot 无挂起列表；重连 UI 无卡 |
| 与 Andy 语义冲突 | 「等待回答」无法跨进程 |

**仅当**产品明确「等待仅限当前 WS 会话、进程死即放弃」才可选内存方案；**本草案不推荐**。

---

## 4. 协议 / snapshot 字段（草案）

### 4.1 必增：`pending_permissions`

在 `session/snapshot` 与 attach/重连载荷中增加：

```text
pending_permissions: PermissionRequest[]   // absent 视为 []
```

每项与现 `agent/permission_request` **1:1 同形**；`kind` **必填**（`permission|approval|ask_user`）；字段：`request_id, turn_id, session_id, tool, rule_id, summary, kind, free_text?, options?, multi_select?, questions?`。

**FE apply：** 走现有 `onPermissionRequest` **入队**；**出卡只认列表，不认** `awaiting_permission` 布尔。

### 4.2 状态枚举

- 增加 `waiting_for_user`（或充实 `awaiting_permission` phase，载荷含上表足够字段）。
- FE 现状：`runState = idle | running | cancelling`；等用户权威=内存 `permissionQueue`。方案要求权威迁到后端 snapshot 列表 + 后端态。

### 4.3 事件保留与约束

| 事件 | 约束 |
|------|------|
| `permission_request` | 保留；新建挂起时发 |
| `agent/permission` 回执 | 保留；用户回答唯一路径 |
| `turn_started` / `turn_finished` | 保留；**物理等待态下 turn 宜未 finished**（更干净）。若已 finished，则 **不得**用 `turn_finished` 误清未答 ask |
| `permission_resolved` | **无 request_id**，FE 不用关卡；若不补 id则 **勿依赖** |

### 4.4 断线语义（与 WP-176 lock 对齐）

- **物理等待态下：断线不应一律 Deny**；Deny **仅**用户点跳过。
- 断线：挂起保留在落盘 / pending 列表；重连靠 `pending_permissions` 重挂卡。
- Cancel 回合（用户 Stop）：`Aborted`（非 Deny），且若 WS 仍连着则 `turn_finished` 必达（FE 关卡靠 turn 事件；WS 已断由 FE 自清残卡）。

---

## 5. FE 对齐

### 5.1 前端中层书面意见（已纳入，可压缩不改结论）

1. **若落盘等待态：** snapshot/attach 必须带挂起 ask（`request_id`+questions）或同型权限请求，否则 UI 无卡可答。
2. **ask_user 与工具授权：** UI 可两套卡，但同一 `permissionQueue` + `agent/permission` 回执；契约上「同型权限等待态」FE 同意；展示可仍分两种卡。
3. **断线：** 若物理等待态，断线不应一律 Deny；Deny 只留用户点跳过（与 WP-176 lock 一致）。
4. **不落盘代价：** 内存挂起 + 断线 Deny → 假 skipped / 残卡。
5. **现状 FE：** 无单独 `waiting_for_user`；`runState=idle|running|cancelling`；等用户权威=内存 `permissionQueue`；snapshot 只有 `awaiting_permission` 布尔/phase，**不带挂起列表 → 重连丢卡**。
6. **方案必须：** `pending_permissions: PermissionRequest[]`（覆盖 permission|approval|ask_user），absent=`[]`；apply 走 `onPermissionRequest`；出卡只认列表。状态加 `waiting_for_user`（或充实 awaiting_permission）。物理等待态下 `turn_finished` 不得误清未答 ask；更干净是等待态时 turn 未 finished。保留 turn_started/finished、permission_request、permission 回执、AskKind。`permission_resolved` 无 request_id、FE 不用；若不补 id 则勿依赖。

**标记：前端中层书面意见已纳入；前端中层 2026-10-16 终审同意（见 §5.2）。**

### 5.2 前端中层终审（2026-10-16）— **同意，已拍板**

1. **对外字段名：`waiting_for_user`**
   - Wire / snapshot / 对外文档用 `waiting_for_user`。
   - 对内可继续 `TurnPhase::AwaitingPermission` 并扩展载荷，不必强行改 Rust 枚举名。
   - 现有 `awaiting_permission: bool`：实现阶段改为 **派生**（`pending_permissions.length > 0`），或逐步废弃；列表圆点可继续吃布尔/新态，但 **出卡只认列表**。

2. **PermissionRequest 与现 `agent/permission_request` 事件 1:1**
   - 必须同形，FE 才能直接走 `onPermissionRequest`。
   - snapshot 数组元素：`kind` **定为必填**（现事件上 optional、默认 permission）；其余与现 `PermissionRequest` 一致：`request_id, turn_id, session_id, tool, rule_id, summary, kind, free_text?, options?, multi_select?, questions?`。

3. **等待态时 turn 未 finished（采纳）**
   - 拒绝「finished + 禁止清卡」（与 FE「running 出卡、finished 清队列」冲突）。
   - 用户 Stop/Cancel → Aborted → 再 `turn_finished` 并清空挂起（与 §4.4 一致）。

4. **desktop 与 web 同一 snapshot 契约**
   - 同一 `pending_permissions` / `waiting_for_user`；不得另起字段名或另一套列表形状。
   - 实现阶段 web 先出卡；desktop 今日无 ask_user UI，可后做面板。

无其他修订。WP-176 lock（断线≠一律 Deny）引用正确。

---

## 6. 显式非目标（本包）

- **不改任何 `src/` / web / desktop 行为**；不开修复实现子包。
- 不在本包改 `AskOutcome` / `reject_all_pending_asks` 实现（方案已定方向；落地归后续修复包）。
- 不与 WP-175 Luna 崩溃绑定因果（Andy：两机各自问题）。
- WP-176/177 修复实现不在本包范围；仅引用断线≠Deny 的 lock 口径。

---

## 7. 后续修复包建议（仅建议，本包不建）

GM 已批本方案且 FE 已终审；实现另立包（须 Andy/GM 点头）时建议拆：

1. **BE-session/runtime：** `waiting_for_user` 态；权限类 tool_call 不预封口；startup 识别 vs seal interrupt 分流；`pending_permissions` 进 snapshot。
2. **BE-protocol：** 事件/字段；断线保留挂起；Deny 仅用户跳过；Cancel→Aborted + turn_finished 必达。
3. **FE（web/desktop）：** 消费 `pending_permissions`；`waiting_for_user`；断线不清权威列表误 Deny。

串行：协议字段名先定 → BE 落盘与 snapshot → FE 并行可在字段冻结后开工。

---

## 8. 本方案包验收（WP-179 done-when）

- [x] 本文档：as-is（file:line）+ 推荐语义 + 落盘论证 + 同型门闩 + 协议草案
- [x] FE 书面意见已写入「FE 对齐」并标终审
- [x] GM 批 plan（approve→ready）；**仍无 src 变更**
- [x] FE 终审契约段回执（前端中层 2026-10-16 同意，§5.2 已 amend）

**禁止：** 本包 deliver 夹带实现 PR / 改 src。
