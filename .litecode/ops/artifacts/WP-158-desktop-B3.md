# WP-158 desktop B3（WP-164）

执行人：frontend_desktop。在主树 /workspace/litecode 上做，只改了 desktop/ 和 scripts/。没有 commit，没有动 web/、package.json 和锁文件。hub.ts 只改了 A4 相关的两处。

## 逐项改动
- **R4（只核对）**：三套阶段类型都不再单独导出。ssh-session.ts:46 `type InstallTarProgressStage` 没有 export；bridge-contract.ts:43 `type RemoteProgressStage` 没有 export；`ComponentPhase` 属于契约公开类型，保留导出。没有改代码。
- **R5 统一读版本号**：
  - `scripts/build_components.mjs` 新增 `--print-version`，打印版本后直接退出。`cargoVersion()` 改为只认 `[package]` 表里的 `version`，修掉了原来「取第一行 version =」的隐患。
  - `scripts/package_linux.sh` 原来用 sed 取版本，改为 `node scripts/build_components.mjs --print-version`。环境变量 LITECODE_VERSION 仍然优先。
  - `scripts/product_version.ps1` 的 `Get-LitecodeProductVersion` 改为调用同一个入口，退出码非 0 或输出为空时 throw。
- **R6 sh/ps1 成对**：assemble_product、smoke_product、serve.sh/serve_win.ps1 这三对文件，文件头都加了一行 `# Paired with scripts/<另一份>: change one, keep the other in sync (WP-164 R6).`。`desktop/src/slim-packaging.test.ts` 新增一条断言，检查两点：三对文件互相标注了对方；assemble 两份都包含 `--remap-path-prefix=`、`=/rustc/registry`、`=/rustc/git`、`RUSTFLAGS (release remap)`。
- **R7 公共函数**：新建 `scripts/lib/common.ps1`，放 `Test-Command` 和 `Assert-LastExitCode`。
  - dot-source 它的脚本：dev_win、serve_win、package_win、package_local、assemble_product。
  - 删掉的重复定义：dev_win、serve_win 里的 Test-Command，package_win 里的 Assert-LastExitCode。
  - 改用 Assert-LastExitCode 的地方：assemble_product 的 cargo build、package_local 的 WSL 命令和 build_components。报错文案和原来一样，都是「X failed with exit code N」。
  - package_win 里 Invoke-ElectronBuilder 的重试逻辑要读 $LASTEXITCODE 的值，所以保留原样。
  - product_version.ps1 不依赖 common.ps1，因为 CI 只单独 dot-source 它。
- **R8（只核对）**："Opening workspace…" 还剩 3 处：hub.ts:342 是本地打开（按 B1 约定保留）；main.ts:929 是重连流程，:1122 是 enterRemoteWorkbench，两者属于不同流程，各发一次，没有重复。没有改。
- **A4 跳过第 4 步**：
  - hub.ts 的「Use this folder」处理函数：completeRemoteSession 返回后写入 token，然后直接调用 `enterRemoteWorkbench` 进入工作区。只有 token 为空或进入失败时，才回退显示第 4 步并给出错误提示，用户可以手动「Copy token / Enter workspace」。
  - 第 4 步面板的说明文字改为回退提示 "Could not open the workspace automatically. Copy the token or try again."。
  - hub.ts 其他地方没有动。
  - main.ts 的进度文案 "Session ready. Token is filled in automatically." 缩短为过渡提示 "Session ready."（ratio 0.96），紧接着是 "Opening workspace…"。

## 版本三角核对（实测）
- `node scripts/build_components.mjs --print-version` 输出 0.2.4（package_linux.sh 和 product_version.ps1 走同一个入口）
- desktop/package.json 的 version 是 0.2.4
- `git describe --tags --abbrev=0` 得到 v0.2.4
三者一致。

## 静态证据
- `rg -n 'type (InstallTarProgressStage|RemoteProgressStage|ComponentPhase)\b' desktop/src`
- `rg -n 'Opening workspace' desktop/src`：共 3 处，见 R8。
- `rg -n 'common.ps1|Test-Command|Assert-LastExitCode|LASTEXITCODE' scripts`：函数定义只在 lib/common.ps1；剩下的 $LASTEXITCODE 只在 package_win 的重试逻辑和 product_version.ps1 里。
- `rg -n 'Token is filled|Session ready|step-token' desktop scripts web/src`：没有其他引用。
- `bash -n scripts/*.sh` 全部通过；`node --check scripts/build_components.mjs` 通过。

## CI 影响（箱内没有 pwsh，只做了静态核对）
- workflows 里引用的路径（scripts/package_win.ps1、assemble_product.ps1 -SkipModel、smoke_product.ps1、product_version.ps1、build_components.mjs、package_linux.sh）都还在，有效。
- windows-sidecar.yml 的 paths 过滤只写了 `scripts/assemble_product.*` 和 `scripts/smoke_product.*`。以后只改 `scripts/lib/common.ps1` 不会触发这个 CI。建议补上 `scripts/lib/**`，但 workflow 不在本包范围内，没有改。
- signed-release 的「Check tag matches product version」这一步在 setup-node 之前就运行，现在要调用 node。GitHub 的 windows-latest 镜像预装了 node，可以用，但建议跑一次 CI 回归确认。

## git status --short desktop scripts
- 之前：WP-160 和 WP-161 的改动（desktop 下 15 个 M，加上 ?? bridge-contract.ts；scripts 下 D package_local_cuda.ps1、M package_win.ps1）。
- 之后新增：M desktop/src/slim-packaging.test.ts；M scripts/{assemble_product.ps1,assemble_product.sh,build_components.mjs,dev_win.ps1,package_linux.sh,package_local.ps1,product_version.ps1,serve.sh,serve_win.ps1,smoke_product.ps1,smoke_product.sh}；?? scripts/lib/。hub.ts、main.ts、package_win.ps1 原本就是 M，本包在上面继续改。

## 重型检查（后端中层 10-15 放行，串行，单项运行）
内存守护：/tmp/guard.sh 每秒采样 MemAvailable，记录在 /tmp/wp164-mem.log，低于 4.5GiB 就 kill。全程没有触发，最低 5839328 kB（约 5.57GiB，02:16:11）。命令和 WP-161 相同（工具在 /tmp/audit）。
1. tsc --noEmit：0 错误，第一次就通过，没有修改。
2. 单测（`node --test --test-concurrency=1 dist/*.test.js`，编译到 desktop/dist，跑完已删除）：128 个测试，128 个通过，0 个失败。比 WP-161 多出的 1 个是新增的 R6 断言「sh/ps1 script pairs …」，它也通过了。
3. knip（配置和 WP-151、WP-161 相同）：和 WP-161 的结果比，没有新增告警。剩下的两项是 electron-builder 和 unlisted binaries，都是基线里的误报。WP-161 当时报的 bridge-contract 里 4 个未用导出类型这次没有出现，原因是这 4 个类型现在已经不带 export 了。这处改动不是本包做的，应该是 WP-161 之后别人改的。
本轮没有需要修的问题。

## 退回后补修（前端中层 10-15 02:17 HKT）
- windows-signed-release.yml：把「Check tag matches product version」整段原样移到 Setup Node（node 22）之后，不再依赖 runner 自带的 node。
- windows-sidecar.yml：push 和 pull_request 的 paths 都加了 `scripts/lib/**`。
- 其它 workflow：windows-desktop.yml 只能手动触发（workflow_dispatch），没有 paths；signed-release 由 release/tag 触发，也没有 paths。调用 common.ps1 的脚本里只有 assemble_product.ps1 进了 sidecar 的触发路径，现在已经覆盖。
- 静态核对：只是挪位置，缩进和内容都没变（见 git diff）；箱内没有 pwsh，也没有 yaml 库，没做实际运行。
