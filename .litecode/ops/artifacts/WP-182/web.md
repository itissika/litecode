# WP-184（web / Phase A）— pending_permissions hydrate

**父包：** WP-182　**契约：** WP-179 §5.2 / WP-186　**状态：** Phase A 已验收绿；Phase B 类型+契约测已写，**等重型槽**后 verify / deliver  
**禁止已遵守：** 未改 Rust / desktop；未 commit；未跑 tsc/vitest（本轮无槽）。

## 改动文件

| 文件 | 要点 |
|------|------|
| `web/src/api/types.ts` | `SessionSnapshot.pending_permissions?: PermissionRequest[]`；`kind` 注释标明 snapshot 必填 |
| `web/src/stores/turnStore.ts` | `toPendingPermission`；`applySnapshotMeter` **仅当键存在**时整表覆盖（含显式 `[]`）；键缺省不动 live；Stop 只发 `agent/cancel` |
| `web/src/stores/connectionStore.ts` | 断线不发 deny；队列乐观保留 |
| `web/src/components/SessionStatusDot.tsx` | `isAwaitingPermission`：信 BE；仅 `undefined` 时用 pending 长度派生 |
| `web/src/components/SessionItem.tsx` / `AgentTab.tsx` | 列表/Tab 圆点接派生 |
| `web/src/test/fixtures/pending_permissions.frozen.json` | 拷贝 WP-181 frozen（不另造形状） |
| `web/src/stores/turnStore.pendingPermissions.test.ts` | Phase A vitest |

出卡仍只认 `permissionQueue[0]`（既有 PermissionCard）。

## hydrate 语义（键缺省 vs 显式空）

| snapshot | 行为 |
|----------|------|
| **无** `pending_permissions` 键 | **不动** live `permissionQueue`（兼容旧 BE / 预 WP-181） |
| **有** `pending_permissions: [...]` | 整表覆盖（与 `onPermissionRequest` 同形 normalize） |
| **有** `pending_permissions: []` | 清空队列 |

WP-181 约定合入后**始终显式发送** `[]`（空闲时），不再依赖「缺省=空」。

## 行为对照

1. 类型 1:1 permission_request；fixture 锁定 kind 必填路径  
2. hydrate 见上表；live 仍按 request_id upsert  
3. 出卡仅 queue[0]  
4. 断线不发 deny；有键的 snapshot 覆盖  
5. 圆点：true→pending；false→不派生；undefined→pendingCount>0  
6. turn_started / turn_finished / Stop 清队列保留  
7. frozen：同 session/turn 的 ask_user（两题）+ permission（bash）

## 测试

`turnStore.pendingPermissions.test.ts`：形状、重建、**键缺省保留 live**、**显式 [] 清空**、upsert、queue[0]、断线、turn 边界、Stop、grant、圆点派生。

## Phase B（WP-186 SNAPSHOT_READY，2026-10-16 HKT）

**状态：** 类型 + 契约测已写；**重型未跑**（无槽）；**未 deliver**；**未 commit**。  
**依据：** `artifacts/WP-186/{progress.md,impl.diff}` — `pending_permissions` 始终写出（含 `[]`）；`WireTurnPhase::AwaitingPermission` 序列化为 `waiting_for_user`，读别名 `awaiting_permission`；`TurnSnapshot.awaiting_permission` **布尔字段名不变**。

### Wire 映射

| BE | FE |
|----|-----|
| `WireTurnPhase::AwaitingPermission` → JSON key `waiting_for_user` | `TurnPhase` 增加 `{ waiting_for_user: { tool, rule_id, summary } }` |
| 读别名 `awaiting_permission`（旧客户端） | 保留 `{ awaiting_permission: … }` 作读兼容 |
| `TurnSnapshot.awaiting_permission: bool`（由 list 长度派生） | 字段名不变；`SessionStatusDot` / `SessionItem` 仍信此 bool |
| `SessionSnapshot.pending_permissions` 始终有键（含 `[]`） | Phase A hydrate：键 present 才整表覆盖 |

辅助：`isWaitingForUserPhase` / `waitingForUserPayload`（`web/src/api/types.ts`）。圆点：phase 为 waiting 对象 → `pending`，否则走既有 bool / 队列派生。

### 本阶段改动文件

| 文件 | 要点 |
|------|------|
| `web/src/api/types.ts` | `waiting_for_user` 正式 wire；legacy 读别名；helpers |
| `web/src/components/SessionStatusDot.tsx` | `deriveSessionStatus` 认 phase waiting |
| `web/src/components/SessionItem.tsx` | 同上 |
| `web/src/stores/turnStore.pendingPermissions.test.ts` | Phase B 契约测（未跑） |
| `web/src/test/fixtures/pending_permissions.frozen.json` | `_doc` 补 WP-186 说明（形状未另造） |

未改：`connectionStore` / hydrate 键语义（Phase A 已定）；Rust `src/`；desktop。

### Phase B 测试（未跑，等重型槽）

`turnStore.pendingPermissions.test.ts` 新增 describe「Phase B」：

1. `isWaitingForUserPhase` / `waitingForUserPayload`：canonical + legacy  
2. remount：`applySnapshotTurn` + `applySnapshotMeter` 用 frozen pending + `waiting_for_user` phase → 队列重建、turnPhase、圆点 `pending`  
3. 显式 `[]` 清空；idle + `awaiting_permission:false` → 非 pending  
4. phase=`waiting_for_user` 即使 bool=false → pending  
5. legacy phase 对象 → pending  
6. fixture 文档字段 `waiting_for_user` / derived 说明仍在  

命令（槽放行后）：`cd web && npx tsc -b`；`npx vitest run src/stores/turnStore.pendingPermissions.test.ts --maxWorkers=1`。

### 仍 defer（需 Session 分支 live binary / 端到端）

- 真机：BE snapshot 带真实 pending 时 UI 重挂卡  
- 与 WP-186 合入后的进程恢复 / 冷启动续跑（属 WP-187 周边）  
- cargo 绿编译的 Session 二进制联调  

本包契约测覆盖形状与 FE 行为；**不做 E2E**，等 前端中层 排槽重型通过后再 deliver。

## 非目标

改 Rust；desktop 第二套字段；依赖 permission_resolved；commit / push。

## 恢复说明（2026-10-16 HKT）

云仓曾短暂落到 Session 分支 `ops/wp-186-session-pending-permissions`，WP-184 web 改动只在 `stash@{*}`（`wip-other-before-wp188`）里。已切回 `ops/team-framework`，用 `git checkout 'stash@{…}' --` **仅恢复** 6 个 web 路径 + 从 stash 未跟踪树恢复测试与 fixture（拷贝自 `artifacts/WP-181/fixtures/pending_permissions.frozen.json`）。**未** `stash pop`，**未**改 `src/` Rust；Session 的 Rust WIP 另 park 在独立 stash `session-wp186-rust-wip-park-for-wp184`。

`web/node_modules` 当时缺失，为跑重型先执行了 `npm ci`（日志 `rerun-npm-ci.txt`）。测试侧小修：`sendRpc` mock 补 `_method: string`（TS2493）；fixture 改为 `__dirname`+`join` 加载（vitest 下 `import.meta.url`/`fileURLToPath` 报 scheme 错）。

## 重型结果（present-key hydrate 已确认）

| 检查 | 命令 | 结果 | 日志 |
|------|------|------|------|
| hydrate | `applySnapshotMeter` + `hasOwnProperty('pending_permissions')` | 仍在 `turnStore.ts` ~1060–1105 | — |
| tsc | `web && ./node_modules/.bin/tsc -b` | **PASS** exit 0 | `artifacts/WP-182/rerun-tsc.txt` |
| vitest | `vitest run src/stores/turnStore.pendingPermissions.test.ts --maxWorkers=1` | **PASS** 13/13 | `artifacts/WP-182/rerun-vitest-pendingPermissions.txt` |

MemAvailable 全程 ≥6GiB（门槛 2GiB）。未 commit。
