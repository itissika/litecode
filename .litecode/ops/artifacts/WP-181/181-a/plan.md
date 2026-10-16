# WP-186（WP-181-a）实施方案 — session：waiting_for_user / 权限类落盘 / pending_permissions / 断线≠Deny

**owner:** session · **approver:** backend_mid · **kind:** direct  
**parent:** WP-181（in_progress，GM 已批）· 兄弟 WP-187（runtime，串行在后）· FE WP-182  
**依据：** WP-179 §2–§5.2；WP-181 plan §3.1–§3.3；brief `.litecode/ops/artifacts/WP-181/181-a-brief.md`；冻结样例 `WP-181/fixtures/pending_permissions.frozen.json`  
**产物目录：** `.litecode/ops/artifacts/WP-181/181-a/`（本文件；实现后补 impl.diff / 测例说明 / 联调记录）  
**行号对照 HEAD：** `f4280ac`（`ops/team-framework`）。批准前不改 `src/`；不 cargo；合入/push 等 Andy。

---

## 0. 目标与边界

### 0.1 落地（本包）

1. **对外态 `waiting_for_user`**：Wire / snapshot / 对外文档用此名；对内可继续 `TurnPhase::AwaitingPermission`（不必强改 Rust 枚举名）。
2. **权限类 tool_call 故意不封口并落盘**：`ask_user | permission | approval`（`AskKind`）同属用户门闩；transcript 写 `item/tool_call`，暂不写 `item/tool_result`，直至用户回执或 Cancel→Aborted。
3. **startup 分流**：权限类未封口 → 恢复 `waiting_for_user`（**不** `seal_tail_unanswered_calls` interrupt）；非权限类未封口仍 seal Incomplete / interrupt。
4. **`pending_permissions: PermissionRequest[]`** 进 snapshot/attach；absent = `[]`；与现 `agent/permission_request` **1:1**；`kind` **必填**。
5. **断线 ≠ Deny**：挂起保留落盘/列表；Deny **仅**用户 Skip；Cancel/Stop → `AskOutcome::Aborted` +（WS 仍连）`turn_finished` 必达。
6. **等待态 turn 未 finished**；`awaiting_permission` 布尔改为派生（`pending_permissions.length > 0`）或兼容，**出卡只认列表**。

### 0.2 非目标（显式）

- 不改 H1 / Responses SSE codec（WP-175）。
- 不改 `web/` / `desktop/`（WP-182）。
- 不改 runtime `PhasePermissionSink` / observer phase 投影权威（WP-187；本包可预留接口/字段，不抢改 sink 进出）。
- 不与 WP-183 Obs 重型并行；开编前向 backend_mid 报槽。
- 不 commit / push；独立分支等 approve 后再切。

---

## 1. 现状 as-is（只读核实）

| 位置 | 行为 | 与目标的差距 |
|------|------|--------------|
| `permission_bridge.rs:49-94` | `WsPermissionBridge::ask`：`request_id` = 内存 UUID；无 WS → **Deny**；cancel token → **Aborted** | request_id / 载荷不落盘；无 WS 仍假 Deny（进程内无连接时） |
| `connection.rs:1649` | `pending_asks: HashMap<String, PendingAsk>` — 连接作用域 | 断线/Quit 丢挂起 |
| `connection.rs:1558-1570` | `reject_all_pending_asks` 一律 `AskOutcome::Deny` | 断线/Cancel 均假 skipped（WP-176 lock） |
| 调用点 `:1753` Quit、`:1764` Cancel、`:1869` request_rx None（断线）、`:1894/1910` deferred Quit/Cancel、`:1971/1976` dummy | 全部走 Deny | 须按路径分流：断线保留 vs Cancel→Aborted vs 用户 Skip→Deny |
| `protocol.rs:102-106` | `WireTurnPhase::AwaitingPermission { tool, rule_id, summary }` → serde `awaiting_permission` | 对外名应为 `waiting_for_user`；载荷缺 request_id/kind/questions |
| `protocol.rs:364` | `TurnSnapshot.awaiting_permission: bool` | 无挂起列表；出卡不能靠布尔 |
| `protocol.rs:368+` / `project.rs:1164-1216` | `SessionSnapshot` **无** `pending_permissions` | FE 重连丢卡 |
| `protocol.rs:512-532` | `PermissionRequest`；`kind` 有 `#[serde(default)]` | 事件路径 kind 可缺省；snapshot 元素 kind **必填**（冻结样例） |
| `writer.rs:290-407` / `sqlite/session.rs:2691-2753` | startup `seal_tail_unanswered_calls` 对尾部未答 call 补 interrupt 结果 | 权限类会被误封，与「继续等用户」相反 |
| `runtime/observer.rs:64` | `TurnPhase::AwaitingPermission` 无 request_id | 181-b 扩展；本包 wire/snapshot 先带齐列表 |
| `tools/ask_user.rs:287-296` | Deny→「User skipped…」；Aborted→ error | 语义已分；本包保证断线不再走 Deny |
| `grants.rs:41-46` | `AskKind::{Permission,Approval,AskUser}` snake_case | 与冻结样例 `permission\|approval\|ask_user` 一致 |

**结论：** 等待是连接内存 oneshot；startup 会把尾部未答 call 封成 interrupt；snapshot 无挂起列表 → 必须落盘 + 分流 seal + 协议字段。

---

## 2. 设计

### 2.1 协议 / snapshot

**A. `SessionSnapshot.pending_permissions`**

- 字段：`#[serde(default)] pending_permissions: Vec<PermissionRequest>`（缺省/absent = `[]`）。
- 元素与现 `agent/permission_request` 参数 **1:1**（见冻结样例）：  
  `request_id, turn_id, session_id, tool, rule_id, summary, kind, free_text?, options?, multi_select?, questions?`。
- **`kind` 必填**：snapshot 序列化路径不得依赖 `AskKind` 的 `#[serde(default)]` 缺省；构造/恢复时显式写入。可：(a) 保持同一 `PermissionRequest` 但保证写入方总带 kind；或 (b) snapshot 用新类型去掉 default——优先 (a) 少分裂，单测锁死「无 kind 不得进 snapshot」。
- 装配点：`project.rs` `build_session_snapshot`（约 :1164）及 manager 侧其它 `SessionSnapshot { ... }` 构造处（`manager.rs` 多处 `awaiting_permission: false`）；subscribe/attach 推 Snapshot 时带出当前 session 的挂起列表。
- 权威来源：session 级挂起表（见 §2.2），**不是**连接 `pending_asks`  alone。

**B. 对外 `waiting_for_user`**

- Wire phase：给 `WireTurnPhase::AwaitingPermission` 加 `#[serde(rename = "waiting_for_user")]`（对内 Rust 名可不动），使 wire 字面为 `waiting_for_user`。  
  - 兼容：若现有客户端仍认 `awaiting_permission`，实现时用 `alias` 或双读（读 `waiting_for_user` | `awaiting_permission`）；写只出 `waiting_for_user`。批前与 FE（WP-182）确认：FE 终审已要新名，优先只写新名。
- `TurnSnapshot.awaiting_permission`：实现为派生 `!pending_permissions.is_empty()`（或 phase==waiting），保留字段一版以免旧客户端崩；**出卡只认列表**（文档+测）。
- session 级可选顶层布尔不新增（冻结样例里的 `waiting_for_user: true` 是说明性字段；权威 = phase + 列表）。若 FE 需要顶层布尔，与 WP-182 对齐后加 `#[serde(default)]` 派生字段——默认不加，避免双源。

**C. 事件保留**

| 事件 | 本包约束 |
|------|----------|
| `agent/permission_request` | 保留；新建挂起时发；载荷与 snapshot 元素同形 |
| `agent/permission` 回执 | 保留；用户回答唯一路径（Allow/Deny/选线） |
| `turn_started` / `turn_finished` | 保留；**等待态不得发** `turn_finished`；Cancel→Aborted 后清空挂起再发 finished（WS 连着必达） |
| `permission_resolved` | 无 request_id，**勿依赖**关卡 |

### 2.2 落盘形状（权限类故意不封口）

**原则：** transcript 上权限类 `item/tool_call` 正常写入（与其它 call 同形），**暂不写** `item/tool_result`。旁路元数据足够 FE 重挂卡与 startup 恢复。

**推荐最小旁路（优先选一，批时确认）：**

- **方案 P1（推荐）：** 同 DB 旁表或 session 元数据 JSON（如 `pending_permissions` blob / 表 `session_pending_permissions`），按 `session_id` 存 `PermissionRequest` 列表（含 `request_id` 稳定化）。tool_call 的 `call_id` 与 `request_id` 关联字段写入元数据。  
  - 优点：startup 不靠猜 tool name；kind/questions 完整。  
  - 缺点：多一张表/一列。
- **方案 P2：** 仅靠 tool_call 尾 + name∈{`ask_user`, …} 识别权限类，questions 从 call arguments 反序列化；`request_id` 重启后重新生成。  
  - **弃选主路径：** FE 重挂需要稳定 `request_id` 才能对上回执；重生 id 会让旧卡回执对不上。若用 P2 必须把原 request_id 写入 call 旁路字段。

**本包采用 P1：** 写入挂起时 upsert 元数据；用户回执/Cancel 封口后删除对应行；startup 读元数据恢复列表与 phase。

**权限类判定（写入与 seal 分流共用）：**

- `AskKind` ∈ {Permission, Approval, AskUser} 的挂起；  
- 落盘侧：元数据有记录 **或**（兜底）tool name == `ask_user` / 授权路径写入的 permission/approval 标记。  
- 非权限类（bash/grep 等执行中崩）：无元数据 → 仍走现有 seal。

**`request_id` 稳定化：**

- 今日 `permission_bridge.rs:69` 每次 `Uuid::new_v4()`。改为：创建挂起时生成一次 → 写入旁路 → 发 `permission_request` → 入连接 `pending_asks`；进程存活期间不变；重启后从旁路恢复同一 id。

### 2.3 startup 分流

改动点：

1. `writer.rs:290` 调用的 `seal_tail_unanswered_calls` / `sqlite/session.rs:2691`：在补 interrupt 之前，对每个尾部未答 call：  
   - 若属权限类（旁路命中）→ **跳过 seal**，收集为「待恢复等待」。  
   - 否则 → 现有 `unanswered_call_output` interrupt 封口。
2. `seal_orphaned_in_progress`（`writer.rs:328`）：权限类 call 若被标 `in_progress`，策略：  
   - 优先：权限等待期间 tool_call 行应为已落盘的 final/普通 item（与今日「call 先写、等 sink」一致），不应长期 `in_progress`；核实 runtime 写时序后，仅对非权限 orphan 做 Incomplete seal。  
   - 若核实发现权限等待时 call 仍 `in_progress`：seal_orphaned 对权限类改为「标 final 但不补 result」，并登记旁路。实现前用测锁定。
3. **恢复时机：** Session resume / subscribe / 首 snapshot：读旁路 → 填 `pending_permissions` → phase=`waiting_for_user` → `awaiting_permission=true`（派生）。  
   - **不**自动向模型续跑；等用户 `agent/permission` 或 Cancel。  
   - 重连后：若无活 oneshot，需 **重建等待**：bridge 再次 `ask` 或 session 层提供「已恢复挂起、只等回执」路径（oneshot 接回 runtime/工具栈）。  
   - **关键风险：** 进程重启后原工具栈/oneshot 已死。必须有「冷恢复」：  
     - 要么：startup 不立刻恢复执行，只把 session 标为 waiting + 列表；用户回执时由 session/runtime **注入** tool_result 并决定是否续跑（可能需 181-b 配合）。  
     - 要么：resume 时重新进入 turn 的 permission wait 点（重）。  
   - **本包推荐：** 冷恢复 = 列表 + phase + 不 seal；用户回执路径写 tool_result 封口并（若 turn 记录仍 open）通知 runtime 继续；若 runtime 无法续跑，至少保证：**不假 Deny、UI 可答、Cancel 可 Aborted 清场**。与 backend_mid / WP-187 在 approve 时确认续跑边界；本包测锁定「列表可重挂 + 回执封口」，续跑若属 181-b 则 handoff 注明。

### 2.4 断线 / Cancel / Deny 分流

改 `reject_all_pending_asks`（及调用方），**禁止**一律 Deny：

| 路径 | 语义 | 动作 |
|------|------|------|
| WS 断线 / `request_rx` None / 连接 Drop | **保留挂起** | **不** send Deny；不清旁路；`pending_asks` 可清（oneshot 已无接收方可选择 drop 而不 resolve，或显式「park」）；snapshot 仍有列表 |
| `SessionRequest::Quit`（进程退） | 同断线：保留落盘 | 不假 Deny；旁路保留供下次启动恢复 |
| `SessionRequest::Cancel`（用户 Stop） | **Aborted** | 对每个 pending：`AskOutcome::Aborted`；旁路清除；写 Aborted 类 tool_result（或现有 ask_user Aborted 文案）；发 `turn_finished`（WS 仍连必达） |
| 用户 Skip（`agent/permission` approved=false 且非 cancel） | **Deny** | 现有 `apply_permission_grant` → Deny 文案；封口；清该条旁路 |
| 用户 Allow | 现有 Allow / Always | 封口；清旁路 |

实现要点：

- 将 `reject_all_pending_asks` 拆成例如 `abort_all_pending_asks`（Aborted）与 `retain_pending_on_disconnect`（只 drain 连接 map、不 resolve 为 Deny；或 resolve 为「连接丢失」但不写 Deny 文案——**不得**让 ask_user 出现「User skipped」）。  
- bridge 无 WS 时（`permission_bridge.rs:82-83`）今日 Deny：改为与「无客户端」策略一致——若已落盘等待，应 park 而非 Deny；无落盘的瞬时无 WS 仍可 Deny/Aborted（批时确认：推荐 **Aborted 或 park**，避免假 skipped）。本包最小改：无 WS → 与断线一致 **不**假 Skip 文案（优先 Aborted 或显式 park error，文案区分「no websocket」≠「user skipped」）。
- 现有单测 `reject_all_pending_asks` 期望 Deny（`connection.rs:2198+`、Cancel 测 `:2440`）→ 按新语义改断言。

### 2.5 等待态 turn 未 finished

- 进入挂起：**不**发 `turn_finished`；turn 保持 live / phase=`waiting_for_user`。  
- 离开：Allow/Deny 封口后回到 ExecutingTools/后续；Cancel→Aborted 后发 `turn_finished` 并清空挂起。  
- 与 FE「running 出卡、finished 清队列」一致（WP-179 §5.2.3）。  
- 核查今日 Cancel 路径是否在 reject Deny 之后才 cancel_turn；改为 Aborted 后仍走 cancel_turn，保证 finished 帧。

### 2.6 与 WP-187 / WP-182 边界

| 包 | 本包做 | 留给对方 |
|----|--------|----------|
| WP-187 runtime | 列表/phase wire 名/等待未 finished 的协议事实 | `PhasePermissionSink` 进出与 pending 权威对齐；冷恢复后续跑 |
| WP-182 FE | 冻结样例已给；snapshot 字段可用后通知联调 | 类型/UI hydrate；行为验收串在本包 snapshot 可用后 |

---

## 3. 改动文件清单（预计）

| 文件 | 变更 |
|------|------|
| `src/client_protocol/protocol.rs` | `SessionSnapshot.pending_permissions`；`WireTurnPhase` serde rename→`waiting_for_user`；文档注释 |
| `src/client_protocol/project.rs` | snapshot 装配填列表；`awaiting_permission` 派生；phase 映射 |
| `src/client_protocol/connection.rs` | 断线/Cancel/Quit 分流；Cancel→Aborted；测例更新；挂起写入旁路钩子 |
| `src/client_protocol/permission_bridge.rs` | request_id 稳定；无 WS 不假 Skip；与旁路协作 |
| `src/session/data/writer.rs` | startup 调用分流（若逻辑上提） |
| `src/session/data/sqlite/session.rs`（或新 `pending.rs`） | 旁路 CRUD；`seal_tail_unanswered_calls` 跳过权限类 |
| `src/session/manager.rs` | resume/snapshot 恢复 waiting；挂起 API |
| `src/session/live.rs` | 视需要扩展 awaiting 语义（派生） |
| 测试 | `connection` 权限/断线/Cancel；`session/data` seal 分流；协议 snapshot 形状 vs 冻结样例；必要时 `ws_*` 协议测 |
| 产物 | `181-a/plan.md`（本文件）；实现后 `impl.diff`、`notes.md`、联调记录 |

**不改：** `web/`、`desktop/`、H1 codec、`runtime/phase.rs` 主体（181-b）。

若实现中发现 `src/tools/ask_user.rs` 必须小改（文案/无 sink），经 tools handoff 或 backend_mid 协调，**不**并行抢权限桥以外的 tools 大改。

---

## 4. 测试计划（目标测 / 协议测；不默认全量）

1. **snapshot 形状：** `pending_permissions` 与 `fixtures/pending_permissions.frozen.json` 字段一致；`kind` 必填；absent=`[]`；`waiting_for_user` wire 名。  
2. **权限类不预封口：** 挂起中 disk 有 tool_call、无 tool_result；旁路有 request。  
3. **startup 分流：** 权限类未封口 → 恢复列表+phase，**无** interrupt 结果行；普通 bash 未封口 → 仍 seal interrupt（现有 `final_immutability_tests` 保持绿或按分流改权限夹具）。  
4. **断线：** 断线后旁路仍在；再 attach snapshot 仍有卡；**无**「User skipped」类 Deny 文案。  
5. **用户 Skip：** Deny + 封口 + 列表清空该条。  
6. **Cancel：** Aborted + 列表空 +（WS 连）`turn_finished` 必达；等待态期间无 finished。  
7. **派生布尔：** `awaiting_permission == (pending_permissions.len()>0)`。  
8. **回归：** 现有 permission grant / ask 不阻塞其它 RPC 等测例更新后绿。

重型：`LC_MIN_MEM_GIB=2` `LC_KILL_MEM_GIB=1`；`CARGO_BUILD_JOBS` 按团队（紧张用 1）；与 WP-183/188 **互斥**，开跑前报 backend_mid 排槽。只跑本包相关 target，不擅自全仓。

---

## 5. 验收（done-when）对照

- [ ] snapshot/attach 含 `pending_permissions`（与 permission_request 1:1，`kind` 必填）；absent=`[]`
- [ ] 对外可见 `waiting_for_user`；等待态 turn **未** finished
- [ ] 权限类 tool_call 落盘未封口；startup 恢复等待而非 seal interrupt；非权限类仍 seal
- [ ] 断线保留挂起、不假 Deny/skipped；用户 Skip=Deny；Cancel=Aborted +（WS 连着）`turn_finished` 必达
- [ ] `awaiting_permission` 派生或兼容；出卡权威=列表
- [ ] 相关单测/协议测绿；产物说明变更面与迁移
- [ ] **无** H1 codec；**无**范围外大重构；**无** web/desktop
- [ ] 通知 backend_mid / FE：snapshot 可联调（WP-182 阶段 B）

---

## 6. 风险与开放问题（approve 时拍板）

| ID | 风险 | 建议 |
|----|------|------|
| R1 | 冷恢复后 runtime/工具栈已死，用户回执无法续跑 | 本包保证列表+封口+不假 Deny；续跑与 181-b 对齐；批注 handoff |
| R2 | Wire 改名 `waiting_for_user` 打破旧客户端 | 只写新名（FE 终审）；必要时读 alias 一版 |
| R3 | `PermissionRequest.kind` 仍 default | snapshot 构造强制 kind；加测 |
| R4 | 旁表迁移/旧 DB | 无旁路时 absent=[]，行为≈今日；有尾部 ask_user 未封口则按 name 兜底恢复或保守 seal——批时选「name 兜底恢复」以免丢卡 |
| R5 | 无 WS 瞬间 ask | 禁止假 Skip；Aborted 或 park |
| R6 | 与 WP-187 文件冲突 | 本包不改 `runtime/phase.rs` sink 主体；permission_bridge/connection 以本包为准 |

---

## 7. 执行顺序（approve → start 之后）

1. 独立分支（等合入指令；不 push `ops/team-framework`）。  
2. 旁路 CRUD + seal 分流 + 协议字段（可先测 seal/snapshot 形状）。  
3. connection 断线/Cancel/Deny 分流 + 测例。  
4. resume/attach 恢复列表；与 bridge request_id 稳定化。  
5. 目标测绿 → 报槽后按需加测 → deliver；产物写入 `181-a/`。  
6. 通知 backend_mid；FE 可联调 snapshot。

**当前状态：** 仅 fill 方案，**不** start、**不**实现、**不** cargo。
