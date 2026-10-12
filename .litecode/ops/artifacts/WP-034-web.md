# WP-034-web（WP-037）— 权限队列 + 断线 reject pendingRpc

父包：WP-034（frontend_mid） / 祖包：WP-033  
执行：frontend（前端-web）  
范围：仅 `web/`。不 commit。未碰 desktop / 引擎 UI / symlink。

## 对齐 WP-033 已定口径

| 口径 | 本包落点 |
|------|----------|
| #1 UI 单卡；底层按真实待批列表 FIFO 挨个批复 | `permissionQueue` + UI 只渲 `queue[0]` + `key=request_id` |
| #2 短线重连不恢复孤儿 ask | 未改 snapshot 回灌；`sessionStore.applySnapshot` 仍无 pending ask；**未**加 `clearPermissionQueue` |
| #6 断线/换代立刻 reject pending，不干等 30s、不自动重放 | `agentWs.rejectAllPendingRpc` 挂在 onclose / connect 换代 / disconnect |

## Step 1 — 权限 FIFO + 单卡 + 禁复活

### 改动
- `web/src/stores/turnStore.ts`
  - `pendingPermission: PendingPermission \| null` → `permissionQueue: PendingPermission[]`
  - 导出 `headPermission(slice)`（队头选择器）
  - `onPermissionRequest`：按 `request_id` 入队（同 id 原地更新，不覆盖整槽）
  - `grantPermission`：只打队头；成功按 id 出队；失败**去掉** `\|\| !latest` 回插（W6）
  - `cancel` / `onTranscriptReverted` / `onTurnStarted` / `onTurnFinished`：`permissionQueue: []`
- `web/src/dockview/panels/AgentPanel.tsx`
  - 选择器改为 `permissionQueue[0]`
  - `<PermissionCard key={pendingPermission.request_id} …>`
- 测试夹具同步：`turnStore.test.ts`、`ComposerDock.test.tsx`、`MessageList.test.tsx`

### 回归测（新增）
- FIFO 双入队 → grant 只出队头
- W6：cancel / transcript revert 后 grant 失败不回插
- 同 `request_id` 不双入队
- PermissionCard `key` remount 清空旧勾选（W5）

### 未做
- 未引入 `dismissedRequestIds`（最小改动：去 `\|\| !latest` 即可）
- 未加 `clearPermissionQueue` API

## Step 2 — 断线 reject pendingRpc

### 改动
- `web/src/api/agentWs.ts`
  - 私有 `rejectAllPendingRpc(reason)`：拷贝 values → clear Map → 逐个 `reject`（闭包内 `clearTimeout`）
  - 调用点：`connect()` 换代后、`ws.onclose`（本代）、`disconnect()` 换代后
  - **不**自动重放任何 RPC
- `connectionStore`：仅透传 `sendJsonRpc`，无重试/重放队列；本包未改
- 断线清卡策略：**最小改动** — 不回灌 + reject；**未**加 `clearPermissionQueue`。cancel/revert 仍整队清空；纯断线时队头卡可仍显示，但在途 grant 的 catch 不再把已 dismissed 的卡复活（Step1 W6）

### 回归测（新增，`agentWs.test.ts`）
- socket close / disconnect / connect 换代 → 在途 RPC 立刻 `WebSocket closed`
- close 后再推进 31s 不会二次 timeout reject

## 测跑

环境：Node v22.14.0（箱内系统 Node 20 无法跑 vitest 5/jsdom/undici）

```
cd web && vitest run \
  src/api/agentWs.test.ts \
  src/stores/turnStore.test.ts \
  src/components/PermissionModal.test.tsx \
  src/dockview/panels/ComposerDock.test.tsx \
  src/components/MessageList.test.tsx \
  src/dockview/panels/AgentPanel.test.tsx
```

结果：**6 files / 126 tests passed**（Step1 绿后进 Step2，合并复跑全绿）

附注：为跑测执行了 `npm install`，`web/package-lock.json` 可能有 diff；与本包逻辑无关，未刻意改依赖声明。

## clearPermissionQueue？

**否。** 未新增该 API。断线路径依赖 reject pendingRpc + 不 snapshot 回灌孤儿 ask。
