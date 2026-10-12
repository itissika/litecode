# WP-035 desktop 实施记录（WP-041）

- 包：WP-041（owner frontend_desktop，审批 frontend_mid）
- 分支：`ops/team-framework`
- 时间：2026-10-12（Asia/Hong_Kong）
- 口径：WP-033 #4+#7；对症 WP-021 D1/D2/D4/D5/D6/D7 + D8
- **未 commit**；**未**跑 `wp.mjs deliver`（由中层验收后交付）

## 改动文件

### Step 1 — 自动恢复 + 单飞
| 文件 | 作用 |
|---|---|
| `desktop/src/lifecycle-mutex.ts` (+test) | `LifecycleMutex` / `SingleFlight` |
| `desktop/src/process-recovery.ts` (+test) | 退避策略与计数（隧道/sidecar） |
| `desktop/src/remote-state.ts` (+test) | D8：`clearManagedRemoteIfSession` |
| `desktop/src/ssh-session.ts` (+test) | 隧道 `ServerAlive*` / `ExitOnForwardFailure`；`intentionalStop`；exit/error 监听；`buildTunnelArgs` |
| `desktop/src/sidecar.ts` (+test) | `intentionalStop`；`watchSidecarExit`；`stopSidecar` 先打主动停标记 |
| `desktop/src/main.ts` | 挂隧道/sidecar 恢复；Open Remote 单飞；sidecar 生命周期串行；D8 成功后再 bind / 失败清句柄；`litecode:remote-disconnected` |
| `desktop/src/preload.ts` | `onRemoteDisconnected` |

### Step 2 — 下载体验
| 文件 | 作用 |
|---|---|
| `desktop/src/components.ts` (+test) | 状态/百分比/网速；pause/stop/retry/abortAll；Range 续传；启动清扫 `.tmp` |
| `desktop/src/main.ts` | IPC `component-pause/stop/retry`；`before-quit` → `abortAll` |
| `desktop/src/preload.ts` | `pauseComponent` / `stopComponent` / `retryComponent`；进度事件扩展字段 |
| `desktop/package.json` | 测试列表纳入新 `*.test.js` |

未改：`web/`、workflows、桌面 CI、引擎 UI、symlink。

## Step 1 行为摘要（D1/D2/D4/D5/D8）

1. **隧道（D1）**：`startTunnel` 固定加 `ServerAliveInterval=15`、`ServerAliveCountMax=3`、`ExitOnForwardFailure=yes`。非主动退出 → 使 `managedRemote` 失效、发 `remote-disconnected`、限次退避后走既有 `reconnectRemote`。`stopManagedRemote` / `stopTunnel` 先 `markIntentionalStop`，Disconnect 不重连。
2. **sidecar（D2）**：READY 后 `watchSidecarExit`；非主动退出 → 限次自动重启并重载本机 workbench；耗尽则弹错误框。`stopSidecar` 先打主动停。
3. **单飞（D4/D5）**：`startRemoteSession` / `completeRemoteSession` 各用 `SingleFlight`（重入拒绝；complete 对已 `pending.ready` 幂等）。`relaunchSidecar` / `connectRemote` / return-to-hub / before-quit 走 `LifecycleMutex`，避免双 stop 后双 start 留孤儿。
4. **D8**：`connectManagedSsh` 在 `connectRemote` **成功之后**才 `bindManagedRemote`；失败 catch 用 `clearManagedRemoteIfSession`。`enterRemoteWorkbench` 失败同样清句柄。

## Step 2 行为摘要（D6/D7 / 口径#7）

1. **进度**：事件带 `status`（idle/downloading/paused/stopped/error/done）、`percent`、`bytesPerSecond`（约 2s 滑动窗），保留原 `phase`。
2. **控制**：Manager 持有每组件 `AbortController`。`pause` abort 并保留 `.part`；`stop` abort 并删 `.part`/staging；`retry` → `ensure`（有 `.part` 则 Range）。`abortAll`（quit）等同 pause。
3. **续传**：稳定路径 `bundles/.tmp/<id>-<sha16>.part`；带 `Range`；非 206 则打日志并整文件重下；落盘前整文件 sha256。
4. **清扫（D6）**：构造时清空 `.tmp` 下 `.part` 与 `embed-model-*` staging。
5. **IPC（仅 desktop）**：`litecode:component-pause|stop|retry` + preload；不碰 `web/`。

## 测试证据

```bash
cd desktop && npm test
```

| 阶段 | 结果 |
|---|---|
| Step 1 完成后 | **121 pass / 0 fail** |
| Step 2 完成后（最终） | **127 pass / 0 fail**（suites 47） |

新增覆盖：mutex/单飞、recovery 计数、D8 清空、隧道参数、sidecar intentionalStop、SpeedTracker/cleanup、pause/keep part、stop/删 part、Range 续传、无 206 回退、abortAll。

## 手测清单（未在本机实跑 Electron；供验收）

- [ ] Open Remote 连上后杀隧道进程 → 断开事件 + 限次自动重连；Hub Disconnect 不重连
- [ ] 本机工作区杀 sidecar → 限次自动重启；手动回 Hub 不误重启
- [ ] 双击 Connect / 快速连点 complete → 第二次被拒或幂等，无双 serve 孤儿
- [ ] 切工作区中途再开另一工作区 → 无双 sidecar
- [ ] 大组件下载：暂停再续、停止、失败重试；下载中关窗 → 后台 abort（保留 `.part`）

## 已知缺口 / 未做

- **D3 桌面 CI**：明确不做（本包禁区）。
- 自动重连依赖 `managedRemote.targetId` + `lastWorkspace`；无历史目标时只断开提示、不重连。
- Range 依赖 CDN/GitHub；不支持时已降级整下。
- Hub/web UI 未接 pause/stop 按钮（仅 desktop IPC/preload API）；进度条仍靠既有 component/remote-progress 通道扩展字段。
- 多实例并发下载（D11）未做。

## 建议给前端中层的短结论

WP-041 两步已串行落地：隧道/sidecar 自动恢复+生命周期单飞（D1/D2/D4/D5/D8），随后组件下载可暂停/停止/重试与 Range 续传并启动清扫 `.tmp`（D6/D7）。`cd desktop && npm test` → **127 pass / 0 fail**。未 commit、未 deliver；请审 `WP-035-desktop.md` 后 deliver。
