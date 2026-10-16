# WP-187（WP-181-b）plan — PhasePermissionSink / phase 与 pending 对齐（runtime）

> **Status:** approved by backend_mid (Q1–Q6 defaults). Elevated from `plan-draft.md`.

**board id:** WP-187  
**parent:** WP-181（`.litecode/ops/artifacts/WP-181/plan.md` §3.4）  
**brief:** `.litecode/ops/artifacts/WP-181/181-b-brief.md`  
**兄弟:** WP-186（WP-181-a，session）已 **done**；FE WP-182  
**owner:** runtime · **approver:** backend_mid  
**产物目录:** `.litecode/ops/artifacts/WP-181/181-b/`  
**口径:** observe 控制流边界对齐；**不另起 ask 通道**；**不修 H1 codec**；**不改** permission_bridge / connection `reject_all`/`abort_all`/`retain_pending`（181-a 已落地）。  
**本文件：** approve 后定稿；实现见 `impl-notes.md` / `impl.diff`（cargo 待槽）。

---

## 0. 基线与分支（强制）

| 项 | 值 |
|----|-----|
| **实现基线 commit** | `4590e84` — `feat(WP-186): pending_permissions sidecar + waiting_for_user`（`ops/wp-186-session-pending-permissions`，**未 push**） |
| **实现分支（approve 后）** | 从 `4590e84` 切 `runtime/wp-187-phase-permission-sink`（或 `runtime/wp-187-…`，backend_mid 命名） |
| **fill 阶段** | **不**切分支、**不** pop stash、**不**改 `src/`；仅写本文件与后续 artifacts |
| **重型** | 与 session/FE cargo 互斥；`LC_MIN_MEM_GIB=2` `LC_KILL_MEM_GIB=1`；建议 `CARGO_BUILD_JOBS=1`；**开跑前报 backend_mid 占槽** |
| **合入** | 独立分支；**勿 push** `ops/team-framework` 除非 Andy 点头 |

---

## 1. 范围 / 非目标

### 1.1 范围（本包落地）

在 WP-186 已冻结 snapshot / `pending_permissions` / wire `waiting_for_user` / 断线≠Deny / Cancel→Aborted 之后：

1. **`PhasePermissionSink` 进出与 phase 权威对齐**  
   - 进入挂起 → L1 `TurnPhase::AwaitingPermission { tool, rule_id, summary }` + `PermissionAwaiting { awaiting: true }`  
   - 离开：Allow / Deny / Aborted 各自 phase 与 turn 边界正确（见 §3）
2. **与 session 挂起列表语义一致（热路径）**  
   - 等待期间：phase ∈ waiting；turn **未** finished  
   - Cancel→Aborted 后：离开 waiting，交由既有 Cancel 路径发 `turn_finished`（本包不重写 connection）
3. **observer / `TurnProgress` 布尔**与「是否在等用户」一致（进入 true / 离开 false）；出卡权威仍是 186 的 `pending_permissions` 列表，本包不另建权威源
4. **相关单测绿**；产物写入 `181-b/`

### 1.2 非目标（强制）

- **不改** `src/client_protocol/permission_bridge.rs`、`connection.rs` 的 `abort_all_pending_asks` / `retain_pending_on_disconnect` / `reject_all_*`（186 已分流）
- **不改** H1 / Responses SSE codec
- **不改** FE `web/` / `desktop/`
- **不另起**第二套 ask 通道 / 不改 `AskOutcome` 枚举语义
- **不抢** WP-188 Obs 已合代码面以外的无关 runtime 大改
- **冷恢复后续跑（181-a R1）**：本包默认 **热路径对齐优先**；冷恢复「用户回执后续跑整条工具栈」若需大改 agent 状态机，列为 handoff / 待确认（见 §7），不在本包默认范围硬做满

---

## 2. 现状 as-is（只读核实，基线 `4590e84` 树）

### 2.1 WP-186 已落地（本包依赖，勿重做）

| 位置 | 行为 |
|------|------|
| `protocol.rs:102-109` | `WireTurnPhase::AwaitingPermission` → serde **`waiting_for_user`**（alias `awaiting_permission`） |
| `protocol.rs:444` / project tests | `SessionSnapshot.pending_permissions`；absent/`[]`；形状测 `wp186_pending_permissions_tests` |
| `session/data/sqlite/pending.rs` + `session/pending.rs` | P1 旁表 `session_pending_permissions` CRUD |
| `manager.rs:269+` / `1558+` | resume 载入、upsert/remove/clear、`pending_permissions_snapshot` |
| `connection.rs:1559+` | `abort_all_pending_asks`（Aborted）、`retain_pending_on_disconnect`（不断 Deny） |
| `permission_bridge.rs:82-84` | 无 WS → **Aborted**（非假 Skip） |
| `controller/mod.rs:297,1063` | attach/snapshot：`awaiting_permission = !pending_permissions.is_empty()`（派生） |

### 2.2 Runtime sink / phase 缺口（本包要修）

| 位置 | 今日行为 | 与目标差距 |
|------|----------|------------|
| `runtime/phase.rs:32-51` | `ask`：先 `PermissionAwaiting{true}` → `inner.ask` → `PermissionAwaiting{false}`；**仅 Allow** 时 `PhaseChanged(ExecutingTools)` | **进入时不发** `PhaseChanged(AwaitingPermission{…})`；Deny/Aborted 离开时 **不**清 phase |
| `runtime/observer.rs:64-68` | `TurnPhase::AwaitingPermission { tool, rule_id, summary }` | 载荷无 `request_id`/`kind`（可接受：wire 列表权威在 session；L1 phase 保持三字段即可） |
| `runtime/observer.rs:225-227` | `PermissionAwaiting { awaiting: bool }` —「纯进度信号，不投影 wire」 | 与 phase 双轨；Manager 只翻布尔（`manager.rs:3525-3527`），**不**据此设 `progress.phase` |
| `runtime/mod.rs:964-972` | 包装链：`CancellingPermissionSink` → `PhasePermissionSink::wrap` | 挂接点正确；改 sink 主体即可 |
| `controller/mod.rs:913-924` | 收到 `PermissionAsk` 才把 `self.phase = AwaitingPermission{…}` | phase 进入依赖 **connection 发 PermissionAsk**，与 sink 不同步；若 Ask 晚于/缺事件，L1/L2 phase 漂移 |
| `connection.rs:1696` | bridge 入队后发 `PermissionAsk` | 186 职责；本包应让 **sink 进入即 PhaseChanged**，使 runtime 观察者不必只靠 Ask 事件 |
| `project.rs:400` | `PermissionAwaiting` → **不投影 wire** | 保持；wire phase 仍靠 `PhaseChanged` / snapshot |
| `permission/sinks.rs:106-118` | `CancellingPermissionSink`：cancel → Aborted | 本包依赖：Cancel 路径 sink 返回 Aborted；phase 离开须覆盖 Aborted |

**结论：** 186 已把列表/wire/断线权威放在 session+connection；runtime 仍用「布尔开关 +（仅 Allow）回 ExecutingTools」，**未**在 enter 时设 `AwaitingPermission`，Deny/Aborted 可能留下 **陈旧 phase=AwaitingPermission**（若曾由 PermissionAsk 设过）。这正是 181-b §3.4 要对齐的缝。

### 2.3 等待 / Cancel 边界（期望 vs 今日）

| 场景 | 期望（WP-179 / 181） | 今日 runtime 侧 |
|------|---------------------|-----------------|
| 进入挂起 | phase=waiting；turn **未** finished | `PermissionAwaiting=true`；phase 靠 PermissionAsk 间接设 |
| Allow | 离开 waiting → ExecutingTools（或后续工具环） | sink 已发 ExecutingTools ✅ |
| Deny（Skip） | 离开 waiting；工具得 Deny 文案；**不**因此 turn_finished（继续环或按工具结果） | `PermissionAwaiting=false`；**phase 可能仍 AwaitingPermission** ❌ |
| Cancel/Stop → Aborted | 离开 waiting；connection abort + `turn_finished`（WS 连） | CancellingSink→Aborted；布尔 false；**phase 未显式离开**；finished 归 connection/cancel_turn（勿抢） |
| 断线 | 列表保留；不假 Deny | 186 `retain_pending_*`；本包 **不**在 disconnect 路径改 bridge |

---

## 3. 设计

### 3.1 PhasePermissionSink 进出契约（核心）

在 `src/runtime/phase.rs` `impl PermissionSink for PhasePermissionSink::ask`：

```
enter:
  on_internal(PermissionAwaiting { awaiting: true })
  on_internal(PhaseChanged {
    phase: TurnPhase::AwaitingPermission {
      tool: prompt.tool.into(),
      rule_id: prompt.rule_id.into(),
      summary: prompt.summary.into(),
    },
    step: current_step,
  })

  reply = inner.ask(prompt, cancel)

leave:
  on_internal(PermissionAwaiting { awaiting: false })
  match reply.outcome:
    Allow { .. }  => PhaseChanged(ExecutingTools, step)   // 保持今日行为
    Deny          => PhaseChanged(ExecutingTools, step)   // 默认：回到工具执行态，交给工具环消费 Deny
    Aborted       => PhaseChanged(Cancelling, step)       // 默认：标取消中；turn_finished 仍由上层 Cancel 路径发出
```

**默认选择（待确认可改，见 §7）：**

- **D1** Deny → `ExecutingTools`（与 Allow 对称离开 waiting，避免卡在 AwaitingPermission）
- **D2** Aborted → `Cancelling`（不在 sink 内发 TurnCompleted / 不伪造 finished）
- **D3** 不扩展 `TurnPhase::AwaitingPermission` 加 `request_id`/`kind`（列表权威在 186；减少 observer/wire 扩散）
- **D4** 仍保留 `PermissionAwaiting` 布尔事件（Manager `TurnProgress` / lifecycle 已依赖）；与 PhaseChanged **成对**发出

### 3.2 与 pending 列表「一致」的含义（本包）

| 层 | 权威 | 本包职责 |
|----|------|----------|
| Wire snapshot `pending_permissions` | session 旁表 + manager（186） | **不写**旁表；不改 upsert/clear |
| Wire phase `waiting_for_user` | `PhaseChanged` → project `wire_phase`；controller PermissionAsk 亦可设 | **enter 必发** PhaseChanged(AwaitingPermission)，使无 Ask 竞态时 L1 仍正确 |
| `awaiting_permission` bool | snapshot 派生自列表长度（186）；热路径 progress 仍吃 PermissionAwaiting | 保证 sink 进出布尔正确；**不**在 runtime 维护第二份列表 |

**一致性验收（热路径）：** 一次 `sink.ask` 阻塞期间 → progress/controller phase ∈ AwaitingPermission 且 `awaiting_permission==true`；返回后布尔 false，且 phase ∉ AwaitingPermission（除非上层又进入下一次 ask）。

### 3.3 不碰的边界

- `permission_bridge.rs` / `connection.rs` 分流与旁表钩子 — **只读依赖**
- `CancellingPermissionSink` — 保持在 Phase 外层（`mod.rs:964-972`）；Cancel 仍先到 Cancelling 再 Phase
- H1 codec、Obs diagnostics 无关键改动（除非测需 import，禁止行为改）

### 3.4 冷恢复（限定）

181-a R1 handoff：进程重启后 oneshot/工具栈已死。

**本包默认：**

- **不**实现完整「冷恢复后续跑」状态机
- 若实现中发现 resume 后 phase 与 `pending_permissions` 不一致（列表非空但 phase Idle）：在 **runtime 可触达且不改 bridge** 的前提下，于 resume/observer 接缝做 **最小 phase 投影补丁**（或文档注明须由 session 在恢复时发 PhaseChanged）— **优先**要求 session 在 hydrate 时已带 waiting phase（186 snapshot）；runtime 仅保证 **下一次热 ask** 契约正确
- 完整续跑 → 记入 artifacts `notes.md` handoff，不阻塞本包 done_when（除非 approve 明确扩大范围）

---

## 4. 实现步骤（approve → start 之后，file-level）

1. **分支**  
   `git checkout -b runtime/wp-187-phase-permission-sink 4590e84`（名称可调；**等 backend_mid 放行再切共享树**）

2. **`src/runtime/phase.rs`**  
   - enter：`PhaseChanged(AwaitingPermission{ tool, rule_id, summary })` + 保留 `PermissionAwaiting{true}`  
   - leave：按 §3.1 三分支；抽小函数免重复  
   - 单元测（同文件 `#[cfg(test)]` 或 `phase` 测模）：mock `PermissionSink` + 记录 observer 事件序

3. **`src/runtime/observer.rs`**  
   - 仅当需要测辅助 / 注释澄清；**默认不改**枚举形状（D3）

4. **`src/runtime/mod.rs`**  
   - 确认 wrap 顺序不变；无逻辑改则不动（避免与其它 WIP 冲突）

5. **`src/session/manager.rs` `apply_event_to_progress`（可选最小）**  
   - 若测发现仅 PhaseChanged(AwaitingPermission) 时 `awaiting_permission` 仍 false：可在 `PhaseChanged` 分支对 `AwaitingPermission` 置 true、对离开 waiting 的 phase 置 false — **或**依赖成对 PermissionAwaiting（优先成对事件，少改 manager）  
   - **改前 ping backend_mid**：manager 属 session 域，仅当测证明必要且 186 同意

6. **不改** `permission_bridge.rs` / `connection.rs` / codec / web

7. **产物**  
   - `181-b/plan-draft.md`（本文件）→ approve 后可 `cp`/`mv` 为 `plan.md`  
   - 实现后：`impl.diff`、`acceptance.md`、必要时 `notes.md`（冷恢复 handoff）

---

## 5. 测试计划（目标测；不默认全量）

| # | 测 | 断言 |
|---|----|------|
| T1 | PhasePermissionSink enter | 事件序：`PermissionAwaiting{true}` 然后 `PhaseChanged(AwaitingPermission{ tool, rule_id, summary })`，再调用 inner |
| T2 | Allow leave | `PermissionAwaiting{false}` + `PhaseChanged(ExecutingTools)` |
| T3 | Deny leave | `PermissionAwaiting{false}` + phase **离开** AwaitingPermission（默认 ExecutingTools） |
| T4 | Aborted leave | `PermissionAwaiting{false}` + `PhaseChanged(Cancelling)`（或 approve 选定的离开相） |
| T5 | Cancel token | 经 CancellingPermissionSink 包装后仍得 Aborted，且 T4 事件成立 |
| T6 | 回归 | 现有 permission / ask_user 相关测不红；**不**跑全仓除非放行 |

重型命令示例（占槽后）：

```bash
LC_MIN_MEM_GIB=2 LC_KILL_MEM_GIB=1 CARGO_BUILD_JOBS=1 \
  cargo test -p litecode --lib phase:: -- --nocapture
# 若测落在其它模组，再加精确过滤；禁止无必要全量
```

---

## 6. 产物路径

```
.litecode/ops/artifacts/WP-181/181-b/
  plan-draft.md     # 本 fill（drafting）
  plan.md           # approve 后定稿（可由 draft 提升）
  impl.diff         # 实现后
  acceptance.md     # 测绿证据 + done_when 勾选
  notes.md          # 可选：冷恢复 / manager 触碰记录
```

父包对照：`.litecode/ops/artifacts/WP-181/plan.md` §3.4、§6（子包相关条）。  
181-a 对照：`.litecode/ops/artifacts/WP-181/181-a/plan.md` §2.6 / R1 / R6。

---

## 7. 冲突 / 协作 / 待确认

### 7.1 文件冲突

| 文件 | 本包 | 他人 |
|------|------|------|
| `runtime/phase.rs` | **主改** | 186 未改；勿与 Obs 无关改动缠在一起 |
| `runtime/mod.rs` | 尽量不改 | 188 曾挂 diagnostics；以 `4590e84` 为基，rebase 时留意 |
| `runtime/observer.rs` | 默认不改形状 | — |
| `session/manager.rs` | 仅当 T 证明必要且 mid 放行 | session 域 |
| `permission_bridge.rs` / `connection.rs` | **禁止改** | 186 权威 |

### 7.2 待确认默认（approve 时可改口）

| ID | 问题 | **fill 默认** |
|----|------|----------------|
| Q1 | Deny 离开相 | `ExecutingTools` |
| Q2 | Aborted 离开相 | `Cancelling`（不在 sink 发 TurnCompleted） |
| Q3 | 是否扩展 AwaitingPermission 载荷 | **否**（保持 tool/rule_id/summary） |
| Q4 | 冷恢复续跑 | **不做满**；notes handoff；热路径 done 即可 |
| Q5 | 是否改 `apply_event_to_progress` | **先不成**；成对 PermissionAwaiting；不够再 ping mid |
| Q6 | 分支名 | `runtime/wp-187-phase-permission-sink` |

---

## 8. done_when 清单

- [ ] 进入挂起：`PhasePermissionSink` 发出 `AwaitingPermission` phase + `PermissionAwaiting{true}`；与热路径 pending 语义一致（等待态 **未** finished）
- [ ] Allow / Deny / Aborted 离开：布尔 false；phase **不再**停留在 AwaitingPermission；Allow→ExecutingTools；Deny/Aborted 按 §3.1/Q1–Q2
- [ ] Cancel→Aborted 后 finished 仍由 186 connection/cancel 路径负责；本包不抢 `reject_all` / bridge
- [ ] 相关测（T1–T5）绿；无全仓无授权重型
- [ ] **无** H1 codec 改动
- [ ] **无** permission_bridge / connection 断线分流抢改
- [ ] 产物在 `.litecode/ops/artifacts/WP-181/181-b/`；实现基线 `4590e84`，分支自该 commit 切出
- [ ] deliver 后报 backend_mid 验收

---

## 9. 执行闸门（当前）

| 步骤 | 状态 |
|------|------|
| fill plan-draft | done |
| submit → backend_mid approve | **approved** (Q1–Q6) |
| Session 186 commit 落盘 | **`4590e84`** |
| 分支 `runtime/wp-187-phase-permission-sink` | **on branch** |
| 改 `phase.rs` | **done** (dirty; no commit) |
| cargo `phase::` | **pending slot** |
| deliver | pending after green tests |
