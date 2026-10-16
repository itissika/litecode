# WP-170 web 实施记录（S3–S6，父包 WP-168）

已 `wp.mjs start`。在 WP-158 未提交改动之上改，没有回退任何东西。只改了 web/，没碰 desktop 和后端，没 commit，没删测试，dk 主题保持不变，也没加 3 秒兜底。
补丁：`WP-170-web.patch`（同目录），17 个文件，只含 WP-170 的改动。connectionStore、sessionStore、settingsStore 这三个文件，基线取的是 HEAD 加上 WP-159 补丁，再和当前文件 diff，所以 WP-158 的行不在补丁里。

## 改动摘要
- **S3 monaco 懒加载**
  - `main.tsx` 删掉了 `import "./lib/monaco"`。
  - `registry.ts` 的 editor 改用新文件 `dockview/config/lazyEditorPanel.tsx`：用 `React.lazy` 加载 EditorPanel，外面包 Suspense，fallback 是 `bg-(--_dk-editor)` 的空 div。
  - `EditorPanel.tsx` 第一条 import 是 `../../lib/monaco`，保证本地 bundle 的 loader 配置在 EditorPane 求值之前完成。
- **S4 字体**
  - `main.tsx` 只同步引入 inter/400 和 jetbrains-mono/400。
  - 其余 5 份字重放进新文件 `fontsDeferred.ts`，由 `whenIdle().then(() => import("./fontsDeferred"))` 在空闲时加载。
  - 新文件 `lib/idle.ts`：有 requestIdleCallback 就用它，没有就退回 setTimeout 0。
- **S5 whenCentreSettled**
  - 新增叶子模块 `lib/centreSettle.ts`，不 import 任何东西，避免 store 之间的循环依赖：之前直接写在 centreChat 里，单测就报了 TDZ 错误。
  - 它提供 `whenCentreSettled`、`noteRestoredCentre`、`noteSessionListSettled`、`noteRootAttached`、`noteTransportFailed`、`settleCentre`，以及测试用的重置函数。
  - `centreChat.ts` 判定走哪条路径：
    - 路径 A：调用 newSession 后订阅 sessionStore 的 `pendingSessionOp`，它不再是 "new_session" 时就 settle；
    - 路径 B：centre 已有面板时调用 `noteRestoredCentre`。
  - 接入点：
    - `connectionStore`：
      - 非 connected 分支（只限 disconnected 或 reconnecting）调用 `noteTransportFailed`；
      - hello 时 engines 改为 `whenCentreSettled → whenIdle → ensureLoaded`；
      - 收到 session/list 推送时调用 `noteSessionListSettled`；
      - ensureSubscribe 成功时调用 `noteRootAttached`。
    - `sessionStore` listSessions 的 finally 调用 `noteSessionListSettled`。
    - `settingsStore.noteWorkspaceReady`：`noteTransportReady()` 仍然立即调用；summary 改在 `await whenCentreSettled(); await whenIdle()` 之后请求。firstRun 的逻辑不变。
- **S6** `FileTree.tsx`：挂载后 `whenCentreSettled().then(() => alive && loadRoot())`，卸载时取消。treeStore 不改。

## 三条路径的触发点（实现后的行号）
- **A 新 profile（centre 为空）**：
  - 新建入口：centreChat 的 ensureCentreChat 调用 newSession。
  - settle：成功分支 `sessionStore.ts:594` 或失败分支 `sessionStore.ts:599` 把 `pendingSessionOp` 置回 null。
  - 一定会 settle：RPC 有 30s 超时（`agentWs.ts:292`），socket 断开时挂起的请求会被统一 reject。
  - 如果调用时没有发出我们自己的 session/new（锁被别的操作占用，或 newSession 被测试 stub），立即 settle。
- **B 恢复布局（centre 已有面板）**：
  - settle 信号，三个任一即可：
    - listSessions 的 finally（`sessionStore.ts:694`）；
    - session/list 推送（connectionStore 的 session/list 分支）；
    - ensureSubscribe 成功（connectionStore 的 ensureSubscribe 的 then）。
  - 列表信号会被锁存，早于 layout settle 到达也算数。
- **C 连接失败或重连**：
  - connectionStore 的 onConnectionChange 收到 disconnected 或 reconnecting 时调用 noteTransportFailed。
  - 握手有 2s 超时（`agentWs.ts:26`），所以不会一直卡在 connecting。
  - settle 每次页面加载只发生一次，之后的重连不再等待。

## Q2 结论：不存在“已连接但不渲染工作区”的 App 状态，不需要额外触发
用 rg 和直接阅读核对了这些位置：
- `main.tsx`：只渲染 `<App/>`，没有条件分支。
- `App.tsx`：无条件渲染 `<AppShellDockview/>`、`<ToastHost/>`、`<ClickShield/>`，没有提前 return，也没有门控。
- `dockview/AppShellDockview.tsx`：无条件渲染 `DockviewReact`。它的 onReady 是 `useDockviewConfig.ts:21`，会调用 `startWorkbench`；`lifecycle.ts` 的所有分支最终都会调用 `noteLayoutSettled`。
- `WelcomeWatermark` 只是 dockview 内部空网格时的水印，工作区本身仍然渲染。
- rg 搜 `AuthGate|LoginPage|WorkspacePicker|PickWorkspace|Onboarding|SplashScreen`，web/src 里没有命中。
- 选择工作区、鉴权这类入口在 desktop 的 hub 页（hub.ts），不在 web 端；web 的 index.html 只有一个入口脚本 `/src/main.tsx`。

结论：只要页面渲染了，layout 就一定会 settle，A、B、C 已覆盖所有情况，所以没有新增“非工作区状态”的触发和测试。首启跳 Provider 页由新测试“holds the summary until the centre settles, then still opens the Provider page”覆盖，原有的两条 firstRun 用例也保持通过。

## 测试改动（不删测试，只改 setup 或 mock）
- `centreChat.test.ts`：新增 A1 到 A4（A4 是只有 popout、主网格为空的情况），以及 B1 到 B3（B3 是列表回复先到、centre 里没有根面板的情况）。
- `connectionStore.reconnect.test.ts`：新增 C1、C2、C3（C3 验证 hello 后 engines 要等 settle 才请求）。mock 的 session stub 补了 `onHello`，beforeEach 和 afterEach 里重置 settle 状态。
- `settingsStore.test.ts`：“workspace ready”这一组的 beforeEach 先 settle centre，这是 setup 改动，原有两条用例的断言没改；另新增一条 summary 等 settle 的用例。
- 新增 `components/FileTree.settle.test.tsx`（2 条）和 `bootImports.test.ts`（S3、S4 的源文件断言，3 条）。

## 单文件 vitest（轻量，--maxWorkers=1，guard 守护）
| 批次 | 范围 | 结果 | 最低 MemAvailable |
|---|---|---|---|
| 1 | centreChat、connectionStore、connectionStore.reconnect、settingsStore、sessionStore.lifecycle、sessionStore.listDedupe、engineStore、treeStore、firstRun、FileTree.settle、bootImports | 11 个文件 / 92 条全部通过 | 4.95GiB |
| 2 | 间接相关：workspace、agentWs、useDockviewConfig、KnowledgeBrowser、SubagentOps、SubagentRosterPanel、SessionList、toolCallStatus、sessionStore.lifecycle、treeStore、messageStore、bootImports | 12 个文件 / 133 条全部通过 | 5.12GiB |
日志：`.tmp/wp159/wp170-related.log`、`.tmp/wp159/wp170-related2.log`。

## 待跑的重型任务（等后端中层放行，并与 WP-169 错开，串行执行）
1. web `tsc -b`：类型还没验证过，单测只做了转译。
2. web 全量 `vitest --maxWorkers=1`：基数 177/1365 加上新增的 17 条。
3. web knip：不得高于 23/21/1/0/0。重点看新导出 `settleCentre`、`resetCentreSettleForTests`、`noteRestoredCentre`，以及 lazyEditorPanel、fontsDeferred 这两个新文件。
4. （是否跑 build 由中层定）可选 `vite build`：确认 monaco 被拆成了独立 chunk、首屏 chunk 里没有 monaco。

## 重型验证（2026-10-15，后端中层放行，guard 阈值 4.5GiB，严格串行）
| 步骤 | 结果 | 最低 MemAvailable |
|---|---|---|
| 1 web tsc -b | rc=0，一次通过 | 4.65GiB |
| 2 web vitest 全量 --maxWorkers=1 | 179 个文件 / 1381 条全部通过（比基数 177/1365 多 2 个文件、16 条） | 4.60GiB |
| 3 web knip | 导出 23 / 类型 21 / 文件 1 / devDep 0 / 未声明 0，与上限持平；本包新增的文件和导出一条都没报 | 5.35GiB |
| 4 web vite build | **被 guard 杀掉**：模块转换完成（9266 个模块）后，MemAvailable 掉到 4,425,400 kB（约 4.22GiB），低于 4.5GiB。dist 没有生成，进程已不在 | 4.22GiB |

- 旧入口大小：**未测**。build 之前 web/dist 不存在，旧报告里也查不到入口 chunk 的大小记录。
- 按规则停下：build 的三项证据（入口大小、没有 monaco、只加载两份 400 字重）**还拿不到**，WP-170 **没有 deliver**。代码没有改动，补丁不变。
- 日志：`.tmp/wp159/wp170-{tsc,vitest,knip,build}.log`。

## 重启后现场核对（2026-10-15 09:35）
- web/ 未提交改动都在：18 个 ??、6 个 D、163 个 M；WP-170 的 6 个新文件都在。
- 补丁与工作区一致：补丁里 17 个文件的所有新增行都在当前文件里，补丁生成之后没有任何 web/src 文件被改动，所以不用重导出。
- 产物齐全：WP-170-web.md、WP-170-web.patch、WP-168-desktop.md、WP-169-desktop.patch，以及 B1、B2、B3 三份。
- `.tmp` 齐全：guard.sh、wp159-tools 的 knip、web.patch、wp170-*.log 共 6 个、`/tmp/audit`。
- web/dist 不存在（上次 build 被杀）。
- 当前 MemAvailable 约 2.4GiB，低于 4.5GiB，不能跑重型任务。desktop 有 21 项改动，不是本包的，没碰。

## 重跑命令（放行后；内存不够 4.5GiB 时 guard 会直接杀掉）
```
cd /workspace/litecode/web
G=/workspace/litecode/.tmp/wp159/guard.sh
# 先确认 MemAvailable ≥ 5GiB：awk '/MemAvailable/{print $2/1048576" GiB"}' /proc/meminfo
# 方案 A（推荐）：限制 Node 堆后再跑 build，已过的 tsc/vitest/knip 不用重跑（代码没变）
PATH=/exec-daemon:$PATH NODE_OPTIONS=--max-old-space-size=2048 $G wp170-build /exec-daemon/node node_modules/vite/bin/vite.js build
# 全量重跑（如需要）：
$G wp170-tsc /exec-daemon/node node_modules/typescript/bin/tsc -b
$G wp170-vitest /exec-daemon/node node_modules/vitest/vitest.mjs run --maxWorkers=1
PATH=/exec-daemon:$PATH $G wp170-knip /workspace/litecode/.tmp/wp159-tools/node_modules/.bin/knip --reporter json
# build 完成后（轻量取证）：
ls -l dist/assets | sort -k5 -n | tail -5
rg -o '<script[^>]*src="[^"]+"|<link[^>]*modulepreload[^>]*>|<link[^>]*stylesheet[^>]*>' dist/index.html
gzip -c dist/assets/<入口chunk>.js | wc -c
rg -l "monaco" dist/assets -g '*.js'
rg -o "@font-face\{[^}]*font-weight:[0-9]+" dist/assets/*.css | sort | uniq -c
```
环境变量：`PATH=/exec-daemon:$PATH`（用 v22.14.0）；`NODE_OPTIONS=--max-old-space-size=2048` 仅用于 build。

## build 证据（版本 A：实测，保留作参考；GM 定版本 B 后不作为验收口径，2026-10-15 11:40 HKT）
- 命令：`web/` 下 `NODE_OPTIONS=--max-old-space-size=2048 /exec-daemon/node node_modules/vite/bin/vite.js build`（只跑 vite build；package.json 的 build 脚本含 tsc -b，所以没用）。自写内存守护：每 1s 采样 MemAvailable，低于 2097152 kB 就杀掉进程树。rc=0，用时 3.27s，没触发守护；MemAvailable 最低 4,403,628 kB（约 4.20GiB），最高 6,268,372 kB。原始输出：`WP-170-vite-build.txt`；守护日志：`WP-170-memguard.txt`。
- 旧入口大小：build 前 web/dist 不存在，旧报告没有记录，写「未测」。
- 入口 chunk：`assets/index-VLsqxIc3.js`，原始 5,264,416 B（vite 报 5,264.41 kB），gzip 1,083,964 B（`gzip -c` 默认级别；vite 报 1,095.19 kB）。体积大，但不是 monaco 带来的，见下一条。
- 入口 chunk 和 index.html 的 modulepreload 中没有 monaco：
  - index.html 的 script 和 modulepreload 列表：`index-VLsqxIc3.js`，以及 rolldown-runtime-C0FnF6B9、react-DiVRNtpo、preload-helper-BLdHbfdN、connectionStore-DelWKO2_、markdownFlush-D1NyG5ly、path-BCuXWHjK、editorStore-LPgH70bF；css 只有 `index-CztSOLd2.css`。这 7 个预载 chunk 用 rg 搜 monaco，0 命中。
  - 入口 chunk 的静态 import 列表：rolldown-runtime、react、preload-helper、editorStore、connectionStore、path，不含 monaco。入口里唯一的 `monaco-editor` 字样是 About 面板的版本号字符串 `{name:"monaco-editor",version:"0.55.1"}`，不是 import。
  - monaco 单独成的 chunk 名：`EditorPanel-D_WZAQPi.js`（1,375,702 B）、`editor.api-BdB7rlVo.js`（2,735,985 B），另有 editor.worker、json.worker、jsonMode。这些都只通过入口里的动态 import（`__vite__mapDeps`，其中 m.f 首项是 EditorPanel）加载。
- 首屏只加载两份 400 字重：
  - index.html 引用的 `index-CztSOLd2.css` 一共 13 个 @font-face，全部是 font-weight:400：Inter 400 有 7 个子集（cyrillic、cyrillic-ext、greek、greek-ext、latin、latin-ext、vietnamese），JetBrains Mono 400 有 6 个子集（同上，少 greek-ext），每个子集都是 woff2 + woff。没有 200、300、500。
  - 其余 5 份字重在 `fontsDeferred` 对应的 chunk 里：`fontsDeferred-Cdw9oN_j.css`，含 font-weight 200×7、300×13、500×13（Inter 200/300/500，JetBrains Mono 300/500），只在入口里通过动态 import 引用。
  - dist 里其余字体（KaTeX_*、codicon）只由按需 chunk 引用，不在首屏 css 里。
- 其他重型校验（WP-158 重验，同一份 web 代码）：
  - `.litecode/ops/artifacts/WP-158/rerun-web-tsc.txt`：0 字节，即 tsc -b 无输出，rc 0（见 `rerun-summary.md:9`）；
  - `rerun-web-vitest.txt`：Test Files 179 passed、Tests 1381 passed (1381)；
  - `rerun-web-knip.txt`：Unused files 1 / exports 23 / exported types 21，devDep 和未声明均为 0，即 23/21/1/0/0（见 `rerun-summary.md:11`）。


## build 证据（采用源码层面证据；GM 定版本 B）
- 入口大小（原始和 gzip）：**待 build**。旧入口大小：未测。
- modulepreload 里有没有 monaco：**待 build**。
- monaco 不在首屏的源码依据：
  - `bootImports.test.ts` 已通过，它断言：main.tsx 的静态 import 不含 `./lib/monaco`；registry.ts 不静态 import EditorPanel；EditorPanel.tsx 的第一条 import 是 `../../lib/monaco`；
  - 引用链：rg 核对 web/src 里 `lib/monaco`、`@monaco-editor/react`、`monaco-editor` 的运行时 import，只出现在 `lib/monaco.ts`、`components/EditorPane.tsx` 和 `lib/monacoShaders.ts`，其中 monacoShaders 只被 `lib/monaco.ts` 引用，且 `theme/monaco.ts`、`lib/litecodeLsp.ts` 对 monaco 只用 `import type`；EditorPane 只被 EditorPanel 引用，EditorPanel 只被 `lazyEditorPanel.tsx` 用动态 `import()` 引用。所以 monaco 只经由一个动态 import 边进入，首屏静态依赖里没有它。
  - 这是源码层面的证据，不能替代 build 产物上的 preload 核对。
- 首屏只有两份 400 字重的依据：
  - `bootImports.test.ts`：main.tsx 静态引入的 `@fontsource/*` 只有 inter/400.css 和 jetbrains-mono/400.css；另 5 份只在 `fontsDeferred.ts`，且只通过 `import("./fontsDeferred")` 动态加载；
  - 产物层面的 @font-face 核对：**待 build**。

## 收口说明（2026-10-15 13:53，GM 指示）
**vite build 未在云箱跑。** 第一次在 MemAvailable 降到 4.22GiB 时被 guard 杀掉，之后按 GM 指示不再在云箱重跑。以下三项**待 build，由 Andy 本地验证**：
1. 入口 chunk 大小（原始和 gzip）；旧入口大小：未测。
2. 入口 chunk 和 index.html 的 modulepreload 里没有 monaco，并确认 monaco 单独成的 chunk 名。
3. 首屏只加载 inter 400 和 jetbrains-mono 400 两份字体。

本地核对命令（在 web/ 下）：
```
npm run build        # 即 tsc -b && vite build，输出到 dist/
ls -l dist/assets | sort -k5 -n | tail -5
gzip -c dist/assets/<入口chunk>.js | wc -c
rg -o '<script[^>]*src="[^"]+"|<link[^>]*modulepreload[^>]*>|<link[^>]*stylesheet[^>]*>' dist/index.html
rg -l "monaco" dist/assets -g '*.js'          # monaco 单独成的 chunk；入口 chunk 和 index.html 引用里不应出现
rg -o "@font-face\{[^}]*font-weight:[0-9]+" dist/assets/*.css | sort | uniq -c   # index.html 引用的 css 里只应有两份 400
```
云箱已通过的验证：tsc -b rc=0、vitest 179 个文件 / 1381 条全过、knip 23/21/1/0/0（见上文）。
