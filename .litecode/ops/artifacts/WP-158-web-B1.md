# WP-158 web B1 交付物（WP-159）

## 批复记录（前端中层，2026-10-14 23:08）
- 计划已批；仍不 start，等 WP-157 采样结束。
- **auth 帧**：可以删。删 agentWs.ts:140-141；改 agentWs.test.ts:87 和 :106（这两条把 "auth" 当示例方法名测 send() 的错误路径，换成仍然有效的方法名，断言不变）；needsAuth 若别处不用则一并删；保留补测「连接后不再发 auth 帧」。
- **U4**：第 3–5 条（JsonRpcRequest、JsonRpcResponse、JsonRpcNotification）直接删；第 1、2、6、7 条（SessionAttached、CompactStarted、LspNotification、TransportRequest）等后端结果。
- **Q1**：U6（中文统一成英文）不在本包，留给多语言包。
- **Q2**：交付物命名为 WP-158-web-B1.md。
- **Q3**：那 209 个只在本文件内用到却导出了的符号，批量去掉 `export` 关键字，不动实现；被测试直接引用的保留。
- **Q4**：@tanstack/virtual-core 放 devDependencies。
- **Q5**：等后端确认不再发 compact_started 后，删除 connectionStore.ts:388 的处理分支，并同步改测试。

## 批复补充（前端中层，2026-10-14 23:22）
1. agentWs.test.ts:87 和 :106 两条测试不删，方法名换成 `session/list`，断言不变；另补一条测试「连接后不发 auth 帧」。不需要 reject。
2. U4 七项全删：
   - SessionAttached：删类型。原因是前端不需要它——后端仍会发 session/attached，但紧跟其后的 snapshot 已经覆盖了这部分信息（不是后端已废弃）。
   - CompactStarted：删类型，同时删 connectionStore.ts:388 的处理分支（后端只发 session/compact_lifecycle），并更新相关测试。
   - JsonRpcRequest、JsonRpcResponse、JsonRpcNotification、LspNotification、TransportRequest：删。
3. 环境：在独立 worktree `.tmp/wp158-web`（detached HEAD）里改；node_modules 软链接到主树。package.json 和锁文件只出 diff（/tmp/wp159/deps.diff），不跑 npm。vitest、tsc、knip 等重负载任务等 W4 静默窗口结束后再跑。不 commit。

## 实施记录（2026-10-14 23:22 起，worktree `.tmp/wp158-web`，detached HEAD 3ddc643）
- 环境：`git worktree add --detach .tmp/wp158-web HEAD`；`web/node_modules` 软链接到主树（该路径被 gitignore，不进 diff）。主树 web/ 零改动。没有跑 npm、vitest、tsc、knip。没有 commit。
- 补丁：`/tmp/wp159/web.patch`（`git diff -M HEAD -- web`），129 个文件，+289 / −793。已用 `git apply --check` 确认能干净地打到主树。依赖改动单独放在 `/tmp/wp159/deps.diff`。

### 改动清单
1. **24 项确认可删**（删前用 rg 复核过，web 和 desktop/src 都是零引用）：
   - 删 5 个文件：api/json.ts、theme/dockview.ts、theme/dockview.css、utils/turnPhase.ts、components/knowledge/KnowledgeRelationStrip.tsx。
   - 删 16 个导出：isEmptyItemShell、extractToolFilePath、skipSplash、peekCitation、refreshDiagnostics、toggleTheme、isRevisionedDocument、mcpServersFromStore、headPermission、KnowledgeInlineBody、KnowledgeStatusBadge、newestUserSeq、groupHasComponent、extractSymbolRefs、PermissionModal 里的 PermissionGrantOpts 中转行、menuItems 常量。
   - 连带删掉因此变成没人用的代码：adapter.ts 的 FILE_TOOLS、KnowledgeMarkdown.tsx 的 knowledgeStatusLabel 和 knowledgeFirstLineSegments import、markers.ts 的 CitedSymbol、transcriptUserRailMarks.ts 的 HumanRow import、queries.ts 的 DockviewPanel import、settingsStore.ts 的 mergeLayeredMcp import。
   - 3 个 devDependency（remotion、@remotion/cli、@types/opentype.js）只放进 deps.diff。
2. **疑似项**：
   - 删 LivePanel 类型；
   - 删 API_PROVIDER_IDS，在 providerBilling.ts 加了一行注释说明 API 侧口径；
   - 删 readDkTokens，同步改 theme/README.md；
   - 把 lib/knowledge/fixture.ts 移到 src/test/fixtures/knowledge.ts，它内部的 import 改成指向 lib/knowledge/，3 个测试的 import 也已更新。
3. **U4 七项**：从 api/types.ts 删除 SessionAttached（前端不需要；后端仍发 session/attached，紧跟的 snapshot 已覆盖）、CompactStarted、JsonRpcRequest、JsonRpcResponse、JsonRpcNotification、LspNotification、TransportRequest。connectionStore.ts 的 `session/compact_started` 处理分支也一并删除（后端只发 session/compact_lifecycle）。测试里没有引用这个事件，不需要改测试。
4. **auth 帧**：
   - agentWs.ts 删掉 onopen 里的 `send("auth")` 和 needsAuth 字段（别处无用）；authToken 仍用于拼握手 URL，保留。
   - agentWs.test.ts 两处把方法名换成 `session/list`，断言不变。
   - 新增测试「AgentWsClient auth (WP-159)」：连上后不发 auth 帧，token 在握手 URL 里。
5. **U2**：web/README.md 新增「WS smoke script」一节，写了三个子命令和环境变量。
6. **U3**：dockview/workbench/index.ts 只保留 `export { groupRole } from "./model"`，文件头注释同步调整。
7. **U5 和 §3 文案**：C1–C17 已改，详见补丁。
   - C14、C15 抽成新文件 src/lib/uiCopy.ts 里的 NO_MODELS_HINT 和 REMOTE_SERVE_HINT。
   - C16 用 SessionItem 里的 deleteLabel 变量，aria-label 和 title 共用。
   - C6 本包只改文案，抽共享组件留到 B3。
   - 测试断言同步改了 2 处：SettingsDialog.test.tsx 的 /saving is paused/、AgentsSection.test.tsx 的 "Only the model is configurable."。没有删测试。
8. **Q3 批量去 export**：清单里 209 个，200 个只去掉了 `export` 关键字。其余 9 个：
   - 6 个被测试直接引用，保留：SettingsSummary、LITECODE_PATHS_MIME、AgentChatShell、PersistStatus、GitFile、EMPTY_SLICE；
   - 2 个有别的文件同名 import，保守保留：SessionStatus、PendingQueueBubble；
   - 1 个所在文件已删：JsonValue。

   明细在 /tmp/wp159/unexport-done.txt 和 /tmp/wp159/unexport-report.json。去 export 之后，用逐文件的轻量 AST 检查确认没有留下没人用的本地符号或 import；tsconfig 开了 noUnusedLocals，最终仍以 tsc 为准。
9. **依赖**：deps.diff 改了 package.json，以及 package-lock.json 根节点的声明：删 remotion、@remotion/cli、@types/opentype.js；加 dockview ^8.4.1 到 dependencies、@tanstack/virtual-core ^3.17.2 到 devDependencies。锁文件里 42 条 remotion 相关的 `node_modules/*` 条目，以及它们的间接依赖、dev 标记，没法手工准确剪除，需要落主树时跑一次 `npm install` 来剪除和校正。

### 未覆盖或有疑问的项
- 没跑重负载：vitest、tsc -b、knip 都等 W4 静默窗口结束后再跑。目前只做了 rg 和逐文件 AST 检查，类型错误要靠 tsc 兜底。
- scripts/ws-smoke.mjs 仍然先发 auth 帧（`sendAuth: true`），而且检查的是旧的信封字段（server_hello、session_list），可能已经和现在的 JSON-RPC 协议对不上。U2 规定脚本本身不改，所以 README 只写了用法，这个问题待中层决定。
- 同类文案不在 §3 清单里，本包没改：lib/firstRun.ts:49 的 "200 is a saner default"，以及 EngineView.tsx:416 的 aria-label 里的 "Click the card to toggle."。

## 批复与补充改动（前端中层，2026-10-14 23:28）
### 新增改动（仍在 worktree 里，只做轻量编辑）
- **新增 · ws-smoke.mjs 修复**（GM 裁定保留）：对齐当前的 JSON-RPC 协议。
  - 不再发 auth 帧，token 走握手 URL 的 `?token=`。
  - 改为识别 `server/hello` 通知；list 模式在收到 hello 后发 `{"jsonrpc":"2.0","id":1,"method":"session/list"}`，以拿到 `result.sessions` 数组为通过。
  - bad-handshake 以收不到 `server/hello` 为通过。
  - 一帧里有多行 JSON 时按行解析。
  - README 的「WS smoke script」一节已同步。
  - 只做了 `node --check` 语法检查，实跑放到 W4 之后。
- **新增 · lib/firstRun.ts:49**：探索卡片的文案改为 U5 建议文 "Long tasks may need ~200."。注意：原句提到了 **Max steps**，新句不再提，但这张卡片本身锚定在 Agents › max-steps。
- **新增 · EngineView.tsx:416**：aria-label 去掉 ". Click the card to toggle."。EngineView.test.tsx 用的是前缀正则（`/typescript server, disabled/i`），不受影响，不需要改断言。
- 补丁已重新生成：`/tmp/wp159/web.patch`，131 个文件，+337 / −854；`git apply --check` 能干净地打到主树。

### 护栏
- 单独跑一个重型任务时，MemAvailable 要 ≥ 4.5 GiB；其余情况 ≥ 6 GiB。重型任务不并行。
- 前端的重型任务指：web build、全量 vitest、npm install。每次跑之前和跑完之后都要报给前端中层，由后端中层放行。

### W4 之后的执行顺序
1. 把补丁 /tmp/wp159/web.patch（以及 deps.diff）应用到主树；
2. npm install（等放行），剪除锁文件里 remotion 的残留条目；
3. tsc -b；
4. 全量 vitest（等放行）；
5. knip（/tmp 临时运行），和基线对比；
6. 实跑 ws-smoke（handshake / list / bad-handshake 三种模式）；
7. 删除 worktree `.tmp/wp158-web`。

## 仓库重置后的核对（2026-10-15 01:28）
- worktree `.tmp/wp158-web` 里的 web 改动完整保留：131 个文件，+337 / −854，和上一版一致。git mv 的那次改名、新增的 lib/uiCopy.ts、node_modules 软链接都还在。按改动清单逐项抽查，全部都在，不需要重做。
- /tmp/wp159 丢失后，产物已改放到仓库内（目录被 gitignore）：
  - `.tmp/wp159/web.patch`：从 worktree 重新生成；
  - `.tmp/wp159/deps.diff`：重新生成，只改 package.json 和锁文件根节点的声明；
  - `.tmp/wp159/unexport-done.txt`：去 export 清单。原来的清单已丢失，这份是按 diff 重建的，正则匹配到 198 条，原清单是 200 条；不需要重跑 ts-prune。
- 对主树执行 `git apply --check`：web.patch、deps.diff 单独和合在一起都通过。
- 注意：worktree 里显示 `desktop/build/icon.ico` 被删除（D），这不是本包的改动，原因不明。补丁只包含 web/，这一项没有处理，请中层核查。

## 落主树与验证（2026-10-15 01:41 起；因内存护栏中止）
- **补丁来源**：`.litecode/ops/artifacts/WP-158-patches/WP-159-web.patch` 和 `deps.diff`，用 `cmp` 确认与 `.tmp/wp159/web.patch`、`deps.diff` 逐字节相同。已在主树执行 `git apply`，只改动 web/。应用前的 git status 存在 `.tmp/wp159/git-before-apply.txt`。
- **npm install**（`npm install --no-audit --no-fund`，npm 9.2.0）：rc=0，最低 MemAvailable 5.14 GiB。
  - 锁文件：删掉 257 条，全部属于 remotion 及其间接依赖（rspack、babel、esbuild 平台包、mediabunny 等）；没有新增条目，也没有任何包的版本变化。
  - 另有 7 条只是加了 `peer` / `optional` 标记（acorn、terser 等）；tailwind oxide-wasm32 的依赖改为嵌套在它自己目录下，所以 git diff 显示 +856 / −4016 行。
  - 有 EBADENGINE 警告：某个包要求 node ^22.22.2，当前是 v22.14.0。不影响安装。
- **tsc -b**：
  - 第一次 rc=2，最低 MemAvailable 5.09 GiB。原因是 lib/knowledge 下有 3 个测试（validate、layoutGraph、refDisplay）用相对路径 `./fixture` 引用测试数据，之前搬家时用 rg 漏掉了。已把它们的 import 改为 `../../test/fixtures/knowledge`，属于计划内的修复。
  - 第二次 rc=0，最低 MemAvailable 5.13 GiB。
- **全量 vitest**：刚启动约 5 秒，MemAvailable 就跌到 4.21 GiB（4415272 kB），低于 4.5 GiB 护栏，监控按规则把整个进程组杀掉了。没有任何结果，日志只有 RUN 头。杀掉后 MemAvailable 回升到 5.75 GiB，也没有残留进程。
- **按规则停下**：knip、ws-smoke、删 worktree、deliver 都没有做。
- 内存监控脚本：`.tmp/wp159/guard.sh`。每秒读一次 `/proc/meminfo` 的 MemAvailable，低于 4718592 kB（4.5 GiB）就杀掉整个进程组；日志在 `.tmp/wp159/*.log`。

## 验证完成（2026-10-15 01:45 起；后端中层已放行）
### 修复
- **knowledge fixture 引用**：lib/knowledge/validate.test.ts、layoutGraph.test.ts、refDisplay.test.ts 三处 `./fixture` 改为 `../../test/fixtures/knowledge`，修复后 tsc -b 为 0。
- **U5 文案断言**：AgentsSection.test.tsx 首次引导锚点用例里 3 处 `/runs out on long tasks/` 改为 `/Long tasks may need ~200/`。只改了断言，没删测试；这是测试改动，不需要重跑 tsc。

### 结果
| 任务 | 命令 | 结果 | 最低 MemAvailable |
|---|---|---|---|
| npm install | `npm install --no-audit --no-fund` | rc=0 | 5.14 GiB |
| tsc -b（第一次） | `/exec-daemon/node node_modules/typescript/bin/tsc -b` | rc=2（fixture 引用） | 5.09 GiB |
| tsc -b（修复后） | 同上 | rc=0 | 5.13 GiB |
| vitest 全量（默认并发） | `vitest run` | 被护栏杀掉 | 4.21 GiB |
| vitest 全量 --maxWorkers=2 | `vitest run --maxWorkers=2` | 约 88 秒时被护栏杀掉 | 4.48 GiB |
| vitest 全量 --maxWorkers=1 | `vitest run --maxWorkers=1` | 175 个文件通过、1 个失败；1362 条通过、1 条失败（U5 断言） | 4.83 GiB |
| vitest 单文件 AgentsSection.test.tsx | `vitest run --maxWorkers=1 <file>` | 20 条全部通过 | 5.32 GiB |
| **vitest 全量 --maxWorkers=1（最终）** | 同上 | **176 个文件 / 1363 条全部通过** | 4.96 GiB |
| knip | `.tmp/wp159-tools/node_modules/.bin/knip --reporter json` | 结果见下表 | 5.67 GiB |

### knip 对比（基线取自 WP-151 审计）
| 类别 | 基线 | 现在 |
|---|---|---|
| 未用导出 | 145 | 23 |
| 未用类型 | 101 | 21 |
| 未用文件 | 6 | 1（scripts/ws-smoke.mjs：手动脚本，按 U2 保留并写进 README） |
| 未用 devDependency | 3 | 0 |
| 未声明依赖 | 2 | 0 |

各类都低于基线。剩下的 23 个导出、21 个类型大多是被测试引用、或 workbench 子模块对外的接口，原始 JSON 在 `.tmp/wp159/knip.json`。

### ws-smoke 实跑（三个子命令都通过）
- 环境：用了现成的二进制 `/workspace/.tgt_mid/debug/litecode`（01:23 编译），没有重新编译后端。
- 启动方式：`LITECODE_WEB_DIST=<stub> LITECODE_TOKEN=smoke-tok litecode serve --bind 127.0.0.1:17483 --loopback-only --require-auth`。
  - 工作区是 `.tmp/wp159/smoke-ws`；
  - web dist 用的是一个只有 index.html 的占位目录（`.tmp/wp159/stub-dist`），因为没放行 build；
  - 跑完已停掉 serve。
- **handshake**：rc=0，收到 server/hello，用时 13 ms。
- **list**：rc=0，session/list 返回了 sessions 数组，用时 16 ms。
- **bad-handshake**：rc=0，错误 token 时连接被拒（websocket error），没有收到 server/hello。
- 原始结果在 `.tmp/wp159/smoke-*.json`。

### 收尾
- 已执行 `git worktree remove --force .tmp/wp158-web`。另一个 worktree `.tmp/wp161-desktop` 属于 desktop 包，没动。
- 主树 web/ 的 git status：      2 ??       6 D     129 M （共 137 项）；`git diff --shortstat -- web`：135 个文件，+1194 / −4920（其中锁文件约 +856 / −4016）。desktop/ 下的改动来自 WP-160，不是本包。
- 没有 commit。

### 复跑命令（都在 web/ 下执行，用 guard.sh 监控内存）
```
G=/workspace/litecode/.tmp/wp159/guard.sh
$G tsc /exec-daemon/node node_modules/typescript/bin/tsc -b
$G vitest /exec-daemon/node node_modules/vitest/vitest.mjs run --maxWorkers=1
PATH=/exec-daemon:$PATH $G knip /workspace/litecode/.tmp/wp159-tools/node_modules/.bin/knip --reporter json
WS_URL=ws://127.0.0.1:17483/ws /exec-daemon/node scripts/ws-smoke.mjs handshake|list|bad-handshake --token smoke-tok
```
