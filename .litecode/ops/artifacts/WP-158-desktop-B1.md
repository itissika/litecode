# WP-160 desktop B1 瘦身实施记录

- 包：WP-160（owner frontend_desktop，审批 frontend_mid），父包 WP-158
- 依据：WP-151-desktop-audit.md（D1–D7、S1–S7、C1–C11、A1、A3、A5）
- 改动先在 worktree `.tmp/wp158-desktop` 完成，再由中层导出补丁 `.litecode/ops/artifacts/WP-158-patches/WP-160-desktop.patch`，2026-10-15 01:4x HKT 用 `git apply` 落到主树
- **未 commit**；按中层要求本次**只做静态检查**（不跑 tsc、npm、knip、打包，重型任务窗口被前端-web 占用），tsc/单测/knip 待窗口放开后补跑

## 改动
| 项 | 处理 |
|---|---|
| D1 | 删 `remote-state.ts` 的 `managedRemoteAfterSuccessfulConnect` |
| D2 | `scheduleTunnelReconnect` 去掉未用的 `workspace` 参数，唯一调用点同步（main.ts:703/741） |
| D3/D4 | 未用参数改 `_a` / `_init` |
| D5/D6 | 16 个值、7 个类型只去掉 `export` |
| D7 | `instance-registry.ts` 再导出去掉 `stripVerbatimLap`（调用方直接从 `./lap-path` 导入） |
| A1 / S1 | 删 preload `pauseComponent/stopComponent/retryComponent`、main.ts 三个 `litecode:component-pause|stop|retry` handler、`ComponentManager.pause/stop/retry` 和只给 stop 用的 `activeParts`；测试改为「abortAll 保留 .part，下次 ensure 走 Range 续传」。保留 `abortAll`、Range 续传、无 206 回退、`.tmp` 启动清扫；`"stopped"` 状态暂留，WP-161 清掉 |
| S3 | 全仓 rg 无任何调用方（web 里的 `litecode:focus-workspace-search` 是另一个窗口事件），按中层裁定删 preload、handler、README 提及；`InstanceRegistry.tryFocusWorkspace` 仍被 openWorkspace 使用，保留。web `auth.ts:72` 手抄声明留给 WP-162 |
| S6 | 删 package_win.ps1 的 `-SkipModel/-SkipLinuxBundle/-SkipPortable` 和 cuda 脚本的 `-SkipLinuxBundle`（无调用方）；**保留** assemble_product.ps1 的 `-SkipModel`（windows-sidecar.yml:59 在用） |
| S2 / A2 | 不动，归 WP-161（契约）+ WP-163（web 断开态） |
| S4 / S7 | 保留（electron-builder 为误报；service.example 被 package_linux 使用） |
| A3 | `scripts/package_local_cuda.ps1` → `tools/local/package_local_cuda.ps1`；`$Root` 改为上两级，`product_version.ps1` 从 `$Root\scripts\` dot-source |
| A5 / C8 | 指纹对话框未动 |
| C1 | 空列表改 "No workspaces yet."（首页 lead 保留） |
| C2 | 删远程页 lead |
| C3 | 三句合一："Session ready. Token is filled in automatically."；第 4 步仍显示（跳过与否归 WP-164 A4） |
| C4 | 远程流程只由主进程发 "Opening workspace…"；本地 openLocal 保留 hub 那句（主进程该路径不发进度，中层已同意） |
| C5 | 进度 "Connected."；校验错误 "Pick a folder inside your home directory." |
| C6 | "Select a folder first." |
| C7 | 静态 "Preparing…" 去掉，保留 JS 的 "Starting…" |
| C9 | "The local server stopped and could not restart."，code/signal 详情保留 |
| C10 | "Code search unavailable on remote (no embed model)." |
| C11 | "Extracting…" |

## 主树 git status（应用后，desktop/scripts/tools/web）
```
 M desktop/README.md
 M desktop/src/{browser-host,bundle-paths,components,components.test,dev-ui,hub,instance-registry,ipc-trust,main,preload,remote-progress,remote-state,ssh-session}.ts
 D scripts/package_local_cuda.ps1
 M scripts/package_win.ps1
?? tools/
```
应用前这几个目录干净。web、desktop/package.json、锁文件零改动。

## 静态核对（rg，可复跑）
```bash
cd /workspace/litecode
rg -n 'package_local_cuda' --glob '!.litecode/**' --glob '!.tmp/**' .     # 只剩新文件自身的用法注释
for s in managedRemoteAfterSuccessfulConnect pauseComponent stopComponent retryComponent focusWorkspace component-pause component-stop component-retry 'focus-workspace"' SkipPortable; do rg -n "$s" desktop scripts tools .github --glob '!node_modules'; done   # 全部 0 命中
rg -n 'scheduleTunnelReconnect' desktop/src      # 定义 :703 + 调用 :741，参数一致
rg -n 'import[^;]*\bstripVerbatimLap\b' desktop/src web/src   # 只从 ./lap-path 导入
rg -n 'SkipModel|SkipLinuxBundle' scripts/package_win.ps1 .github/workflows   # 只剩 windows-sidecar.yml:59 对 assemble 的 -SkipModel
```
结论：去掉 export 的 23 个名字没有被别的文件导入；删除的符号和 IPC 通道全仓无残留引用；搬走的脚本没有断掉的引用。

## 待补
- 重型窗口放开后补跑：desktop `tsc --noEmit`、`npm test`、knip（对比 WP-151 /tmp 基线）。
- worktree `.tmp/wp158-desktop` 等中层确认后再删。
