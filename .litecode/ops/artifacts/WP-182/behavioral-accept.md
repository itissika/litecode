# WP-182 行为验收（契约层，非真机 UI）

**时间：** 2026-10-16 08:32 CST (Asia/Hong_Kong)  
**分支：** `ops/wp-186-session-pending-permissions`  
**HEAD：** `4590e84f62fccfb14ef3180ea2bac11857ea5289`  
**依据：** WP-181 frozen fixture + WP-186 wire；`artifacts/WP-182/web.md`  
**vitest：** `behavioral-vitest.txt` — **19/19 PASS**，exit 0  
**范围：** 纯重连 hydrate `pending_permissions` / `waiting_for_user` wire（本树够用）

---

## 路径核对（vs `web.md`）

| web.md 列出 | 实际路径 | 状态 |
|-------------|----------|------|
| types（SessionSnapshot.pending_permissions 等） | `web/src/api/types.ts` | OK：`pending_permissions?`、`waiting_for_user` / legacy `awaiting_permission`、`isWaitingForUserPhase` / `waitingForUserPayload` |
| turnStore | `web/src/stores/turnStore.ts` | OK：`toPendingPermission`；`applySnapshotMeter` 仅当键存在时整表覆盖 |
| connectionStore | `web/src/stores/connectionStore.ts` | OK：断线注释明确不发 deny，队列乐观保留 |
| SessionStatusDot | `web/src/components/SessionStatusDot.tsx` | OK：`isAwaitingPermission` + `deriveSessionStatus` 认 phase waiting |
| SessionItem | `web/src/components/SessionItem.tsx` | OK：列表圆点接派生 |
| AgentTab | `web/src/dockview/tabs/AgentTab.tsx`（非 `components/`） | OK：`deriveSessionStatus(..., { pendingPermissionCount })` |
| pendingPermissions.test | `web/src/stores/turnStore.pendingPermissions.test.ts` | OK：Phase A + Phase B |
| frozen fixture | `web/src/test/fixtures/pending_permissions.frozen.json` | OK：拷贝 WP-181；`_doc` 含 WP-186 说明 |

**结论：** 工作树产物与 `web.md` 一致；AgentTab 实际在 `dockview/tabs/`（web.md 简称未写全路径）。HEAD/分支与派活一致。未 stash pop / 未 commit / 未改 Rust。

---

## 契约对照（PASS / 证据）

### 1. `pending_permissions` 始终有键（WP-186 wire 约定；FE hydrate 认 present）

**PASS**

- 类型注释：`types.ts` ~549–556 — WP-186 always writes the key (including `[]`)；FE 仅在键 present 时覆盖。
- 测：`remount/hydrate with WP-186-shaped snapshot remounts cards + pending status`（显式带键重建队列）。
- 测：`frozen fixture still documents waiting_for_user + derived awaiting_permission`（fixture `_doc` + `pending_permissions` 长度 2）。
- 代码：`turnStore.ts` `applySnapshotMeter` ~1064–1105 — `hasOwnProperty('pending_permissions')` 才整表覆盖。

### 2. `waiting_for_user` phase

**PASS**

- 类型：`TurnPhase` 含 `{ waiting_for_user: … }`；legacy 读别名 `{ awaiting_permission: … }`（`types.ts` ~52–89）。
- 测：`isWaitingForUserPhase accepts waiting_for_user and legacy awaiting_permission`
- 测：`deriveSessionStatus: waiting_for_user phase → pending even if bool false`
- 测：`deriveSessionStatus: legacy awaiting_permission phase object → pending`
- 测：`remount/hydrate with WP-186-shaped snapshot…` — `turnPhase` 为 waiting，圆点 `pending`

### 3. 缺省键 vs `[]`

**PASS**

- 测：`absent pending_permissions key leaves live queue untouched`
- 测：`present pending_permissions: [] clears the live queue`
- 测：`explicit pending_permissions: [] clears queue; waiting_for_user gone → not pending`
- 代码：`applySnapshotMeter` present-key hydrate（含显式 `[]` 清空；缺省不动 live）

### 4. 断线不 deny

**PASS**

- 测：`disconnect path never sends agent/permission deny; queue kept until snapshot overwrite`
- 测：`Stop/cancel clears queue and sends agent/cancel only (no permission deny)`
- 测：`only grantPermission (Cancel/Deny/Allow) sends agent/permission`
- 代码：`connectionStore.ts` ~178–180 — 断线不发 deny；队列乐观保留，等下次 snapshot 覆盖。

### 5. 圆点派生（list dots trust BE `awaiting_permission` else pending length；phase waiting 优先）

**PASS**

- 测：`trusts explicit true / false; derives only when undefined`（`isAwaitingPermission`）
- 测：`deriveSessionStatus: waiting_for_user phase → pending even if bool false`
- 代码：`SessionStatusDot.tsx` — phase waiting → `pending`；否则信 BE bool；仅 `undefined` 时用 pending 长度。
- 接入：`SessionItem.tsx`、`dockview/tabs/AgentTab.tsx` 均走 `deriveSessionStatus` / `isAwaitingPermission`。

---

## vitest 摘要

| 项 | 值 |
|----|-----|
| 命令 | `npx vitest run src/stores/turnStore.pendingPermissions.test.ts --maxWorkers=1` |
| 结果 | **19 passed / 19**，1 file |
| exit | **0** |
| 日志 | `artifacts/WP-182/behavioral-vitest.txt` |
| tsc | **未跑**（中层未放行占槽） |

---

## 仍 defer（真机 / 端到端）

本树够用：纯重连 hydrate `pending_permissions` / `waiting_for_user` wire。以下仍 defer：

1. **真机 UI**：BE snapshot 带真实 pending 时重挂 PermissionCard —— 需 **cargo 槽重编** 后的 Session binary；**先别切 wp-187 / 不依赖 PhasePermissionSink**。
2. **本箱 `target/debug/litecode`**：mtime `2026-10-15 13:24 CST`，早于 WP-186 HEAD `4590e84`（`2026-10-16 08:22 +0800`），**不可当真机契约**。
3. 与 WP-186 合入后的进程恢复 / 冷启动续跑（周边，非本包契约测范围）。
4. E2E / 真机联调本身 —— 等中层要到 cargo 槽重编后再做。

---

## 是否 unblock 真机联调

**部分 unblock（契约层）：**

- **是** — vitest 契约层已绿，可 unblock 前端中层去要 **cargo 槽重编** Session binary。
- **否（本树 binary）** — 箱上 `target/debug/litecode` mtime 早于 WP-186，**不可当真机契约**；真机联调须等重编后再开。

---

## 禁区遵守

未改 Rust；未 commit；未 stash pop；未占桌面；未 npm ci；未跑 tsc。

---

## 真机空闲态（阶段1 · live wire）

**时间：** 2026-10-16 08:36 CST  
**Binary：** `/workspace/litecode/target/debug/litecode` @ `4590e84`，mtime `2026-10-16 08:34`（long-iso）— 已核对为 WP-186 重编后  
**Workspace：** `/tmp/lc-wp182-smoke`　**Bind：** `127.0.0.1:7483`  
**日志：** `artifacts/WP-182/live-smoke-idle.txt`

| 检查 | 结果 | 证据 |
|------|------|------|
| health | **PASS** | `{"ok":true,...}` |
| handshake `server/hello` | **PASS** | `ws-smoke.mjs handshake` exit 0 |
| `session/list` | **PASS** | `ws-smoke.mjs list` exit 0，`sessionListOk:true` |
| snapshot **恒有** `pending_permissions` 键 | **PASS** | `session/new` → `session/snapshot`；`hasOwnProperty` true |
| 空闲值 `[]` | **PASS** | `pending_permissions: []`，`pending_len:0` |
| turn / waiting_for_user | N/A | 新 session 无 turn（`turn: null`） |

**备注：** 起服需 `LITECODE_WEB_DIST`（箱上无 `web/dist`）；用 `/tmp/lc-wp182-web-dist` stub `index.html`，**未** npm build / npm ci / cargo / serve.sh。  
**session_id：** `01M5126ZR9GXDYF326AWWFKNHT`

---

## 非空重连（阶段2 · 旁表注入）

**时间：** 2026-10-16 08:36 CST  
**方式：** 停 serve → `session_pending_permissions` INSERT → 同 binary 再起 → `session/subscribe` 重连  
**日志：** `artifacts/WP-182/live-smoke-pending.txt`  
**注入：** **成功**（python3 sqlite3；表本已存在于 WP-186 schema）

| 检查 | 结果 | 证据 |
|------|------|------|
| 注入行可读 | **PASS** | row `(…, req_ask_user_e2e_001, ask_user, Choose deploy target)` |
| 重连 snapshot 有键 | **PASS** | `has_pending_permissions_key: true` |
| `pending_permissions.length ≥ 1` | **PASS** | `pending_len: 1` |
| `kind` 必填 | **PASS** | `"kind":"ask_user"` |
| 形状对照 frozen | **PASS** | 与 `web/src/test/fixtures/pending_permissions.frozen.json` 的 ask_user 条目字段一致（questions/q_env/options） |
| turn phase waiting | N/A | `turn: null`（旁表注入无 live turn；非卡点） |

**仍 defer（UI）：** Electron / PermissionCard 真机出卡未做（禁占桌面）；需中层另派。

---

## 真机联调清理

进程已停；`7483/7484` 已释放。临时探针 `/tmp/lc-wp182-probe.mjs`、stub dist `/tmp/lc-wp182-web-dist` 留在 /tmp，未进 git。
