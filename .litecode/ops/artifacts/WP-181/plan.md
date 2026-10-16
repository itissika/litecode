# WP-181 实现方案（backend_mid）— 落地 WP-179：waiting_for_user + 权限类落盘

**依据：** WP-179 done + §5.2 前端中层终审；Andy/GM 开实现。  
**兄弟包：** WP-182（frontend_mid）FE 类型/UI；行为验收以本包 snapshot 可用为准。  
**产物：** `.litecode/ops/artifacts/WP-181/`

---

## 1. 目标

落地 WP-179 已批语义（禁止范围外大重构）：

1. **对外态 `waiting_for_user`**：Wire / snapshot / 对外文档用此名；对内可继续 `TurnPhase::AwaitingPermission` 并扩展载荷（不必强行改 Rust 枚举名）。
2. **权限类 tool_call 物理写入且故意不封口**：`ask_user | permission | approval` 同属用户门闩；transcript 写 `item/tool_call`，暂不写 `item/tool_result`，直至用户回执或 Cancel→Aborted。
3. **startup 分流**：权限类未封口 → 恢复 `waiting_for_user`（**不** `seal_tail_unanswered_calls` interrupt）；非权限类未封口仍走现有 Incomplete / interrupt。
4. **`pending_permissions: PermissionRequest[]`** 进 snapshot/attach；absent = `[]`；与现 `agent/permission_request` **1:1**；`kind` **必填**。
5. **断线 ≠ Deny**：挂起保留；Deny **仅**用户点跳过。
6. **Cancel / Stop → `AskOutcome::Aborted`**（非 Deny），WS 仍连则 **`turn_finished` 必达**；等待态期间 turn **未 finished**。
7. **`awaiting_permission: bool`**：实现阶段改为派生（`pending_permissions.length > 0`）或逐步废弃；**出卡只认列表**。

---

## 2. 域拆分与串行

| 子包 | owner | 域 / 路径 | 内容 |
|------|-------|-----------|------|
| **WP-186**（WP-181-a） | `session` | `src/{session,client_protocol,serve}` | 协议字段 + 落盘 + startup 分流 + snapshot `pending_permissions` + `reject_all`→Aborted（断线保留）+ Cancel→Aborted + turn 等待态未 finished |
| **WP-187**（WP-181-b） | `runtime` | `src/{runtime,agent}` | `PhasePermissionSink` / observer phase 与 `waiting_for_user` 投影对齐；挂起进入/离开时 phase 切换；与 session 挂起列表一致 |

**串行理由（协议+落盘优先）：**

1. 先做 **181-a**：冻结 wire/snapshot 形状与落盘恢复，FE（WP-182）才能联调行为验收。
2. 再做 **181-b**：runtime 门闩 sink / phase 与 a 的挂起权威对齐；避免两包同时改 `permission_bridge` / connection 与 phase 投影。
3. **不**单立 tools 子包：`ask_user` / `authorize` 已按 `AskOutcome` 分支（Deny 文案 vs Aborted）；本包改动面在 bridge/connection/session。若实现中发现 `src/tools/ask_user.rs` 必须小改，由 181-a 经 tools handoff 或 backend_mid 代协调，**不**与 181-a 并行抢权限桥文件。

**FE WP-182：** 非子包；阶段 A（类型/mock hydrate）可与 181-a **并行**；阶段 B 行为验收 **串在** 本包 snapshot 可用之后。

---

## 3. 实现要点（按 WP-179 + FE §5.2）

### 3.1 协议 / snapshot（181-a）

- 增加 `pending_permissions`；元素字段：`request_id, turn_id, session_id, tool, rule_id, summary, kind, free_text?, options?, multi_select?, questions?`；`kind` 必填。
- 对外暴露 `waiting_for_user`（phase 或等价枚举档）；web/desktop **同一** snapshot 契约，不得另起字段名。
- 保留：`agent/permission_request` 入队、`agent/permission` 回执、`turn_started` / `turn_finished`、`AskKind`。
- **勿依赖**无 `request_id` 的 `permission_resolved` 关卡。

### 3.2 落盘与 startup（181-a）

- 权限类 call 写入 transcript，故意不封口；旁路或同表保存足够重挂卡的元数据（至少 request_id + kind + questions/options/summary/rule_id）。
- `seal_orphaned_in_progress` / `seal_tail_unanswered_calls`：**识别权限类** → restore wait；其余仍 seal。
- 进程重启后 session/turn ∈ `waiting_for_user`，snapshot 带出列表。

### 3.3 断线 / Cancel（181-a）

- 改 `reject_all_pending_asks`（及 disconnect 路径）：**断线不得一律 Deny**；挂起保留落盘/列表。
- 用户 Skip → Deny；用户 Stop/Cancel → Aborted → 清空挂起 → `turn_finished`（WS 仍连时必达）。
- 等待态：**不**发 `turn_finished`（与 FE「finished 清队列」一致）。

### 3.4 Runtime 对齐（181-b）

- `PhasePermissionSink` / `PermissionAwaiting`：进入挂起 → 权威态与 pending 列表一致；Allow/Deny/Aborted 离开时 phase 正确。
- 不改控制流语义超出 WP-179；不引入第二套 ask 通道。

---

## 4. 显式非目标

- **不做 H1 / Responses SSE codec** harden（归 WP-175 后续 Fix-H1，需实网 wire）。
- **不做** WP-183 Obs（panic hook / 工具审计 / wire 关联）— 与本包重型互斥排槽。
- **不改** FE `web/` / `desktop/`（归 WP-182）。
- 不把 WP-175 与 WP-176 绑成因果；本包吸收 WP-176 lock：**断线 ≠ 假 skipped**。

---

## 5. 重型与排槽

- 云电脑：`LC_MIN_MEM_GIB=2` `LC_KILL_MEM_GIB=1`；`CARGO_BUILD_JOBS` 按团队口径（profile/CI 常为 2；本机紧张时用 1）。
- **与 WP-183 重型互斥**；开跑前向 backend_mid 报槽。
- 优先目标测 / 协议测，避免无必要全量。

---

## 6. 验收（done-when）

- [ ] snapshot/attach 含 `pending_permissions`（与 `permission_request` 1:1，`kind` 必填）；absent = []
- [ ] 对外可见 `waiting_for_user`；等待态 turn **未** finished
- [ ] 权限类 tool_call 落盘未封口；startup 恢复等待而非 seal interrupt；非权限类仍 seal
- [ ] 断线保留挂起、**不**假 Deny/skipped；用户 Skip=Deny；Cancel=Aborted +（WS 连着）`turn_finished` 必达
- [ ] `awaiting_permission` 布尔派生或兼容，出卡权威=列表
- [ ] 相关单测 / 协议测绿；产物说明变更面与迁移
- [ ] **无** H1 codec 改动；**无**范围外大重构
- [ ] FE（WP-182）可用本包真实 snapshot 做行为验收（联调记录写入 artifacts）

---

## 7. 子包 brief 草稿路径

- `.litecode/ops/artifacts/WP-181/181-a-brief.md`（session）
- `.litecode/ops/artifacts/WP-181/181-b-brief.md`（runtime）

父包 `fill --kind split` 后 submit；子包保持 drafting，待父包 approve 后由域 owner fill→submit（approver=`backend_mid`）。
