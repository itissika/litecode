# WP-158 前端瘦身合并报告

父包 WP-158 · 执行 frontend_web / frontend_desktop · 审批 frontend_mid  
日期：2026-10-15 · 已提交：`05ab3c4`（web/desktop，和 WP-165 线混在一起）+ `6399cca`（scripts）

## 总览与包状态

| 包 | 侧 | 批次 | 内容 | 状态 |
|---|---|---|---|---|
| WP-159 | web | B1 | 删死代码 / 去 export / 依赖剪除 / U5 文案 / ws-smoke | **done** |
| WP-160 | desktop | B1 | 删死符号与 IPC / 文案 C1–C11 / 脚本搬家 | **done** |
| WP-161 | desktop | B2 | `bridge-contract.ts` 契约 | **done** |
| WP-162 | web | B2 | 接契约 + X1/X2（tokens 滚动条）+ X5 percent | **done** |
| WP-163 | web | B3 | 9 组重复合并 + A2 断开态 | **done** |
| WP-164 | desktop | B3 | R4–R8 + A4 向导跳步 + 两处 CI 修补 | **done** |

六包均已验收关闭。主树改动叠在一起，无一提交。

## 各批改动摘要

### B1 · WP-159（web）
- 删 5 文件与一批未用导出；U4 七项类型与 `compact_started` 分支；auth 帧改为握手 URL。
- 209 个文件内符号批量去 `export`（测试直接引用的保留）。
- 依赖：去 remotion 等；`dockview` 入 dependencies，`@tanstack/virtual-core` 入 devDependencies；锁文件剪 257 条 remotion 相关。
- U5/§3 文案与同类漏项；ws-smoke 对齐 JSON-RPC（handshake / list / bad-handshake）。
- knowledge fixture 搬家后补 3 处测试 import；AgentsSection 断言跟 U5。

### B1 · WP-160（desktop）
- D1–D7：删未用符号/参数/export；A1/S1 删 pause/stop/retry 组件 IPC；S3 删 focus-workspace；S6 删无用 Skip*；A3 脚本迁 `tools/local/`。
- C1–C11 文案收敛。本批仅静态 rg；tsc/单测/knip 在后续批补跑。

### B2 · WP-161（desktop 契约）
- 新建 `desktop/src/bridge-contract.ts`（`.ts` 非 `.d.ts`）：`export type` + 4 个通道 `export const`。
- preload 因 sandbox 不能运行时 import 常量，通道名仍为字面量，类型用 `typeof` 锁定。
- `percent` 必填；去掉 `"stopped"`。无 `remote-reconnected`（见裁定）。

### B2 · WP-162（web 契约）
- `auth.ts` / `browserPanel.ts` 删手抄声明，`import type` 契约；删 `percent()`，0 显示脉冲条。
- 滚动条规则收进 `tokens.css`，删 `index.css` 与 hub 内联副本。
- 测试 mock 用 `stubDesktopBridge()`；未改 tsconfig/vite。

### B3 · WP-163（web 合并 + A2）
- 9 组合并：FilePathHeader、compactMarkdown、**useVerticalSplit**、Select 导出、BaseTab、ScopedDefinitionEditor、useFloatingLayer、useSessionPanelBootstrap、TranscriptBody（**部分**）。
- W-R10 跳过（外壳几行，硬抽加参）。
- A2：只接 `onRemoteDisconnected`，`.status-bar-root[data-disconnected]` 顶边 `--_dk-red-500`、条带文字 muted；无清除逻辑，靠 reload。
- 提示文案与 WP-159 交付后逐字一致（ScopeHint `bindFirst`）。

### B3 · WP-164（desktop R4–R8 + A4）
- R5：`build_components.mjs --print-version`，版本三角均为 0.2.4。
- R6：成对 sh/ps1 互标 + 一致性测试；R7：`scripts/lib/common.ps1`。
- R4/R8 只核对。
- A4：令牌填好后直接 `enterRemoteWorkbench`；失败回退第 4 步。进度「Session ready.」→「Opening workspace…」。
- CI 补修：release「核对 tag」挪到 Setup Node 之后；`windows-sidecar.yml` paths 加 `scripts/lib/**`。

## 验证数字

### web（最终以 WP-163 为准）
| 项 | 结果 |
|---|---|
| tsc -b | 0 |
| vitest `--maxWorkers=1` | **177 文件 / 1365 条全绿**（内存护栏：默认与 =2 曾跌破 4.5GiB 被杀；站规以后全量须 `--maxWorkers=1`，约 4 分钟） |
| knip | **23 / 21 / 1 / 0 / 0**（导出/类型/文件/devDep/未声明） |
| WP-151 基线 | 145 / 101 / 6 / 3 / 2 |
| ws-smoke | B1 三子命令通过（现成二进制，未编后端） |

B1/B2 当时为 176/1363；B3 因新增 StatusBar.test（+1 文件、+2 条）变为 177/1365。

### desktop（最终以 WP-164 为准）
| 项 | 结果 |
|---|---|
| tsc --noEmit | 0 |
| 单测 concurrency=1 | **128 / 128**（B2 为 127；+1 为 R6 成对断言） |
| knip | 无新增告警；余 electron-builder / unlisted binaries 为 WP-151 已判误报 |

## 过程裁定

1. **无 `remote-reconnected`**：断开态仅由 `litecode:remote-disconnected` 置上，重连成功整窗 reload 清除。
2. **契约是 `.ts` 不是 `.d.ts`**（通道常量需运行时值）。
3. **4 个类型去 export**（RemoteProgressStage / SessionMode / UiTheme / Unsubscribe）；knip 两端各扫各的。
4. **percent=0** 显示脉冲条（同原「大小未知」）。
5. **W-R9** 落点 `lib/useVerticalSplit.ts`；**W-R3** 部分合并（不做单一 readOnly 组件）；**W-R10** 跳过。
6. **提示文案**与 WP-159 交付后一致，不回退到更早 HEAD。
7. **jscpd 未跑**（非验收项；不装不跑）；重复按 9 组清单 rg 核对，无遗漏。
8. **两处 CI 修补**见 WP-164（tag 核对顺序 + sidecar paths）。

## 风险

1. **ps1 / yaml 仅静态核对**：箱内无 pwsh、无 yaml 库，未实跑；需首次 GitHub CI 确认（尤其 release tag 核对与 sidecar 触发）。
2. **sidecar 进度文案变更**：main 侧「Session ready. Token is filled in automatically.」缩短为「Session ready.」，紧接「Opening workspace…」——属 A4 过渡提示，界面文案有可见变化。**GM 已批准**（去掉 “Token is filled in automatically.”）。

## 遗留

1. **FileTreeContextMenu** 与 **SessionStatusLine** 各自有「点外部关闭」逻辑；W-R5 只覆盖 Dropdown/Popover，本次未扩范围。
2. **U6**（中文统一英文）不在本包，留给多语言包。

## 提交

前端改动已经进了两个提交（用 `git log` / `git show --stat` 核对过）：
- `05ab3c4` feat(web,desktop)：WP-159~164 的 web/、desktop/ 改动。**同一个提交里还混着 WP-165 线**（WP-169 desktop 启动排位、WP-170 web 启动排位）和 WP-166/167 的 Rust（src/、tests/）。
- `6399cca` chore(scripts)：scripts/ 下 14 个文件，包括 WP-160 的 package_win.ps1、package_local_cuda.ps1（删除），以及 WP-164 R5–R7 的 12 个脚本。

各子包的 diff 和文件/hunk 归属见 `WP-158/WP-158-file-map.md`，各 `WP-15x/16x-*.diff` 也在同一目录。GM 提醒过：以后不同的包要分开提交，这样才能单独回退。
