# WP-168 desktop（WP-169：启动先出骨架窗口）

执行人：frontend_desktop。在主树里改，基于 WP-158 尚未提交的改动之上，没有 commit。
本包改动的文件：desktop/src/main.ts、desktop/src/sidecar.ts（中层为 WP-169 开放）、desktop/src/sidecar.test.ts；新增 desktop/src/boot-sequence.ts 和 desktop/src/boot-sequence.test.ts。hub.ts、bridge-contract.ts、preload.ts、web/、package.json 和锁文件都没有动。
补丁 `WP-169-desktop.patch` 是对 WP-169 开工前基线（/tmp/wp169-base/ 下的 main.ts、sidecar.ts、sidecar.test.ts）做的 diff，只包含本包的改动，`git apply -R --check` 已通过。

## 改动
- **S1 本地启动**
  - 拿到工作区后，立刻并行做两件事：`startSidecar` 和创建骨架窗口（`createWindow({kind:"skeleton"})`）。
  - 骨架窗口照常在 ready-to-show 时显示。
  - READY、verifySidecarAttached、attachSidecarWatch、localWorkbenchUrl 都完成后，才用 safeLoadURL 把工作台加载进**同一个窗口**。
  - sidecar.ts 没有改，READY 等待和 90s 超时的语义也不变。
- **S1b 远程启动**：同样先出骨架窗口，`waitForWorkspaceHealth` 通过后再 safeLoadURL(handshakeUrl)。
- **失败路径**：sidecar 启动、attach 校验或 health 检查任一失败，都会关掉骨架窗口（destroy），然后抛出原来的错误对象。外层的处理和现在一样：stopSidecar、弹 showErrorBox、app.exit(1)。为了不让关窗触发 window-all-closed 退出，关窗前先置 `closingFailedBoot`，window-all-closed 看到它就直接返回。
- **S2 recordRecent**：boot 路径里的调用改到工作台 loadURL 成功之后，只调一次，仍然只 console.error。用户操作触发的那几处（工作区切换、open-workspace、return-to-local）没动。不带工作区、进 hub 的分支行为不变。
- **窗口和单实例**
  - 尺寸仍是 1280×800，不恢复位置。
  - 骨架窗口创建后立即 `registry.start` 并 `setWorkspace(工作区)`，比原来更早，所以骨架期间第二次打开同一工作区会聚焦这个骨架窗口。
  - 远程模式下有 LITECODE_WORKSPACE 时，也在这时就 setWorkspace。
- **中层补充 (a) 骨架页**：内联的 data: URL，body 背景 #0a0a0a，整页 `-webkit-app-region: drag`，没有文字、按钮和脚本。
  - 加载骨架时把 trustContext 置为 null。
  - ipc-trust 只信任 hub 的 file URL 和工作台的 http origin，data: 页即使有 preload 也拿不到任何受信 IPC，何况它本身不发 IPC。
  - 主进程调用 loadURL 不触发 will-navigate，工作台随后仍走 safeLoadURL 的 G5 校验。
- **sidecar.ts：startSidecar 新增可选的 `signal?: AbortSignal`**（只用这一种机制，不加 onSpawn 回调）
  - 调用时 signal 已经 abort：直接以 `signal.reason` reject，不 spawn 子进程。
  - 等 READY 期间 abort：立即 `child.kill()`，清掉 90s 定时器，以 `signal.reason` reject。
  - 无论 READY、失败还是超时，只要先 settle 就移除 abort 监听，之后再 abort 不会误杀已经 READY 的 sidecar。
  - READY 等待、90s 超时和 /health 轮询的逻辑都没变。不传 signal 的调用方（main.ts 的 attachSidecarWatch 重启、工作区切换）行为完全一致。
  - `waitForReady` 改为导出，方便测试。
  - 理由：原来子进程在 READY 之前拿不到，骨架期间用户关窗时只能等 READY 再 stop，最坏会有一个看不见的进程挂到 90s 超时；改后关窗当场就杀掉子进程，不留孤儿，退出也不必等 READY。
- **中层补充 (b) 骨架期间用户关窗**（Alt+F4 或任务栏关闭）
  - runBootSequence 自己建一个 AbortController，把 signal 传给 startBackend，本地分支再把它传给 startSidecar。
  - 监听到 closed 后：先 `controller.abort(BootAbortedError)`，正在启动的 sidecar 当场被杀，startSidecar 也随之 reject；如果 sidecar 已过 READY，就对已有句柄调用 stopSidecar。整个启动以 `BootAbortedError` 结束。main 的 boot 捕获到它就直接 return，不弹错误框，也不 exit(1)，按正常关窗流程退出（window-all-closed → quit → before-quit）。
  - before-quit 在 stop 之前先 await 清理 Promise（`bootAbortCleanup`）。这个 Promise 现在会立刻结束，不再等 READY。
  - 如果关窗时已经过了 READY、还在做 health 检查，就直接停掉已有的句柄；stopSidecar 会标记为主动停止，所以 attachSidecarWatch 不会把它重启。
  - 远程模式没有本地进程，退出时不等待 health 轮询。

## 设计
新增 `boot-sequence.ts`，导出 `runBootSequence(deps)`，不依赖 Electron，所有依赖都由调用方注入：startBackend、createSkeleton、onBackend、prepare、loadWorkbench、stopBackend、closeSkeleton、recordRecent、onAbort。它负责：后端和骨架并行启动；后端就绪后在同一窗口里加载工作台；成功后调用一次 recordRecent；失败时关窗并原样抛错；用户关窗时中止并清理。main.ts 的本地分支和远程分支都通过它执行。

## 测试
sidecar.test.ts 新增 4 条（用 node 子进程模拟 sidecar）：
1. READY 之前 abort：子进程被杀，以 signal.reason reject。
2. 调用时已 abort：同样当场杀掉子进程、reject。
3. 不传 signal：行为不变，收到 LITECODE_READY 后 resolve。
4. 已经 READY 之后再 abort：监听已移除，子进程不被杀。

boot-sequence.test.ts 共 8 条：
1. 骨架窗口在 READY 之前就已创建；loadURL 发生在 READY 和 prepare 之后；recordRecent 在 load 之后只调用一次。
2. sidecar 启动失败：骨架被关闭，抛出的是原错误对象；没有 load，没有 recordRecent，也不按用户关窗处理。
3. attach 或 health 校验失败：骨架被关闭，原样抛错。
4. 远程分支：health 通过后，在同一窗口加载 handshake URL。
5. 远程分支：health 失败时关窗并原样抛错。
6. sidecar 还在启动时用户关窗：通过 signal 中止，子进程当场被杀，清理不等 READY 就结束，以 BootAborted 结束，不做失败处理，也不 load。
7. READY 之后、health 期间用户关窗：已运行的 sidecar 被 stop，之后也不会 load。
8. 启动成功时 signal 从未被 abort，sidecar 也不会被 stop。

## 静态核对（rg）
- `rg -n 'createWindow\(' desktop/src/main.ts`：只剩定义处、hub 分支和 createBootSkeleton，旧的 createWindow(content) url 调用已经没有了，BootContent 只剩 hub 和 skeleton 两种。
- `rg -n 'recordRecent\(' desktop/src/main.ts`：boot 里只在 runBootSequence 的 recordRecent 回调里调用；1312、1780、1817 是用户路径，没有改。
- `git apply -R --check WP-169-desktop.patch` 通过。
- `rg -n 'startSidecar\(' desktop/src`：只有 boot 本地分支传了 signal；main.ts:809（重启）和 :1302（工作区切换）没有传，行为不变。
- `git status --short desktop` 比开工前多了 `?? desktop/src/boot-sequence.ts`、`?? desktop/src/boot-sequence.test.ts`、`M desktop/src/sidecar.ts`、`M desktop/src/sidecar.test.ts`；main.ts 原本就是 M。

## 待真机验
- 骨架窗口的实际观感、从骨架切到工作台时有没有一帧闪烁（同色背景）、首屏时间、无边框窗口下整页拖动的效果，都「待真机验」。
- 骨架期间窗口没有标题栏按钮，只能用 Alt+F4 或任务栏关闭，「待真机验」。

## 待放行
- desktop tsc --noEmit、单测（1 worker，原有 128 条，加 boot-sequence 8 条和 sidecar 4 条）、knip 都属于重型任务，还没有运行，要等后端中层放行。

## 重型检查结果（后端中层放行后，串行，NODE_OPTIONS=--max-old-space-size=1536）
内存守护：/tmp/guard.sh 每秒采样 MemAvailable 到 /tmp/wp169-mem.log，低于 4718592 kB 就 kill。没有触发，最低 6707416 kB（约 6.4GiB，09:57:49）。
1. tsc --noEmit（`node /tmp/audit/node_modules/typescript/bin/tsc -p tsconfig.wp161.json`）：0 错误。
2. 单测（编译到 desktop/dist，`node --test --test-concurrency=1 dist/*.test.js`，跑完删除 dist）：140 个测试，140 通过，0 失败（49 个 suite）。140 = 原有 128 + boot-sequence 8 + sidecar 4。
3. knip（配置同 WP-164）：和 WP-164 的结果完全一致，没有新增告警，只剩 electron-builder 与 unlisted binaries 两条基线误报。
本轮没有改代码，补丁不需要重新生成。
