# WP-158 desktop B2（WP-161 桥接契约）

执行人：frontend_desktop · 主树 /workspace/litecode（ops/team-framework），只动 desktop/，未 commit，保留 WP-160 未提交改动。

## 逐项改动
1. 新建 `desktop/src/bridge-contract.ts`（唯一源头）。类型用 export type，通道名用 export const。
2. preload.ts：只 `import type` 契约；暴露对象改为 `const bridge = {...} satisfies LitecodeDesktopBridge`，再 `exposeInMainWorld("litecode", bridge)`。删掉本地手抄的 7 个类型和 4 条 "Keep identical" 注释。
   - 判断：preload 是 sandbox，tsc 直出 CommonJS、不打包，运行时 import 会生成 `require("./bridge-contract")` 而加载失败。所以 preload 保留通道字面量，但类型标为 `typeof <契约常量>`，字面量一旦和契约不一致即编译报错。
3. main.ts：COMPONENT_PROGRESS / REMOTE_PROGRESS / REMOTE_DISCONNECTED 三个通道改从契约导入，删除 main.ts 自己的 REMOTE_DISCONNECTED_CHANNEL。
4. components.ts：删 COMPONENT_PROGRESS_CHANNEL；ComponentId / ComponentPhase / ComponentDownloadStatus / ComponentProgressEvent 改为从契约 import type 再 re-export（main、测试的现有 import 不变）。
5. browser-host.ts 删 BROWSER_STATE_CHANNEL，browser-view.ts 改从契约导入；remote-progress.ts 删 REMOTE_PROGRESS_CHANNEL 和本地 RemoteProgressEvent/Stage，改为从契约 re-export 类型。
6. X5：`percent` 改必填。发事件点共 3 处：emitProgress（原本就有 `?? progressPercent(...)`）、catch 里 paused 和 error 两处补 `percent: 0`。
7. 去掉 `"stopped"`：契约和 ComponentDownloadStatus 里删除；DownloadControlError.kind 收窄为 `"pause"`，删掉 controlKindFromAbort 的 "stop" 分支和 `kind === "stop"` 删 .part 的分支（WP-160 删 stop IPC 后已无生产者）。测试里没有 "stopped"/"stop" 引用，未改。
8. 契约不含 focusWorkspace，也不含 pause/stop/retry 组件控制。
9. tsconfig：`include: ["src/**/*"]`，新文件自动纳入，未改。

## 契约摘要（bridge-contract.ts）
- 常量：REMOTE_PROGRESS_CHANNEL、COMPONENT_PROGRESS_CHANNEL、BROWSER_STATE_CHANNEL、REMOTE_DISCONNECTED_CHANNEL = "litecode:remote-disconnected"。
- 远程断开：主进程→渲染端单向；载荷 `RemoteDisconnectedEvent = { reason: string }`，reason 只作排障，界面不显示文字；触发点：隧道意外断开、隧道自动重连失败、远程模式下本地 sidecar 崩溃。不设 reconnected 事件：重连成功整窗 reload 是唯一恢复路径；web 断开态只由此事件置上、只由 reload 清除。订阅口 `onRemoteDisconnected(handler): Unsubscribe`。
- 类型：LitecodeDesktopBridge（与 preload 实际暴露的 45 个方法一一对应）、RemoteProgressEvent/Stage、ComponentId/Phase/DownloadStatus/ProgressEvent/Status、EnsureComponentResult、RecentWorkspace、RemoteHistoryItem、BrowserBounds/State、SessionMode、UiTheme、StartRemoteSessionInput、Unsubscribe。

## web 引用方案（WP-162 落地，本包未改 web）
- 推荐：相对路径 `import type { LitecodeDesktopBridge } from "../../desktop/src/bridge-contract";`（层级按 web 文件实际位置），并在全局声明 `interface Window { litecode?: LitecodeDesktopBridge }`。
- 备选：web tsconfig 加 `paths: { "@desktop-contract": ["../desktop/src/bridge-contract.ts"] }`；若 web 的 tsconfig 有 include/rootDir 限制，需要把该文件纳入。
- 只用 import type（启用 verbatimModuleSyntax/isolatedModules 时也安全），web 不引用通道常量，打包产物不含 desktop 代码。

## 静态证据（rg）
- `rg -n '"stopped"|"stop"|Keep identical|must match preload' desktop/src` → 无结果
- `rg -n 'focusWorkspace|pauseComponent|stopComponent|retryComponent' desktop/src/bridge-contract.ts desktop/src/preload.ts` → 无结果
- `rg -n '_CHANNEL = "' desktop/src` → 只剩 bridge-contract.ts 的 4 个导出，以及 preload.ts 的 4 个 `typeof` 绑定字面量
- `rg -n 'percent' desktop/src/components.ts` → 833（emitProgress）、917、930（catch 补 0）

## git status --short desktop
之前：M README.md、browser-host、bundle-paths、components.test、components、dev-ui、hub、instance-registry、ipc-trust、main、preload、remote-progress、remote-state、ssh-session（均为 WP-160）。
之后：在此基础上新增 `M desktop/src/browser-view.ts`、`?? desktop/src/bridge-contract.ts`；本包另改了 browser-host、components、main、preload、remote-progress。

## 重型检查（后端中层 10-15 放行后，串行，每次只跑一项）
内存守护：/tmp/guard.sh 每秒记录一次 MemAvailable 到 /tmp/wp161-mem.log，低于 4.5GiB 就立即 kill。全程没有触发 kill，最低值 5583816 kB（约 5.32GiB，02:01:05）。
工具只装在 /tmp/audit（`ELECTRON_SKIP_BINARY_DOWNLOAD=1 npm i --prefix /tmp/audit knip typescript@5.8 @types/node@24 electron@44 --ignore-scripts`），仓库的 package.json 和锁文件都没动。

1. tsc --noEmit：`cd /tmp/audit && node node_modules/typescript/bin/tsc -p tsconfig.wp161.json`（tsconfig 继承 desktop/tsconfig.json，类型从 /tmp/audit 解析）。
   - 第一次跑报 1 个错：preload.ts 少导入 `BrowserBounds` 类型。补上 import type 后重跑，0 错误。
   - `typeof` 引用 import type 常量的写法编译通过。
2. 单测（1 并发）：tsc 编译到 desktop/dist/（已 gitignore），执行 `node scripts/copy-theme-tokens.mjs`，再跑 `NODE_PATH=/tmp/audit/node_modules node --test --test-concurrency=1 dist/*.test.js`。
   - 结果：127 个测试，127 通过、0 失败（47 个 suite）。
   - 跑完已删掉 dist/。
   - 先前试过把测试输出到 /tmp，有 12 项因为按相对路径找不到仓库里的 scripts/ 和 package.json 而失败，属于环境问题，与代码无关，所以改为输出到 desktop/dist。
3. knip：`node /tmp/audit/node_modules/knip/bin/knip.js --config /tmp/audit/knip.wp161.json --no-progress --reporter compact`，配置与 WP-151 相同。
   - 和 WP-151 基线比：未用导出从 9 组降到 0；基线里的未用导出类型（ComponentFormat、InstallRecordEntry、ComponentLocate、InstanceRecord、RemoteProgressStage、InstallTarProgress*）都已不再出现。
   - electron-builder 和 unlisted binaries 两项与基线相同（WP-151 S4 已判为误报）。
   - 新增 1 条：bridge-contract.ts 的 RemoteProgressStage、SessionMode、UiTheme、Unsubscribe 是未用导出类型。它们是给 web 用 import type 引用的公开契约类型，desktop 内部本来就不用，而 knip 的范围不含 web，属于预期。WP-162 接上 web 后应会消失；如需清零，也可以去掉这 4 个的 export。

## 本轮修复
- preload.ts：import type 补上 `BrowserBounds`。
