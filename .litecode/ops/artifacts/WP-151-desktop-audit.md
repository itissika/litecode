# WP-151 desktop + scripts 瘦身调研（子包 WP-153 · frontend_desktop · 只读）

范围：`desktop/`（src 37 个非测试/测试 TS + scripts/copy-theme-tokens.mjs + build/）与 `scripts/`（17 个文件）。方法：仅静态分析（knip + tsc --noEmit 交叉 + rg 人工确认）；未构建、未运行 Electron、未改任何产品/依赖/锁文件、未 commit。后端报错不在范围内。

## 1. 死代码

工具原始输出（knip，配置在 /tmp，含测试入口）：
```
Unused devDependencies (1)
package.json: electron-builder
Unlisted binaries (1)
package.json: tsc, electron, electron-builder
Unused exports (9)
src/browser-host.ts: BROWSER_ID_PATTERN, normalizeBrowserBackground, sanitizeBrowserBounds
src/bundle-paths.ts: missingLinuxBundleMessage
src/components.ts: COMPONENTS_RELEASE_REPO, COMPONENTS_BASE_ENV, COMPONENT_IDS, releaseDownloadPrefix
src/dev-ui.ts: isLoopbackDevHost, sidecarDevOrigin
src/hub.ts: loadThemeTokensCss, buildHubHtml
src/instance-registry.ts: stripVerbatimLap
src/ipc-trust.ts: classifySurfaceUrl, isWorkbenchPopoutDocument
src/remote-state.ts: managedRemoteAfterSuccessfulConnect
src/ssh-session.ts: SshCommandError, scpRemoteDest
Unused exported types (4)
src/components.ts: ComponentFormat, InstallRecordEntry, ComponentLocate
src/instance-registry.ts: InstanceRecord
src/remote-progress.ts: RemoteProgressStage
src/ssh-session.ts: InstallTarProgressStage, InstallTarProgress
```
另以「仅生产入口（main.ts / preload.ts）」再跑一次（/tmp/wp153-knip-prod.txt），结果与上面相同 → **没有未用文件，也没有只被测试引用的导出**。

tsc --noEmit --noUnusedLocals --noUnusedParameters：
```
../../workspace/litecode/desktop/src/components.test.ts(375,62): error TS6133: 'init' is declared but its value is never read.
../../workspace/litecode/desktop/src/main.ts(398,24): error TS6133: 'a' is declared but its value is never read.
../../workspace/litecode/desktop/src/main.ts(703,52): error TS6133: 'workspace' is declared but its value is never read.
```

### 1.1 确认可删
| # | 路径 | 证据 | 建议 | 风险 |
|---|---|---|---|---|
| D1 | desktop/src/remote-state.ts:14 `managedRemoteAfterSuccessfulConnect` | knip 报未用导出；`rg -w managedRemoteAfterSuccessfulConnect desktop web` 只在定义处出现 1 次；函数体只是 `return next` | 删除这个函数 | 低 |
| D2 | desktop/src/main.ts:703 `scheduleTunnelReconnect(…, workspace, …)` 的 `workspace` 参数 | tsc TS6133；函数体内没有读取；唯一调用点在 :741 | 删掉这个参数，同时改调用点 | 低 |
| D3 | desktop/src/main.ts:398 `find((a, i, arr) => …)` | tsc TS6133 | 改成 `_a` 或 `indexOf` 写法 | 无（仅样式） |
| D4 | desktop/src/components.test.ts:375 `init` 参数 | tsc TS6133，测试 mock | 改成 `_init` | 无 |
| D5 | 16 个只在本文件内用的 `export`：browser-host（BROWSER_ID_PATTERN、normalizeBrowserBackground、sanitizeBrowserBounds）、bundle-paths（missingLinuxBundleMessage）、components（COMPONENTS_RELEASE_REPO、COMPONENTS_BASE_ENV、COMPONENT_IDS、releaseDownloadPrefix）、dev-ui（isLoopbackDevHost、sidecarDevOrigin）、hub（loadThemeTokensCss、buildHubHtml）、ipc-trust（classifySurfaceUrl、isWorkbenchPopoutDocument）、ssh-session（SshCommandError、scpRemoteDest） | knip；逐个跑 `rg -lw <name> desktop/src web/src`，都只命中定义文件，且文件内出现 ≥2 次（说明内部在用） | **只去掉 `export` 关键字**，不删实现 | 低；去掉后测试如果想直接测这些函数就得重新导出 |
| D6 | 7 个只在本文件内用的导出类型：components（ComponentFormat、InstallRecordEntry、ComponentLocate）、instance-registry（InstanceRecord）、remote-progress（RemoteProgressStage）、ssh-session（InstallTarProgressStage、InstallTarProgress） | 同 D5 | 同 D5 | 低 |
| D7 | instance-registry.ts:9 把 `stripVerbatimLap` 再导出 | knip；调用方都直接从 `./lap-path` 导入（rg 结果：lap-path.ts、lap-path.test.ts） | 再导出里去掉 stripVerbatimLap | 低 |

### 1.2 疑似（需要复核）
| # | 路径 | 证据 | 建议 | 风险 |
|---|---|---|---|---|
| S1 | preload.ts:197-211 `pauseComponent` / `stopComponent` / `retryComponent`，以及 main.ts:1538 起对应的 `litecode:component-pause|stop|retry` handler | 对 preload 的 71 个桥接键逐个 rg：web/src（排除 auth.ts 类型）和 hub.ts 里都**没有调用方** | 「待 Andy 定」：做 UI 接上，还是连同 handler 一起删 | 删了以后再做暂停/重试就得重新加回来 |
| S2 | preload.ts:178 `onRemoteDisconnected`，以及 main.ts:125/680 的 `emitRemoteDisconnected` | 主进程会发这个事件，但 web 和 hub 都没有订阅 → 远程断开时用户看不到任何提示 | 「待 Andy 定」：在 web 状态栏接上，或者删掉 | 现在是个静默的 UX 缺口 |
| S3 | preload.ts:219 `focusWorkspace`，以及 main.ts:1706 handler | web 里没有调用方（只在 auth.ts 类型里有声明）；多实例聚焦实际走的是 `openWorkspace` | 先确认 openWorkspace 内部是否已经覆盖这条路径，覆盖了就删 | 中：多实例行为 |
| S4 | knip 报 devDependency `electron-builder` 未使用 | **误报**：package.json 的 `dist:win`/`pack:dir` 和 scripts/package_win.ps1:40 都用 `npx electron-builder`。knip 是因为仓库里没有 node_modules 才同时报了 unlisted binaries | 保留 | — |
| S5 | scripts/package_local_cuda.ps1 | rg 只在文件自身里出现（AGENTS/README/workflows 都没有引用）；文件头写着「Personal CUDA nightly」 | 「待 Andy 定」：留作个人脚本、移到 tools/personal，还是删 | 有人手动在用 |
| S6 | 废弃的空操作参数：package_win.ps1:18（-SkipModel/-SkipLinuxBundle/-SkipPortable）、package_local_cuda.ps1:14（-SkipLinuxBundle）、assemble_product.ps1:10 | 注释原文就写着 Deprecated no-op | workflows 里 rg 不到这些参数后就可以删 | 外部旧调用会因为参数不存在而报错 |
| S7 | scripts/litecode.service.example | 只被 package_linux.sh 引用（会打进 Linux 包） | 保留 | — |

scripts 其余文件的调用方证据（`rg -l <name>`，已排除 .litecode/ops）：assemble_product.*、build_components.mjs、bundle_embed_model.sh、package_win.ps1、product_version.ps1、package_linux.sh、smoke_product.*（windows-sidecar.yml）都有 workflow 或脚本调用；dev_win/serve*/wire_watch/package_local 在 README/AGENTS 里列为手动入口；ensure_linux_bundle.ps1 被 package_win、dev_win 调用。**scripts 没有可以直接删的文件。**

依赖：desktop 的 devDeps 一共 4 个（@types/node、electron、electron-builder、typescript），全部在用；没有 dependencies。

## 2. 重复实现
| # | 分组 | 差异 | 合并目标 | 影响面 / 风险 |
|---|---|---|---|---|
| R1 | 桥接协议类型：desktop/src/preload.ts（71 个键，实现）↔ web/src/api/auth.ts `LitecodeDesktopBridge`（手写镜像） | web 那边全是可选字段，而且已经漂移：S1-S3 的键只在类型里有、没人用；recents 的返回类型在 auth.ts 里内联重复了 3 次 | 抽一个共享的 `bridge-contract.d.ts`（desktop 生成或放在共享目录），web 和 preload 都 import | 中：两个 tsconfig 和构建边界都要动；跨端，由中层汇总 |
| R2 | 组件进度类型：components.ts `ComponentProgressEvent`/`ComponentPhase` ↔ web/src/api/auth.ts `DesktopComponentProgress` | web 的版本少了 status、percent、bytesPerSecond，phase 字面量是复制过去的 | 并进 R1 的共享契约 | 低到中 |
| R3 | 百分比计算：components.ts 已经发出 `percent` ↔ web EmbedModelDownload.tsx:95 `percent()` 又自己算了一遍 | 取整方式一样 | web 直接用 `event.percent`，没有时再回退 | 低 |
| R4 | 进度阶段类型三套：ssh-session `InstallTarProgressStage`、remote-progress `RemoteProgressStage`、components `ComponentPhase`，由 remote-progress 映射到同一条 hub 进度条 | 用途不同，映射已经集中在 remote-progress.ts | 不合并，只把类型去掉 export（D6） | — |
| R5 | 版本号读取三处：product_version.ps1（Get-LitecodeProductVersion）、package_linux.sh:14（sed）、build_components.mjs:53 | 正则各不相同；sh 只取第一行 `version =`，Cargo.toml 结构变了就可能取错 | 统一成一个 `node scripts/build_components.mjs --print-version` 式的入口，或者共用同一个正则 | 低到中：发布版本号 |
| R6 | sh/ps1 成对：assemble_product（151/167 行）、smoke_product（102/136）、serve.sh/serve_win.ps1（334/272） | 逻辑镜像，WP-066 的 remap 也是两边各写一遍 | 不建议合并成一份（平台原生更稳）；建议在文件头互相标注「改一处必须同步另一处」，并在 slim-packaging.test.ts 里加 sh/ps1 一致性断言 | 低 |
| R7 | ps1 小工具函数重复：Test-Command（dev_win.ps1:48、serve_win.ps1:80）；Assert-LastExitCode / $LASTEXITCODE 检查（package_win 7 处、package_local 3 处、assemble 1 处） | 实现基本一样 | 抽到 `scripts/lib/common.ps1` 再 dot-source | 低；CI 路径要回归 |
| R8 | 「打开工作区」文案和流程：hub 内联 JS 的 `message("Opening workspace…")` ↔ main.ts emitRemoteProgress 的 "Opening workspace…"（:928、:1121） | 同一句话在两层各出现一次 | 见第 3 节 C4 | 低 |

## 3. 多余提示文案（仅有界面部分：Hub 窗口、Open Remote 向导、dialog.show*、下载进度）
NSIS：package.json 里 `nsis` 只有选项，没有自定义 .nsh 或文案（用的是 electron-builder 默认文案）→ 不涉及。托盘/菜单：rg `new Tray|Menu.buildFromTemplate` 无结果 → 不涉及。

| # | 位置 | 原文 | 建议文 | 类型 | 风险 |
|---|---|---|---|---|---|
| C1 | hub.ts 首页 lead + 空列表 :568 | "Open a local folder, or connect a remote machine over SSH."（lead）以及 "No workspaces yet. Open a local folder, or connect a remote machine over SSH." | 去掉 lead，或者把空列表改成 "No workspaces yet." | 可合并 | 低 |
| C2 | hub.ts 远程页 | 标题 "Open remote" + lead "Connect with user@host, deploy Litecode automatically, then choose a workspace." | lead 删掉（下面的 1-4 步已经说明了流程） | 可删 | 低 |
| C3 | Token 步骤：hub lead、hub message、main.ts:1105 | "Generated automatically for this session. You do not need to type it." / "Token generated automatically. Copy if needed, then enter the workspace." / "Remote session ready. Review the token, then enter the workspace." | 只保留一句："Session ready. Token is filled in automatically." | 可合并（3→1） | 低；「待 Andy 定」：整个第 4 步要不要直接跳过 |
| C4 | main.ts:928/1121 + hub | "Opening workspace…" ×3 | 只在主进程发一次 | 可合并 | 低 |
| C5 | main.ts:1038 + hub | "Choose a workspace folder." / "Connected. Choose a workspace folder." / "Choose a workspace folder below the remote home." | 进度用 "Connected."；校验错误用 "Pick a folder inside your home directory." | 可合并/可缩短 | 低 |
| C6 | hub | "Select a folder, or double-click to enter it." | "Select a folder first." | 可缩短 | 低 |
| C7 | hub 第 2 步 | 静态 "Preparing…" 加 JS 写入的 "Starting…"（:401） | 只保留一个 | 可合并 | 无 |
| C8 | main.ts:556-558 主机指纹对话框 | title "Verify SSH host identity" / message "First connection to ${key}" / detail "Verify this fingerprint with the host administrator before continuing:\n…" | detail 改成 "Fingerprint:\n…"（title 已经说了要校验） | 可缩短 | 低；安全提示措辞要谨慎，「待 Andy 定」 |
| C9 | main.ts:775-777 sidecar 崩溃对话框 | detail "code=… signal=…" | 保留（排障用）；message 可以缩短为 "The local server stopped and could not restart." | 可缩短 | 低 |
| C10 | main.ts:827 | "Embed model unavailable — remote code search will be off. Continuing…" | "Code search unavailable on remote (no embed model)." | 可缩短 | 低 |
| C11 | ssh-session.ts 部署进度 6 句（Checking… / Using existing… / Uploading… / Verifying… / Extracting… / ready） | 都是阶段提示 | 保留；"Extracting server under remote home…" 可以缩短为 "Extracting…" | 可缩短 | 无 |

## 4. 功能介绍大纲（「?」弹框用，只收桌面壳有界面的交互）
1. **工作区首页（Hub）**：打开本地文件夹或远程机器，最近列表可以置顶、移除、一键重连。入口：启动窗口的 Local / Remote 按钮和列表。
2. **远程打开（SSH）**：输入 user@host，可以选密码、私钥或 SSH agent，自动部署服务端后浏览并选择远程目录。入口：Hub → Remote（4 步向导）。
3. **主机指纹确认**：第一次连接时弹出指纹，确认后才继续。入口：连接时的系统对话框。
4. **会话令牌**：自动生成，可以复制后进入工作区。入口：向导第 4 步 Copy token / Enter workspace。
5. **无边框窗口控制**：最小化、最大化/还原、关闭。入口：Hub 和工作台标题栏（web TitleBar 经桥接调用）。
6. **内嵌浏览器面板与弹出窗口置顶**：在工作台里开网页，弹出窗口可以置顶。入口：浏览器面板头部按钮，弹出窗口 chrome 的 pin。
7. **按需组件下载**：嵌入模型首次使用时下载，显示进度。入口：设置 → 引擎 → 下载卡片（web 面板，桌面端提供能力）。暂停/停止/重试目前**没有界面**（S1）。
8. **主题跟随与拖入文件**：主题和宿主同步；把文件拖进窗口时能拿到真实路径。入口：设置主题、拖放。

## 5. 硬编码文案统计
口径：
- 扫描范围：`desktop/src/*.ts`，排除 `*.test.ts`。用 TypeScript AST（/tmp/audit 里的 typescript）解析。
- 只认用户可见的上下文：对象属性 `title/message/detail/buttons/label/placeholder…`（对话框、进度事件）；`dialog.show*`、`showErrorBox`、`emitRemoteProgress` 的实参；对 `.textContent/.title/.placeholder` 的赋值；Hub 的 HTML 模板（文本节点和 title/placeholder/aria-label，已剔除 style/script）；Hub 内联客户端 JS（textContent、title/aria 赋值、`message("…")`，带 `true` 参数的算错误）。
- 分类：`error`（属性名是 error、showErrorBox、带错误标志，或命中失败类关键词正则）；`hint`（detail、placeholder、title、aria-label 等 tooltip 类）；其余算 `ui`。
- 排除：console.*、import、标识符/路径/ID 形态的字符串（例如 `user@192.168.1.10` 这样的示例占位）、模板插值部分（只算字面量）。`new Error(...)` 单独列为 throw，**不计入总量**：大多数是内部错误，少数会透传到 hub 状态栏，做多语言时需要再逐条看。
- 条数按出现次数计（不去重，例如 "Cancel" 算 3 条）。字符量 = 压缩空白后的字面量长度。

结果：
| file | UI n/chars | hint+tooltip n/chars | error n/chars | (throw, excluded) n/chars |
|---|---|---|---|---|
| browser-host.ts | 1/7 | 0/0 | 0/0 | 4/125 |
| components.ts | 5/65 | 0/0 | 1/15 | 28/853 |
| dev-ui.ts | 0/0 | 0/0 | 0/0 | 9/445 |
| hub.ts | 39/768 | 14/88 | 5/175 | 1/95 |
| ipc-trust.ts | 0/0 | 0/0 | 0/0 | 2/59 |
| lifecycle-mutex.ts | 0/0 | 0/0 | 0/0 | 1/30 |
| main.ts | 12/319 | 2/83 | 4/181 | 45/1690 |
| parse-user-host.ts | 0/0 | 0/0 | 0/0 | 5/178 |
| paths.ts | 0/0 | 0/0 | 0/0 | 1/31 |
| pending-remote.ts | 0/0 | 0/0 | 0/0 | 1/48 |
| remote-targets.ts | 0/0 | 0/0 | 0/0 | 2/108 |
| secure-store.ts | 0/0 | 0/0 | 0/0 | 2/118 |
| sidecar.ts | 0/0 | 0/0 | 0/0 | 2/45 |
| ssh-credential-material.ts | 0/0 | 0/0 | 0/0 | 4/175 |
| ssh-session.ts | 6/196 | 0/0 | 0/0 | 32/1289 |
| ssh-targets.ts | 0/0 | 0/0 | 0/0 | 3/79 |
| **total** | 63/1355 | 16/171 | 10/371 | 142/5368 |

合计（不含 throw）：**89 条 / 1897 字符**，约 85% 集中在 hub.ts 和 main.ts。做多语言时的估量：desktop 壳的工作量很小，可以并进 web 的 i18n 包，hub 需要先把 HTML 模板改成键值表。

复跑：
```
node /tmp/wp153-strings.mjs /workspace/litecode          # 汇总表
node /tmp/wp153-strings.mjs /workspace/litecode --list   # 逐条明细（file kind ctx text）
```
脚本全文（/tmp/wp153-strings.mjs）：
```js
// WP-153 hardcoded user-visible copy counter for desktop/src (read-only).
// Usage: node /tmp/wp153-strings.mjs [repoRoot] [--list]
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
const require = createRequire("/tmp/audit/");
const ts = require("typescript");
const root = process.argv.slice(2).find((a) => !a.startsWith("--")) || "/workspace/litecode";
const LIST = process.argv.includes("--list");
const dir = path.join(root, "desktop/src");
const files = fs.readdirSync(dir).filter((f) => f.endsWith(".ts") && !f.endsWith(".test.ts"));
const UI_KEYS = new Set(["title", "message", "detail", "buttons", "label", "placeholder", "checkboxLabel", "buttonLabel", "noLink"]);
const HINT_KEYS = new Set(["detail", "placeholder", "tooltip", "hint"]);
const DOM_PROPS = new Set(["textContent", "innerText", "title", "placeholder", "value"]);
const ERR_RE = /\b(fail(ed|s)?|error|unable|could not|cannot|can't|crash(ed)?|invalid|not found|unavailable|denied|refused|timed? ?out|missing)\b/i;
// user-visible = has a 2+ letter word; drop identifier/path/id-like tokens (snake_case, dotted, slashed, camelCase, kebab).
const hasWords = (s) => {
  const t = s.trim();
  if (!/[A-Za-z\u4e00-\u9fff]{2,}/.test(t)) return false;
  if (/\s/.test(t)) return true;
  return !/[_./:#\\]|[a-z][A-Z]|^[a-z]+-[a-z-]+$/.test(t);
};
const rows = [];
function add(file, kind, text, ctx) {
  const t = text.replace(/\s+/g, " ").trim();
  if (!t || !hasWords(t)) return;
  rows.push({ file, kind, text: t, ctx, chars: t.length });
}
function classify(text, base) {
  if (base === "error" || ERR_RE.test(text)) return "error";
  return base;
}
function litText(n) {
  if (ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n)) return n.text;
  if (ts.isTemplateExpression(n)) return [n.head.text, ...n.templateSpans.map((s) => s.literal.text)].join(" ");
  return null;
}
// HTML inside template literals (hub.ts markup): text nodes + title/placeholder/aria-label attrs.
// Inline browser script embedded as a template (hub client JS): DOM text, aria/title attrs, message("...") status calls.
function scanClientJs(file, js) {
  if (!/\(function\(\)\{/.test(js)) return false;
  const q = `"([^"\\n]+)"|'([^'\\n]+)'`;
  const grab = (re, kindOf, ctx) => { for (const m of js.matchAll(re)) for (const lit of m[0].matchAll(new RegExp(q, "g"))) { const t = lit[1] ?? lit[2]; if (!/[<>]/.test(t)) add(file, kindOf(t), t, ctx); } };
  grab(/\.(textContent|innerText)\s*=\s*[^;\n]+/g, (t) => classify(t, "ui"), "js@textContent");
  grab(/(\.title\s*=|setAttribute\("(aria-label|title)",)[^;\n]+/g, (t) => classify(t, "hint"), "js@title/aria");
  for (const m of js.matchAll(/\bmessage\(([^;\n]+)\)/g)) {
    const isErr = /,\s*true\s*\)?\s*$/.test(m[1]);
    for (const lit of m[1].matchAll(new RegExp(q, "g"))) add(file, isErr ? "error" : classify(lit[1] ?? lit[2], "ui"), lit[1] ?? lit[2], "js@message");
  }
  return true;
}
function scanHtml(file, html) {
  if (!/<(div|button|label|h\d|p|span|input|section|a)\b/i.test(html)) return false;
  const noCode = html.replace(/<style[\s\S]*?<\/style>/gi, " ").replace(/<script[\s\S]*?<\/script>/gi, " ");
  for (const m of noCode.matchAll(/\b(title|placeholder|aria-label)="([^"]+)"/g)) add(file, classify(m[2], "hint"), m[2], `html@${m[1]}`);
  for (const m of noCode.matchAll(/>([^<>]+)</g)) {
    const t = m[1].replace(/\$\{[^}]*\}/g, " ");
    if (/[{};=]/.test(t)) continue;
    add(file, classify(t, "ui"), t, "html@text");
  }
  return true;
}
for (const f of files) {
  const src = fs.readFileSync(path.join(dir, f), "utf8");
  const sf = ts.createSourceFile(f, src, ts.ScriptTarget.Latest, true);
  const visit = (n) => {
    const text = litText(n);
    if (text !== null) {
      // full html blobs
      if (text.length > 200 && scanClientJs(f, text)) return;
      if (text.length > 200 && scanHtml(f, ts.isTemplateExpression(n) ? n.getText(sf) : text)) return;
      let p = n.parent;
      while (p && (ts.isArrayLiteralExpression(p) || ts.isConditionalExpression(p) || ts.isBinaryExpression(p) || ts.isParenthesizedExpression(p) || ts.isTemplateSpan(p))) p = p.parent;
      if (!p) return;
      // exclusions: logs, throws, imports, comparisons
      const call = ts.isCallExpression(p) ? p.expression.getText(sf) : "";
      if (/^console\./.test(call) || ts.isImportDeclaration(p) || ts.isExportDeclaration(p)) return;
      if (ts.isNewExpression(p) && /Error$/.test(p.expression.getText(sf))) { add(f, "throw", text, "new Error"); return; }
      if (ts.isPropertyAssignment(p) && ts.isIdentifier(p.name)) {
        const k = p.name.text;
        if (k === "error") return add(f, "error", text, "prop@error");
        if (UI_KEYS.has(k)) return add(f, classify(text, HINT_KEYS.has(k) ? "hint" : "ui"), text, `prop@${k}`);
      }
      if (ts.isCallExpression(p) && /(showErrorBox|showMessageBox|showOpenDialog|emitRemoteProgress)$/.test(call))
        return add(f, classify(text, /Error/.test(call) ? "error" : "ui"), text, `call@${call}`);
      if (ts.isBinaryExpression(n.parent) && n.parent.operatorToken.kind === ts.SyntaxKind.EqualsToken && ts.isPropertyAccessExpression(n.parent.left) && DOM_PROPS.has(n.parent.left.name.text))
        return add(f, classify(text, n.parent.left.name.text === "title" ? "hint" : "ui"), text, `dom@${n.parent.left.name.text}`);
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
}
const kinds = ["ui", "hint", "error", "throw"];
const by = {};
for (const r of rows) { by[r.file] ??= Object.fromEntries(kinds.map((k) => [k, { n: 0, c: 0 }])); by[r.file][r.kind].n++; by[r.file][r.kind].c += r.chars; }
const tot = Object.fromEntries(kinds.map((k) => [k, { n: 0, c: 0 }]));
console.log("| file | UI n/chars | hint+tooltip n/chars | error n/chars | (throw, excluded) n/chars |\n|---|---|---|---|---|");
for (const [f, v] of Object.entries(by).sort()) {
  console.log(`| ${f} | ${kinds.map((k) => (tot[k].n += v[k].n, tot[k].c += v[k].c, `${v[k].n}/${v[k].c}`)).join(" | ")} |`);
}
console.log(`| **total** | ${kinds.map((k) => `${tot[k].n}/${tot[k].c}`).join(" | ")} |`);
if (LIST) for (const r of rows) console.log(`${r.file}\t${r.kind}\t${r.ctx}\t${r.text}`);
```

## 6. 「待 Andy 定」汇总
| # | 事项 | 选项 |
|---|---|---|
| A1 | 组件暂停/停止/重试桥接（S1） | 做 UI 接上 / 删 IPC 与 preload |
| A2 | 远程断开事件没有订阅方（S2） | web 加断开提示 / 删除 |
| A3 | package_local_cuda.ps1 个人脚本（S5） | 保留 / 移出 scripts / 删 |
| A4 | 远程向导第 4 步「Session token」（C3） | 保留并缩短文案 / 自动进入，跳过这一步 |
| A5 | SSH 指纹对话框文案缩短（C8） | 保留安全提示原文 / 缩短 |

## 7. 基线与复跑
开工前（/tmp/wp153-before.txt）：
```
 M .litecode/ops/board.json
 M .litecode/ops/events/inbox.json
 M .litecode/ops/events/outbox.jsonl
 M .litecode/ops/log.jsonl
 M .litecode/ops/tool_traces.jsonl
 M .litecode/ops/tools/board.md
?? .litecode/ops/artifacts/WP-147-fill.txt
?? .litecode/ops/artifacts/WP-151-fill.txt
---
ce6e7d8595a1dfc4efc492268b2deeecb042a855f97bb39cfb1abafe10e4e115  desktop/package.json
1c551b1dc125921c7b9aa6b83665a888741b248e8eff8167579c5854591c8ae7  desktop/package-lock.json
9ff2a4c3a03c66cfe9ae6bcfc408e8583a8546b48f5605a9e2e23ffc5f232e2f  web/package-lock.json
```
结束时（/tmp/wp153-after.txt）：见下方附加内容（唯一的新增文件就是本产物）。

复跑命令原文：
```
# 工具（仅 /tmp）
ELECTRON_SKIP_BINARY_DOWNLOAD=1 npm i --prefix /tmp/audit knip typescript@5.8 @types/node@24 electron@44 --ignore-scripts --no-audit --no-fund
# knip（配置在 /tmp）
cat /tmp/audit/knip.wp153.json   # {"entry":["src/main.ts","src/preload.ts","src/*.test.ts","scripts/*.mjs"],"project":["src/**/*.ts","scripts/**/*.mjs"]}
cd /workspace/litecode/desktop && node /tmp/audit/node_modules/knip/bin/knip.js --config /tmp/audit/knip.wp153.json --no-progress --reporter compact > /tmp/wp153-knip.txt
cd /workspace/litecode/desktop && node /tmp/audit/node_modules/knip/bin/knip.js --config /tmp/audit/knip.wp153.prod.json --no-progress --reporter compact --include files,exports > /tmp/wp153-knip-prod.txt
# tsc（noEmit；tsconfig 在 /tmp，include 指向仓库）
cd /tmp/audit && node node_modules/typescript/bin/tsc -p tsconfig.wp153.json > /tmp/wp153-tsc.txt
# 文案统计
node /tmp/wp153-strings.mjs /workspace/litecode --list > /tmp/wp153-strings.txt
# 关键 rg
cd /workspace/litecode
for s in managedRemoteAfterSuccessfulConnect BROWSER_ID_PATTERN scpRemoteDest; do rg -lw $s desktop/src web/src; done
rg -n 'pauseComponent|retryComponent|stopComponent|focusWorkspace|onRemoteDisconnected' desktop/src web/src
for f in scripts/*; do b=$(basename $f); rg -l --hidden -g '!.git' -g '!.litecode/ops/**' -F "${b%.*}" .; done
rg -n 'dialog\.show|new Tray|Menu\.buildFromTemplate' desktop/src
```

```
 M .litecode/ops/board.json
 M .litecode/ops/events/inbox.json
 M .litecode/ops/events/outbox.jsonl
 M .litecode/ops/log.jsonl
 M .litecode/ops/tool_traces.jsonl
 M .litecode/ops/tools/board.md
?? .litecode/ops/artifacts/WP-147-fill.txt
?? .litecode/ops/artifacts/WP-151-desktop-audit.md
?? .litecode/ops/artifacts/WP-151-fill.txt
---
ce6e7d8595a1dfc4efc492268b2deeecb042a855f97bb39cfb1abafe10e4e115  desktop/package.json
1c551b1dc125921c7b9aa6b83665a888741b248e8eff8167579c5854591c8ae7  desktop/package-lock.json
9ff2a4c3a03c66cfe9ae6bcfc408e8583a8546b48f5605a9e2e23ffc5f232e2f  web/package-lock.json
```
