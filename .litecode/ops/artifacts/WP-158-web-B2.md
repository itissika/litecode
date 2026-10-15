# WP-158 web B2 交付物（WP-162）

## 批复记录（前端中层，2026-10-14 23:28）
- WP-162 计划已批；等 B1（WP-159）done 后开工，同时仍需 WP-161 done。
- Q1：hub.ts 的滚动条副本由 web（WP-162）来删，WP-164 不碰 hub.ts。
- Q2：滚动条规则放进 tokens.css。
- Q3：最小限度的 tsconfig / vite 改动在本包范围内，但必须在交付物里逐条列出。
- 护栏和执行顺序同 B1：重型任务前后都要报中层，由后端中层放行；MemAvailable 单独跑重型任务时 ≥ 4.5 GiB，其余 ≥ 6 GiB；重型任务不并行。

## 实施记录（2026-10-15 02:03 起，主树；WP-159 的改动未 commit，保持不动）
已 `wp.mjs start`。只做了轻量编辑，没跑 tsc、vitest、knip。

### 改动文件
web（主树里叠在 WP-159 改动之上）：
1. `src/api/auth.ts`（X3+X4）：
   - 删掉手抄的 `DesktopComponentId`、`DesktopComponentProgress`、`DesktopComponentStatus` 和整个 `LitecodeDesktopBridge`（约 170 行）；
   - 改为 `import type { ComponentProgressEvent, LitecodeDesktopBridge } from "../../../desktop/src/bridge-contract"` 并转出；
   - `declare global { interface Window { litecode?: LitecodeDesktopBridge } }` 保留。window.litecode 本身仍是可选的，因为纯浏览器里没有宿主；但桥接上的方法按契约都是必填。
2. `src/lib/browserPanel.ts`：web 自己手抄的 `BrowserBounds`、`BrowserPanelState` 两个类型删除，改为从契约转出（`BrowserState as BrowserPanelState`）。调用方不用改。
3. `src/dockview/panels/settings/engines/EmbedModelDownload.tsx`（X5）：
   - 删掉 `percent()`；运行中的状态改为 `{ message, percent }`，只读 `event.percent`；
   - 契约规定 0 表示还没开始或大小未知，这时显示原来的脉冲条，大于 0 时显示百分比；
   - 类型名 `DesktopComponentProgress` 改为 `ComponentProgressEvent`。
4. 测试（只改 mock 和断言，没删测试）：
   - EmbedModelDownload.test.tsx、embedInstall.test.ts：类型改名，所有 emit 补上必填的 `percent`（下载中 25，其余 0）；
   - 新增 `src/test/desktopBridge.ts` 的 `stubDesktopBridge(partial)`：契约里方法都是必填，测试只想 stub 用到的几个，就用它包一层；
   - 用它包装了 8 个测试文件里的 15 处 `window.litecode = {…}`：BrowserHeaderAction、externalDrop、mentionDrop、browserObscure、BrowserPanel、EngineView、embedInstall、EmbedModelDownload。
5. `src/theme/tokens.css`（X1+X2）：
   - 文件头加了说明：desktop 的 copy-theme-tokens.mjs 和 hub.ts 会直接读取，这是唯一源，不能挪、不能改名；
   - 文件末尾收进滚动条规则：`*` 的 scrollbar-width/color，以及 `::-webkit-scrollbar` / track / thumb / thumb:hover / corner。
6. `src/theme/index.css`：删掉原来那份滚动条规则（31 行）。

desktop（唯一允许碰的文件）：
7. `desktop/src/hub.ts`（X2）：删掉内联的滚动条规则（含注释共 9 行）。hub 页面已经通过 `${tokens}` 内联整份 tokens.css，所以滚动条样式改由 tokens.css 提供。`bridge-contract.ts` 只读，没改。

### tsconfig / vite 改动：无
- web 用相对路径的 `import type` 引用 `../../../desktop/src/bridge-contract`。tsconfig.app.json 是 `include: ["src"]`、`noEmit`、没有 rootDir 和 composite 限制，TS 会自动把被引用的文件纳入检查，所以不用改。
- `verbatimModuleSyntax` 下 `import type` 编译时会被擦除，vite 和 vitest 打包里不会有 desktop 代码，也就不用改 vite 配置。
- 这一点等 tsc -b 验证；如果 tsc 报 include 相关错误，再按最小改动补，并列入本节。

### 疑问
- Q1：WP-161 预期 desktop knip 报的 4 个未用类型（RemoteProgressStage、SessionMode、UiTheme、Unsubscribe）会消失。但 web 现在只直接 import 了 LitecodeDesktopBridge、ComponentProgressEvent、BrowserBounds、BrowserState，这 4 个只是在契约内部被 LitecodeDesktopBridge 等用到，并没有被 desktop 或 web 直接 import。desktop 的 knip 只扫 desktop 工作区，看不到 web 的引用。因此预计这 4 条**不会**消失。要清掉，要么由 desktop 去掉这 4 个类型的 export，要么让 knip 覆盖 web 和 desktop 两边。请中层定。
- Q2：A2 的断开态按契约只有 onRemoteDisconnected 置位、reload 清除，没有 reconnected 事件。这和 WP-163 批复 Q2 里「重连成功后恢复」需要对齐，属于 B3 的事，这里先记一笔。
- Q3：X5 之后，下载刚开始（percent 为 0）时显示的是脉冲条，不显示 0%，这和原来未知大小时的表现一致。若要显示 0%，需要中层确认。

### 重负载顺序（等后端中层放行）
1. tsc -b；
2. 全量 vitest `--maxWorkers=1`；
3. knip：web 用 `.tmp/wp159-tools`，desktop 按 WP-161 的配置跑。

## WP-162 验证结果（严格串行，guard.sh 守护，阈值 4.5GiB，全程未触发 kill）
| 步骤 | 命令 | 结果 | 最低 MemAvailable |
|---|---|---|---|
| 1 web tsc -b（首跑） | `tsc -b` | rc=2：测试 mock 有 6 处类型错（ComponentStatus 缺 status；ensureComponent 的 id 要是字面量） | 5.04GiB |
| 1 web tsc -b（修后） | 同上 | rc=0 | 5.03GiB |
| 2 web vitest 全量 | `vitest run --maxWorkers=1` | 176 个文件 / 1363 条全部通过 | 4.80GiB |
| 3 web knip | `.tmp/wp159-tools/.../knip --reporter json` | 导出 23 / 类型 21 / 文件 1 / devDep 0 / 未声明 0，与 WP-159 的 23/21/1/0/0 相同，没有一类上升 | 5.43GiB |
| 4 desktop tsc --noEmit（只读） | `cd /tmp/audit && tsc -p tsconfig.wp161.json` | rc=0 | 5.47GiB |
| 5 desktop knip（WP-161 配置，只读） | 见下 | 只剩 electron-builder（devDep）和 unlisted binaries（tsc/electron/electron-builder），与基线相同，WP-151 S4 已判为误报。**RemoteProgressStage、SessionMode、UiTheme、Unsubscribe 已不再报告** | 5.63GiB |

tsc 失败的修法：只改了测试 mock 的类型（EmbedModelDownload.test.tsx、EngineView.test.tsx 加 `as const` 和 `status` 字段），没删测试，没改 tsconfig 或 vite，也没改 desktop。

## 裁定记录
- percent=0 时显示 pulse 条，和旧版“大小未知”的表现一致。
- desktop 的 4 个类型由 desktop 去掉 export；knip 两边各扫各的。
- B3 断连态：收到 remote-disconnected 时置位，重连后整窗 reload 时清除。不自制重连检测，不加恢复事件（同步记录在 WP-158-web-B3.md）。

## git status（只看 web/，未 commit）
- 状态：      3 ??       6 D     140 M 
-  146 files changed, 1308 insertions(+), 5206 deletions(-)
- desktop/ 下的改动来自 WP-160/161，加上 WP-162 已批的 hub.ts 滚动条删除（本轮没再改）。

## 复跑命令
```
G=/workspace/litecode/.tmp/wp159/guard.sh
cd web && $G b2-tsc /exec-daemon/node node_modules/typescript/bin/tsc -b
cd web && $G b2-vitest /exec-daemon/node node_modules/vitest/vitest.mjs run --maxWorkers=1
cd web && PATH=/exec-daemon:$PATH $G b2-knip /workspace/litecode/.tmp/wp159-tools/node_modules/.bin/knip --reporter json
cd /tmp/audit && $G b2-desktop-tsc /exec-daemon/node node_modules/typescript/bin/tsc -p tsconfig.wp161.json
cd desktop && $G b2-desktop-knip /exec-daemon/node /tmp/audit/node_modules/knip/bin/knip.js --config /tmp/audit/knip.wp161.json --no-progress --reporter compact
```
日志在 .tmp/wp159/b2-*.log。
