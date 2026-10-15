# WP-158 子包 ↔ 提交/文件映射（补 GM 验收第 1 项）

整理：frontend_mid · 2026-10-15 HKT · 只读整理，没有改 git 历史，也没有新提交。

## 提交
- `05ab3c4` feat(web,desktop)：**混合提交**。前端部分 = WP-158（WP-159~164）+ WP-165 线（WP-169 desktop、WP-170 web）；`src/`、`tests/` 下的 Rust 属于 WP-166/167，与 WP-158 无关。
- `6399cca` chore(scripts)：只有 scripts/ 下 14 个文件，= WP-160 的 `package_win.ps1`、`package_local_cuda.ps1`（删除）+ WP-164 R5/R6/R7 的 12 个脚本。
- WP-158 的范围之外，B1~B3 交付时都没有单独提交，所以没办法从 git 里干净地按包切出每个包的 diff。

## 各包 diff 文件（本目录）
| WP | 文件 | 性质 |
|---|---|---|
| WP-159 web B1 | `WP-159-web.diff` + `WP-159-web-deps.diff` | **精确**：施工时导出的阶段补丁（= `WP-158-patches/WP-159-web.patch`、`deps.diff`），基线是开工前的 HEAD |
| WP-160 desktop B1 | `WP-160-desktop.diff` | **精确**：阶段补丁（= `WP-158-patches/WP-160-desktop.patch`），含 scripts/package_win.ps1、package_local_cuda.ps1 |
| WP-161 desktop B2 | `WP-161-desktop.diff` | 文件范围：`git show 05ab3c4 --` bridge-contract.ts(A)、preload、main、components、browser-host、browser-view、remote-progress |
| WP-162 web B2 | `WP-162-web.diff` | 文件范围：api/auth.ts、lib/browserPanel.ts、EmbedModelDownload(.test).tsx、embedInstall.test.ts、EngineView.test.tsx、test/desktopBridge.ts(A)、BrowserHeaderAction/externalDrop/mentionDrop/browserObscure/BrowserPanel 测试、theme/tokens.css、theme/index.css、desktop/src/hub.ts（只有滚动条那段） |
| WP-163 web B3 | `WP-163-web.diff` | 文件范围：新增 9 个（FilePathHeader、compactMarkdown、useVerticalSplit、BaseTab、ScopedDefinitionEditor、useFloatingLayer、useSessionPanelBootstrap、TranscriptBody、StatusBar.test）+ 修改 22 个（含 status-bar.css） |
| WP-164 desktop B3 | `WP-164-desktop.diff` | 文件范围：05ab3c4 的 hub.ts、main.ts、slim-packaging.test.ts + 6399cca 的 12 个脚本（不含 package_win.ps1、package_local_cuda.ps1 的 WP-160 部分） |

“文件范围”的意思是：取的是这个文件在合并提交里的**全部** hunk。如果一个文件被几个包改过，这份 diff 里也会带上其他包的 hunk。归属见下一节。

## 共享文件的 hunk 归属
- `desktop/src/main.ts`：WP-160（删 component pause/stop/retry、focus-workspace handler）· WP-161（通道常量改从 bridge-contract 导入，删本地 REMOTE_DISCONNECTED_CHANNEL）· WP-164 A4（"Session ready. Token is filled in automatically." → "Session ready."）· **WP-169**（`runBootSequence` 本地/远程两个分支、closingFailedBoot、BootAbortedError）。WP-169 的部分以 `../WP-168/WP-169-desktop.patch` 为准。
- `desktop/src/hub.ts`：WP-160（C 系列文案）· WP-162（删内联滚动条 9 行）· WP-164 A4（"Use this folder" 处理里直接 enterRemoteWorkbench + 第 4 步回退文案）。
- `desktop/src/components.ts`、`preload.ts`：WP-160 删 IPC/符号，WP-161 改接契约、去掉 "stopped"。
- web 中 WP-159 和 B2/B3 都改过的文件：api/auth.ts、ModelSwitcher、StatusBar、EditToolView、ui/Dropdown、ui/Popover、CustomToolsSection、McpServersSection、EmbedModelDownload。WP-159 那部分以 `WP-159-web.diff` 为准，其余是 WP-162/163 的改动。
- web 中 WP-159 和 **WP-170** 都改过的文件：stores/connectionStore.ts、sessionStore.ts、settingsStore.ts。WP-170 的部分以 `../WP-168/WP-170-web.patch` 为准。

## 05ab3c4 里属于 WP-165 线、不算 WP-158 的前端文件
- desktop（WP-169）：boot-sequence.ts(A)、boot-sequence.test.ts(A)、sidecar.ts、sidecar.test.ts，再加上 main.ts 里上面列出的那部分。
- web（WP-170）：main.tsx、bootImports.test.ts(A)、fontsDeferred.ts(A)、lib/idle.ts(A)、lib/centreSettle.ts(A)、centreChat(.test).ts、dockview/config/lazyEditorPanel.tsx(A)、registry.ts、EditorPanel.tsx、FileTree.tsx、FileTree.settle.test.tsx(A)、connectionStore.reconnect.test.ts、settingsStore.test.ts，再加上上面 3 个 store 里的那部分。

## 尚有歧义
- `web/src/lib/knowledge/{layoutGraph,refDisplay,validate}.test.ts` 和 `web/src/test/fixtures/knowledge.ts`（由 lib/knowledge/fixture.ts 改名而来）不在任何阶段补丁的文件头里。按内容判断，是 WP-159 把 fixture 挪进 test/ 时连带改的 import，**暂归 WP-159**，但没有补丁直接证明。
- `web/package.json` / `package-lock.json` 归 WP-159（deps.diff）。lock 文件的完整变化要用 `git show 05ab3c4 -- web/package-lock.json` 看。
- WP-161~164 没有施工时导出的阶段补丁，所以 hunk 级别的归属是按各包交付记录（`../WP-158-desktop-B2/B3.md`、`../WP-158-web-B2/B3.md`）人工判定的，不是机器切分的。
