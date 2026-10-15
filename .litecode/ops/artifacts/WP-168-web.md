# WP-168 web（WP-170）实施记录

## 状态
- 已 start，S3–S6 的代码已写完；只跑了相关单文件 vitest，按指示停下。
- 没 commit，没碰 desktop 和后端。在 WP-158 未提交改动之上做的最小 diff。
- 补丁：`.litecode/ops/artifacts/WP-168/WP-170-web.patch`，共 17 个文件（11 个修改、6 个新增），+484 / −14。
  - 基线是 HEAD 加上 WP-159 补丁，补丁里只含 WP-170 自己的改动。重建基线时，3 个 store 的差异核对过，只有本包的改动。

## 改动
- **S3**：
  - 删掉 `main.tsx` 里的 `lib/monaco` 引入；
  - registry 的 editor 改为 `LazyEditorPanel`（新文件 `dockview/config/lazyEditorPanel.tsx`：React.lazy 加 Suspense，fallback 是 `bg-(--_dk-editor)` 的空 div）；
  - `EditorPanel.tsx` 第一行改为 `import "../../lib/monaco"`，保证本地 bundle 的配置先于 EditorPane 生效。
- **S4**：`main.tsx` 只同步引入 inter 和 jetbrains-mono 的 400；其余 5 份移到 `fontsDeferred.ts`，由 `whenIdle()` 之后动态 import。新增 `lib/idle.ts`（requestIdleCallback，没有就用 setTimeout 0）。
- **S5**：
  - 新增 `lib/centreSettle.ts`：不依赖任何模块，用来打破 connectionStore → centreChat → sessionStore → connectionStore 的循环引用。提供 `whenCentreSettled`、`settleCentre`、`noteRestoredCentre`、`noteSessionListSettled`、`noteRootAttached`、`noteTransportFailed`。
  - `centreChat.ts` 按路径 A 和 B 判定：路径 A 订阅 pendingSessionOp，不是 "new_session" 时立即 settle；路径 B 走 noteRestoredCentre。
  - `connectionStore.ts`：
    - 收到 hello 时，engines 改为 `whenCentreSettled().then(whenIdle).then(ensureLoaded)`；
    - 进入 disconnected 或 reconnecting 时调用 noteTransportFailed；
    - 收到 session/list 推送时调用 noteSessionListSettled；
    - ensureSubscribe 成功时调用 noteRootAttached。
  - `sessionStore.ts`：listSessions 的 finally 里调用 noteSessionListSettled。
  - `settingsStore.ts`：noteTransportReady 仍然立即调用；summary 改为等 `await whenCentreSettled(); await whenIdle();` 之后再请求，firstRun 的逻辑不变。
  - `connectionStore.ts:441`（settings/changed）不改（Q3 已获认可）。
- **S6**：`FileTree.tsx` 挂载后，等 `whenCentreSettled()` 再 loadRoot，带卸载取消标志，没有超时兜底。

## Q2 结论：不存在“已连接但不渲染工作区”的 App 状态
用 rg 和静态阅读查过：
- `main.tsx:21`：无条件 render `<App />`；
- `App.tsx:24-30`：无条件渲染 `<AppShellDockview />`、ToastHost 和 ClickShield，没有任何条件分支或 return null；
- `AppShellDockview.tsx`：无条件渲染 `<DockviewReact onReady>`；`WelcomeWatermark` 只是 dockview 内部空网格的水印，不是另一个页面；
- `useDockviewConfig.ts:21`：onReady 里无条件调用 `startWorkbench`；
- `lifecycle.ts`：startWorkbench 的所有分支都会走到 finishChromeAndSettle → noteLayoutSettled；
- 在 web/src 里搜 `AuthGate|LoginPage|WorkspacePicker|PickWorkspace|Onboarding|SplashScreen`，**0 处命中**。web 里没有鉴权门；工作区选择和欢迎页（Hub）在 desktop 的 hub.ts，那时根本不会加载 web 工作台。

所以 Q2 不需要额外处理，也不加 vitest。首启跳 Provider 页的功能由 settingsStore 新测试保证：summary 等 settle 之后才请求，settle 后仍会打开 Provider 页。

## 单文件 vitest（轻量，--maxWorkers=1，guard 守护）
- 第一组 11 个文件：**11 个文件 / 92 条全部通过**，最低 MemAvailable 4.95GiB。文件是 centreChat、connectionStore、connectionStore.reconnect、settingsStore、sessionStore.lifecycle、sessionStore.listDedupe、engineStore、treeStore、firstRun、FileTree.settle（新增）、bootImports（新增）。
- 第二组（引用了 registry、EditorPane、FileTree 的测试）：workspace.test、treeStore.test，2 个文件 / 9 条全部通过，最低 5.36GiB。
- 新增测试共 18 条：
  - 路径 A：A1–A4，其中 A4 是“只有独立窗口”的情况；
  - 路径 B：B1–B3，其中 B3 是“没有根面板、列表回复早于 layout settle”的情况；
  - 路径 C：C1–C3，其中 C3 是“收到 hello 后 settle 前不拉 engines”；
  - summary 等 settle、FileTree 2 条、bootImports 3 条。
- 改动的已有测试（只改 mock 或准备步骤，没删测试）：
  - settingsStore.test 的 "workspace ready" 两条：beforeEach 里先 settle centre；
  - connectionStore.reconnect.test 的 session 桩加了 `onHello: vi.fn()`；
  - 两个文件都在用例之间重置 centreSettle 的状态。

## 待跑的重型任务（等后端中层放行，并和 WP-169 错开，串行执行）
1. web `tsc -b`；
2. web vitest 全量 `--maxWorkers=1`，基数 177/1365，预计增加 2 个文件、18 条；
3. web knip，要求不高于 23/21/1/0/0。centreSettle 的 resetCentreSettleForTests 只被测试使用，knip 可能会报，需要关注。
4. build 本包不需要。如果要验证 monaco 是否真的拆成了单独的 chunk，需要额外放行一次 web build。
