# WP-021 前端产物（WP-023：WP-029 web / WP-030 desktop）

## web

- 包：WP-029（owner frontend，审批 frontend_mid）。范围 `web/src`，只读：不改产品代码和测试，不 commit，不碰引擎 UI 和 symlink 复制。
- 来源：复用 `WP-018-frontend.md` 里属于 web 的内容（F1–F8、R2、T 清单），2026-10-12 15:10 HKT 逐条按当前代码复核行号后迁入；WP-018 原文件不动。desktop 相关的 F9–F11、C1、T8 不迁，归 WP-030。
- 基线：`cd web && npx vitest run src/api/` → 13 files / **68 pass / 0 fail**（本次复跑）。

### 1. 可疑不稳实现（9 条可疑 + 3 条核查无问题）

| # | 类别 | 证据（文件:行号） | 问题 | 风险 | 置信度 | 建议修法 |
|---|---|---|---|---|---|---|
| W1 | 重连 / 在途 RPC | `web/src/api/agentWs.ts:77`（pendingRpc）、`:103`（connect 换代 generation+=1）、`:165-189`（onclose）、`:243`（disconnect 换代）、`:282-285`（30s 超时） | 换代或断线时不 reject `pendingRpc`，旧 socket 上的请求只能等 30s 超时 | 断线后 permission / revert / set-model 静默挂 30s；串行 await 链整体卡住 | 高 | onclose / connect / disconnect 时遍历 pendingRpc 全部 reject("WebSocket closed") 并清空；补单测 |
| W2 | 重连退避 | `agentWs.ts:133-137`（onopen 清零 reconnectAttempt + 置 connected）vs `:415-418`（server/hello） | 在 hello 之前就把退避清零并显示已连接 | 服务端在 upgrade 后、hello 前断开（崩溃循环、代理）时，以 base 延迟紧循环重连；UI 闪 connected | 中高 | 清零和 connected 状态挪到收到 server/hello 时 |
| W3 | RPC 发送 | `agentWs.ts:304`（sendJsonRpc 的 `ws.send` 没有 try/catch）vs `:261-268`（send 有，FE-10） | send 抛错时 promise 被拒绝，但 pendingRpc 条目和 30s 定时器泄漏 | 小泄漏；错误口径不一致 | 高 | 包 try/catch，失败时 delete 加 clearTimeout |
| W4 | 多 Ask / 单会话 | `web/src/stores/turnStore.ts:59`（pendingPermission 单槽）、`:686-711`（onPermissionRequest 直接覆盖） | 同一 session 后到的 permission_request（并行工具、子 agent）覆盖前一个，前一个 request_id 永远没有应答 | 回合挂在第一个 ask 上，表现为"卡住" | 中（取决于服务端是否保证串行 ask） | 按 request_id 排 FIFO 队列，grant 回执后出队；或在服务端契约里明确单 ask 串行，前端加 dev 断言 |
| W5 | Ask 卡片状态串号（新发现） | `web/src/dockview/panels/AgentPanel.tsx:613-625`（`<PermissionCard>` 没有 `key`）、`web/src/components/PermissionModal.tsx:231`、`web/src/components/AskUserCard.tsx:61-66`（pickedByQ / textByQ / step 是本地 state） | W4 覆盖发生时组件不重挂，上一张 ask 的勾选、自由文本、步进位置带到新卡上 | 用户把旧答案提交给新问题（答错题） | 中高 | `<PermissionCard key={pendingPermission.request_id} …>` |
| W6 | Ask 回滚复活（新发现） | `turnStore.ts:677`（`latest?.request_id === pending.request_id \|\| !latest` 时恢复卡片），对照 `:618-630`（cancel 置 pendingPermission:null）、`:632-642`（revert 置 null） | grant RPC 在途时用户 cancel 或 revert 把槽清空，之后 RPC 失败会把已取消回合的旧卡复活 | 出现一张对已结束回合的幽灵卡，点了必报错 | 中 | 回滚只在 `latest?.request_id === pending.request_id` 时恢复；或记录 `dismissedRequestIds` |
| W7 | Ask 重复提交 | `turnStore.ts:644-684`（回执前卡片可点，没有 in-flight 锁） | 双击会发两次 agent/permission | 第二次被服务端判为未知 request 并 toast 报错，或重复写 always 规则 | 中高 | slice 加 `permissionInFlight: request_id`，同一 id 在回执前忽略 |
| W8 | 取消失败静默 | `turnStore.ts:618-630`（乐观置 cancelling，`.catch(() => {})` 在 `:629`） | RPC 失败时 UI 永远停在 cancelling，违背 FE-04 禁止静默 catch | 只能刷新页面 | 高 | catch 时回滚 runState 和 pendingPermission，并 toast |
| W9 | LLM 重试双发 | `turnStore.ts:997-1009` | 没有 in-flight 锁，状态要等服务端事件才变，快速双击发两次 agent/retry | 可能重复唤醒同一 transcript | 中 | 发送前本地置 phase=retrying，失败回滚 |
| W10 | Ask 重连恢复 | `web/src/stores/sessionStore.ts:176-185`（snapshot 只回灌 turn / llm_reconnect / meter / bash） | snapshot 不带待答 ask，刷新、新窗口或重新 attach 后卡片不再出现 | 回合阻塞到用户取消为止 | 中（需确认服务端 snapshot 字段） | 服务端 snapshot 带 pending_permission，前端在 applySnapshot 时回灌（要协议域配合） |
| ✓ | 重订阅 | `web/src/stores/connectionStore.ts:160-167`（非 connected 时清空 subscribedSessions）、`:465-486`（ensureSubscribe 去重 + pendingSubscriptions 防旧请求覆盖） | 已核查，无问题；旧 handler 由 agentWs 的 generation 守卫（`agentWs.ts:131 current()`） | — | — | 无需修 |
| ✓ | knowledge 版本 / 防抖 | `web/src/stores/knowledgeStore.ts:493-499`（同 epoch 旧版本拒收）、`:539-559`（refresh 串行 + refreshAgain 合并）、`:629-641`（debounce 60ms） | 已核查：refresh 串行，不会有旧响应晚到覆盖 | — | — | 无需修 |
| ✓ | message seq 顺序 | `web/src/stores/messageStore.ts:156`（按 seq 排序）、`:212-224`（bySeq 去重）、`:370-372`（尾追加区间） | 已核查：bySeq 幂等，乱序 delta 由排序吸收，暂未发现竞态 | — | 中 | 暂不修；可补「snapshot 晚于 delta 到达」的单测 |

另外，`connectionStore.ts:496-497` 的 unsubscribe `.catch(() => {})` 是 fire-and-forget，属于可接受的静默。

覆盖缺口（只是补测建议，不是裁剪）：agentWs 缺「断线 reject pendingRpc」「hello 前断开不清零退避」两条用例；turnStore 缺「两个 permission_request 连发」「cancel 失败回滚」「grant 在途时 cancel 再失败」「retry 双击」四条用例。

### 2. R2：web death-list 中 basename 自指的弱断言

**结论：未收敛，只写了加强方案；没有改测试，落地需要审批。**

定位（5 处，行号已按当前代码复核）：
- `web/src/api/r5-death-list.test.ts:45`
- `web/src/api/r6-death-list.test.ts:54`
- `web/src/api/r9-death-list.test.ts:71`
- `web/src/api/session-seq-a-death-list.test.ts:54`
- `web/src/api/session-seq-d-death-list.test.ts:43`

5 处都是 `expect(basename(__filename)).toBe("<自身文件名>")`。

为什么弱：
1. 这条断言是同义反复，只有重命名文件才会让它失败，它守不住任何不变量，只是凭空多出 5 个"绿"。
2. 豁免规则 `name.includes("death-list")`（`r5:21`、`r6:30`、`r9:33`、`model-selection:24`、`seq-a:30`、`seq-d:19`）会静默放过任何名字里带 death-list 的产品文件。
3. 没有断言扫描到的文件数 N>0。`WEB_SRC` 一旦漂移、walk 结果为空，就会全部假绿。
4. 各门口径不一致：r5 / r6 / r9 / model-selection 会扫其他 *.test.ts，seq-a / seq-d 和 r4 / serve-auth 不扫。

加强方案：
- 抽出共享 helper `web/src/test/deathListGate.ts`，提供 `collect({includeTests:false})` 和 `scan(files, needles)`。
- 豁免改成精确正则 `/^[\w-]+-death-list\.test\.ts$/`。
- 删掉 5 条 basename 用例，换成下面四条断言：
  - ① 扫描文件数 `files.length > 50`，并且必须包含锚点 `stores/turnStore.ts`；
  - ② `FORBIDDEN.length > 0`；
  - ③ 植入 fixture 自测：往临时文件里写入 `FORBIDDEN[0]`，断言 `scan` 恰好报出 1 条；
  - ④ 扫描 roots / globs 同时覆盖 `.ts` 和 `.tsx`（断言集合里至少有一个 `.tsx`）。
- 口径统一为产品门不扫测试文件。
- 不进 slim CI 必过集（维持 WP-012 / 017 的结论）。完整代码草案见 `WP-018-frontend.md` §2.3。

### 3. web 测试「可删 / 可放宽 / 重复 / 假绿」清单（9 条，只列不删）

| # | 分类 | 测试（文件:行号） | 理由 | 建议 |
|---|---|---|---|---|
| T1 | 可删（假绿） | 5 条 basename 用例（见 §2） | 同义反复，不守不变量 | 删，换成 §2 的 non-vacuous 断言 |
| T2 | 重复 | `r9-death-list.test.ts:13` 的 FORBIDDEN_PRODUCTION 与 `r5:11`（partialText / partialReasoning）及 `r4:11` 的 tool_start / liveTools 重叠 | 同一针两道门，回归时双红噪声；R5 被 R9 整体覆盖 | 删 R5，或从 R9 去掉重叠的针，二选一 |
| T3 | 重复（可合并） | 8 份各自复制的 `walkTsFiles`：r4:24、r5:13、r6:23、r9:26、seq-a:23、seq-d:12、serve-auth:22、model-selection:17 | 实现重复导致豁免口径漂移 | 抽共享 helper |
| T4 | 弱断言 / 名不副实 | `r9-death-list.test.ts:56`「accepts shared golden Item JSON fixtures」 | 只检查 item.type 是否在枚举里，没有经过 FE adapter，证明不了前端能解析 | 加强为把 fixture 喂给 adapter 并断言投影结果，否则删除 |
| T5 | 近恒真 | `model-selection-death-list.test.ts:54` `src.includes("modelId")` | 正向子串断言几乎恒真 | 删这一行；`:56` 的 "Missing: " 可保留，或改成渲染测试 |
| T6 | 可放宽（过时） | `serve-auth-death-list.test.ts:18-19` UI 文案针 "Auth token (dev only)"、"Save auth" | 守的是已删除的文案，改个措辞就能绕过 | 去掉文案针，保留路由针和类型针 |
| T7 | 实现细节 class 断言 | `web/src/components/PermissionModal.test.tsx:259` `className toContain("w-full")` | 断言的是 Tailwind 类名，改样式会假红，改行为却不会红（ISSUE-008 同类） | 放宽为按 role / 可点击断言，或接受为样式快照 |
| T8 | 保留 | seq-a / seq-d 主扫描、r4、r6 | 守 seq 身份和持久化同构，Rust 侧 death_list_gate 有对应 | 保留，只删其中的 basename 用例 |
| T9 | 已解决 | `debug-fold.test.tsx`（已不存在）、ISSUE-008 justify-start（`PermissionModal.test.tsx:260-261` 已改成注释） | 已处理 | 从待办移除 |

扫描方法：`rg` 查 `it.skip|describe.skip|it.todo|expect(true)`，`web/src` 下 0 命中；death-list 自读和 basename 逐个文件复核；vitest 只做只读运行。

### 交叉审 desktop

见「## desktop」末尾「### 交叉审（web）」（15:20 HKT 完成）。

### 交叉审（desktop）

> 审查人：前端-desktop · 2026-10-12 15:20 HKT · 只读：逐条打开 `web/src` 引用的行号核对，没有改代码或测试。

**A. W1–W10 证据核对**

| # | 核对 | 分级意见 | 备注 |
|---|---|---|---|
| W1 | 属实 | 合理（高） | `pendingRpc` 只在 `agentWs.ts:283`（超时）和 `:393-395`（收到响应）被删；onclose（165-189）、connect（100-105）、disconnect（240-245）都没有 reject |
| W2 | 属实 | 合理（中高） | `:135-136` 在 onopen 就清零并置 connected，`:415-417` 才置 handshakeComplete；onclose 走 `socketOpened` 分支时，每轮都从 base 延迟重连 |
| W3 | 属实 | 合理（低，小泄漏） | `:304` 裸调 `this.ws.send`；`send()`（262-268）有 try/catch |
| W4 | 属实 | 合理（中，取决于服务端） | `onPermissionRequest` 直接覆盖单槽；我在 Rust `src/` 里没搜到串行 ask 的契约，建议先向协议域确认 |
| W5 | 属实 | 建议降到**中** | `AgentPanel.tsx:614` 没有 key；`AskUserCard.tsx:61-65` 的本地 state 会残留。注释里写了「step 已 clamp」，说明作者知道组件会被复用，但勾选和文本没处理。实际触发依赖 W4，所以不应该比 W4 高 |
| W6 | 属实 | 合理（中） | `turnStore.ts:677` 的 `\|\| !latest` 正好能把 cancel（624）或 revert（640）清空后的卡片复活 |
| W7 | 属实 | 建议降到**中** | 没有 in-flight 锁，确实成立。不过第二次请求通常只会被服务端拒绝并 toast；「重复写 always 规则」是假设，没给证据 |
| W8 | 属实 | 合理（高） | `:629` `.catch(() => {})`，前面已乐观置 cancelling，违反 FE-04 |
| W9 | 属实 | 建议降到**低-中** | `:999` 已经有 `phase==="failed" && idle` 守卫，双击的窗口只有一个 RTT，服务端多半是幂等的（假设） |
| W10 | 属实（前端侧） | 合理（中） | `sessionStore.ts:179-186` 没有回灌 ask；Rust `src/` 里也搜不到 `pending_permission`，确认需要协议域配合，单靠前端修不了 |
| ✓×3 | 抽查 1 条（connectionStore generation 守卫 `agentWs.ts:131`）属实 | — | 另外两条没逐行复核 |

**B. R2 方案评审**

结论：**合理，可以落地**，但有 3 个副作用要先改：

| 点 | 意见 |
|---|---|
| 共享 helper + 精确豁免正则 | 同意，能解决 T3 的口径漂移，也堵住 `includes("death-list")` 误豁免产品文件的问题（6 处已核对属实） |
| 「口径统一为产品门不扫测试文件」 | **有保留**：r5 / r6 / r9 / model-selection 现在**会扫**其它测试文件，能拦住 mock 或 fixture 里复活的旧字段。统一成不扫，等于悄悄削弱 4 道门。建议 `includeTests` 按门配置，默认保持各门现在的口径，或者明确列出允许豁免的测试文件 |
| ① `files.length > 50` | 魔法数字太脆，会随 web/src 规模漂移。只用「>0 + 锚点 `stores/turnStore.ts` 存在」就够了 |
| ③ 植入 fixture 自测 | 同意，这是唯一能证明「门真的会红」的断言。临时文件要写到 `os.tmpdir()` 再传给 `scan(files)`，**不要写进 web/src**，否则并发跑 vitest 时会互相污染，中断后还会留下残留文件 |
| 不进 slim CI 必过集 | 同意，跟 WP-012 / 017 的结论一致 |

**C. 9 条测试裁剪评审**

| # | 意见 | 理由 |
|---|---|---|
| T1 | 同意 | 5 处 basename 断言都核对过，是同义反复；前提是 §2 里替代的 non-vacuous 断言同批落地 |
| T2 | 有保留 | 已核对 R9 `FORBIDDEN_PRODUCTION` ⊇ R5（partialText / partialReasoning 都在），删 R5 在不变量上是安全的。但 R5 是 ISSUE / 回归编号的追溯锚点，建议保留 R5 文件、去掉重复的针，或者在 R9 里注明「含 R5」后再删 |
| T3 | 同意 | 8 份 walk 实现都是复制出来的；和 R2 的 helper 一起做 |
| T4 | 同意（倾向加强而不是删） | `r9:56-69` 只检查 type 是否在枚举里，名不副实；但它是 web 侧唯一读 Rust golden fixture 的点，删了就丢掉这条跨语言的锚。建议接上 adapter 投影 |
| T5 | 同意 | `src.includes("modelId")` 几乎恒真；同一用例里的 `"Missing: "` 和两条负向针要保留 |
| T6 | **反对** | `serve-auth:18-19` 是**负向针**（禁止已删除的 UI 回来），不是锁定现有文案。换个说法确实能绕过，但去掉它们就会丢掉「原样复活旧 auth UI」这道唯一的防线。保留的成本几乎为零，建议保留，或者改成断言组件或 testid 不存在 |
| T7 | 有保留 | `PermissionModal.test.tsx:259` 的 `w-full` 只是实现细节，这点同意。但同一段还断言了 `flex-col`（256），放宽要两处一起改，换成 role/布局语义的断言；不建议直接删，否则 ISSUE-008 的回归就没人看着了 |
| T8 | 同意 | 保留主扫描，只删 basename 用例 |
| T9 | 同意 | `debug-fold.test.tsx` 已不存在，已核对 |

**结论**：W1–W10 证据 10/10 属实（0 部分属实、0 不符），建议 W5、W7 降到中，W9 降到低-中；R2 可以落地，但先改「统一不扫测试」和「写进 web/src 的植入 fixture」这两处副作用；9 条裁剪里同意 6 条、有保留 2 条（T2、T7）、反对 1 条（T6）。

## desktop

> 作者：前端-desktop · WP-030（父 WP-023）· 2026-10-12 · 只读审查，未改代码/测试，未 commit  
> 基线：`cd desktop && npm test` → **109 pass / 0 fail / 0 skip**（39 suites，约 3.1s）  
> 范围：`desktop/src`、`scripts/`、`.github/workflows/windows-*.yml`。「假设」= 未实测验证。

### Part 1 — 可疑不稳实现

| # | 风险 | 位置 | 问题 / 失败场景 | 建议修法 |
|---|---|---|---|---|
| D1 | 高 | `ssh-session.ts:638-659`（startTunnel），`main.ts:741-751`（managedRemote） | `ssh -N -L` 隧道子进程**无 exit/error 监听**，也没有 `ServerAliveInterval`。网络闪断/休眠唤醒后隧道死掉，workbench 指向失效的本地端口，`managedRemote` 仍保留旧句柄；没有重连提示，也不会自动重连，只能用户回 hub 手动 Reconnect | tunnel `on("exit")` 后置 `managedRemote` 失效，并向主窗口发 `remote-disconnected`；加 `-o ServerAliveInterval=15 -o ServerAliveCountMax=3 -o ExitOnForwardFailure=yes`；可选：退避自动重连（复用 `reconnectRemote`） |
| D2 | 高 | `sidecar.ts:176-181`、`main.ts:103,1051-1072` | `waitForReady` 的 exit 监听在 READY 后 `settled` 直接 return，main 也**没有挂 post-ready 的 exit 监听**。sidecar 运行中崩溃时 UI 白屏/请求全失败，`sidecar` 句柄过期，没有重启或提示 | `startSidecar` 返回后在 main 挂 `process.on("exit")`：非主动停止时提示并提供「重启」（受限次数的自动重启）；`stopSidecar` 先打「主动停止」标记，避免误报 |
| D3 | 高（流程） | `.github/workflows/*`（全部） | **没有任何 workflow 跑 `desktop npm test`**；`windows-sidecar.yml:4-24` 的 paths 不含 `desktop/**`；`package_win.ps1:107` 只跑 `npm run build`。ISSUE-028 的三道门只在本地生效，CI 永远绿 | 在 `windows-desktop` / release 的 build 前加 `npm test`，或新建 `desktop-test.yml`（paths: `desktop/** scripts/** .github/workflows/windows-*`） |
| D4 | 中 | `main.ts:1360-1364`（start-remote-session），`main.ts:862-880` | 双击 Connect 或重入时，`clearAllPendingRemotes` 只清**已登记**的 pending，清不到正在跑的 `startRemoteSession`。两条流程会并行 scp+`tar -xf` 到同一个远端目录；`completeRemoteSession` 被并发调用时，两个 serve 都起来，只有一个被 `managedRemote` 记住，另一个泄漏 | main 侧加单飞锁（进行中直接拒绝或复用同一个 promise），hub 按钮在 promise 结束前 disable；`completeRemoteSession` 以 `pending.ready` 幂等 |
| D5 | 中 | `main.ts:1051-1072`（relaunchSidecar）、`1074-1088`（connectRemote） | 切换工作区与 connectRemote 没有互斥。两次 relaunch 并发时都 `stopSidecar(sidecar)` 同一个旧句柄，然后各自 start，后赋值的覆盖前者，**孤儿 sidecar 一直占着工作区锁**（`--parent-pid` 只在主进程退出时兜底） | 串行化 sidecar 生命周期操作（promise 链或 mutex） |
| D6 | 中 | `components.ts:689-692,717-719,729-748` | `.part`/staging 只在**进程内** finally 里清理。下载中强退、崩溃或断电后，`bundles/.tmp/*-<nonce>.part` 和 `embed-model-<nonce>/` 永久残留（每次几十 MB），之后不会再扫 | `ComponentManager` 构造或首次 `download` 时清理 `.tmp` 下超过 N 小时的条目，或启动时直接清空 `.tmp`（先确认没有别的实例在用：可按 pid 命名） |
| D7 | 中 | `components.ts:405-446` | 不支持断点续传，也不能取消：每次重试都从 0 开始（3 次，退避 0.8s/1.6s）；`AbortController` 只在局部用，没有 cancel API，`before-quit`（`main.ts:1596-1607`）不会中止下载。弱网下大模型反复失败，关窗后还在后台下载 | 先补 cancel（Manager 持有 ctrl，quit/取消时 abort）；续传可以后做（Range + `.part` 保留 + 完整 sha 复核） |
| D8 | 中 | `main.ts:739-756` | `managedRemote` 赋值以后，如果 `connectRemote`（751）抛错，catch 会停掉 tunnel/serve 并删除 key 目录，但 **`managedRemote` 仍指向已停止的句柄**，下次 `stopManagedRemote` 会重复停并再删一次 | catch 中 `if (managedRemote?.session === session) managedRemote = null`；或把赋值挪到 `connectRemote` 成功之后 |
| D9 | 中 | `assemble_product.ps1:37-38` | `npm ci` / `npm run build` 之后**没检查 `$LASTEXITCODE`**（`$ErrorActionPreference=Stop` 管不到 native 退出码；假设 pwsh<7.4 或者没开 `PSNativeCommandUseErrorActionPreference`）。web 构建失败时会用旧的 `web/dist` 打包，CI 照样绿 | 和 `package_win.ps1:29` 一样加 `Assert-LastExitCode` |
| D10 | 中 | `windows-signed-release.yml:9-15,149-169` | 同时监听 `release: published` 和 `push tags v*`：先推 tag 再发布 Release 会**跑两次**，两个 run 并发往同一个 Release 上传同名资产（可能 already_exists 失败，或互相覆盖）（假设：取决于 gh-release action 的行为） | 只保留一个触发器（推荐 tag push），或加 `concurrency: release-${{ tag }}` |
| D11 | 低 | `components.ts:751-760`、`main.ts:676` | 两个 app 进程（多实例）各有自己的 `inflight`，可能同时下载。`installLinuxTar` 先 rm `.sha256` 再 rename tar，另一个进程这时 `resolveLinuxBundle` 会认为文件缺失；`main.ts:676` 读 checksum 遇到 ENOENT 会直接抛错 | 读 checksum 失败时重新 `ensure` 一次；或者用 `.sha256` 原子替换（写 tmp 再 rename），不要先删 |
| D12 | 低 | `components.ts:737-744` | 替换模型的步骤是 rename old → rm old → rename staging，中间崩溃会导致没有模型（下次会重新下载，能自愈） | 可接受；或者先 rename staging 到 `final.new` 再交换 |
| D13 | 低 | `ssh-session.ts:422,446-448` | scp 中断时远端 `~/.litecode-upload-*.tar` 不会被清理（cleanup 只包住了 exec 段） | 把 try 扩到包含 scp；或者下次安装时清理旧的 `.litecode-upload-*` |
| D14 | 低 | `ssh-session.ts:433-439` | 解包到已有的目标目录前没有清空，旧版本残留文件会留着；stamp 最后写，中断后下次会因 hash 不同而重新解包（已缓解大半） | 解到 `target.new` 再 `mv`，或者先 `rm -rf` 受控子目录 |
| D15 | 低（假设） | `hub.ts:384-389`、`remote-progress.ts:86` | error 阶段被刻意过滤掉，进度条停在最后的比例，靠 ensure 抛错后 hub 改文案。hub 的 catch 如果没重置进度条，就会出现「卡在 38%」的陈旧 UI（未逐行核对 hub 的 catch） | 失败时 hub 重置 bar 并显示错误；或者在 main 侧失败时发 `{stage:"error"}` |
| D16 | 低（假设） | `windows-signed-release.yml:123-127` | 签名开关靠 regex 改 `package.json`。字符串不完全匹配时会**静默**不签名，日志却打印「signing enabled」 | 替换后断言已命中（`-match '"signExecutable": true'`），否则 throw |

**已缓解（不计入风险）**：  
- sha256+size 在 rename 前校验，超长流立刻中止，失败时删 `.part`，Verification 错误不重试（`components.ts:344-379,437-441`）。  
- install record 在文件落位**之后**原子写（tmp+rename）（`283-290,709`）。  
- extract 先写 staging，失败时 finally 删掉；拒绝路径穿越和链接；截断归档会抛错（`494-587,746-748`）。  
- 同进程并发 `ensure` 共享同一个 promise；装好后清掉旧错误（`650-672,635-646`）。  
- ISSUE-028 三道门：remote-progress 失败传播 + listener 移除、bundle 全序、slim 卫生（`remote-progress.test.ts`、`bundle-paths.test.ts`、`slim-packaging.test.ts`）。  
- release 版本三角在 build 前就会阻断（`windows-signed-release.yml:78-91`）；缺组件时 `build_components.mjs` 不带 `--allow-missing` 会 throw，加上 `fail_on_unmatched_files: true`（138-139,169），两层都会阻断。  
- `assemble_product.ps1:30` 先清空输出目录，残缺时 `package_win.ps1:71-73` 会 throw；`package_win.ps1:156-162` 在运行时阻止 models/linux 泄漏进包；临时 builder config 在 finally 里删除。  
- startRemoteSession 失败时 finally 删除明文凭据目录（DESK-03）。

### Part 2 — 测试裁剪清单（只列，不删）

| 来源 | 测试 / 步骤 | 分类 | 理由 |
|---|---|---|---|
| desktop/src | `sidecar.test` › stopSidecar「is a no-op for a null handle」 | 可删 | 第 186 行一句 `if (!handle) return` 的恒真断言，几乎不会回归 |
| desktop/src | `ssh-credential-material.test` › removeMaterializedKey「is a no-op for an empty key directory」 | 可删 | 同上，琐碎的 no-op；真正的风险（异常路径的清理）已由 withMaterializedKeyDirectory 覆盖 |
| desktop/src | `slim-packaging.test` › 「package_win.ps1 defaults to slim…」 | 可放宽 | 对脚本**源码文本**做 regex（含注释 `Default = slim`）：改个注释就红，逻辑坏了却可能还绿。真正的门是 `package_win.ps1:156-162` 的运行时检查。建议只保留「存在 leak throw」一条，或者改成在 CI 上检查 `win-unpacked` |
| desktop/src | `slim-packaging.test` › 「assemble_product.ps1 only copies models when -WithModel」 | 可放宽 | 同上，锁死了文案和变量名 |
| desktop/src | `bundle-paths.test` › 「candidate list order matches packaged→…→development」(242) | 可放宽 | 锁的是内部候选数组这个实现细节，行为已由 193 号「earlier linux candidates win」覆盖 |
| desktop/src | `sidecar.test` › formatSidecarBootFailure ×2 | 可放宽 | 字符串格式快照，只需断言包含 code/bin/log 关键字段即可 |
| desktop/src | `bundle-paths.test` › 84/91/99（单条 env/bundle-root 优先） | 重复 | 已被 120「locks the full precedence」和 193 全序用例覆盖 |
| desktop/src | `remote-progress.test` › remoteDownloadRatio ×2 + mapComponentProgressToRemote ×2 | 重复 | 64/107 号 ensureComponentForRemote 用例已端到端走过映射和过滤；可以合并成一条表驱动用例 |
| desktop/src | `ssh-credential-material.test` › 「recursively removes the temp directory」 | 重复 | 与 withMaterializedKeyDirectory 两条（正常/抛错）重叠 |
| desktop/src | `remote-progress.test` 整体（相对 main.ts 接线） | 假绿风险 | 测的是抽出来的纯函数，`main.ts:442-458` 的 subscribe/unsubscribe 接线和 `broadcastComponentProgress` 并不在测试内；接线写错时测试照样绿 |
| desktop/src | `package.json` `test` 脚本显式枚举 18 个 `dist/*.test.js` | 假绿风险 | 新增 `*.test.ts` 忘了登记就**静默不跑**；目前 18/18 都已登记。建议改成 glob 或加一致性检查 |
| scripts | `smoke_product.sh` vs `smoke_product.ps1` | 重复 | 同一套 smoke（READY/401/锁冲突/重启）；sh 版没有任何 workflow 调用 |
| scripts | `smoke_product.ps1:115-119` 端口复用比较 | 假绿 | 两个分支都只 `Write-Host`，不构成断言；可删，或者明确只是信息输出 |
| workflows | `windows-sidecar.yml:49-53` Cache embed models + `:60-62` models 存在性 echo | 可删 | 用了 `-SkipModel`，模型不进产物；echo 分支没有断言 |
| workflows | `windows-desktop.yml:45-50` CSC 检测 echo | 可删 | 两个分支只打印日志，没有行为 |
| workflows | `windows-desktop.yml:32-36`、`windows-signed-release.yml:110-114` Cache embed models | 可删（假设） | `models/` 下含 onnx/.data 已入 git（`git ls-files` 可见），cache 冗余；如果其实是 LFS 指针就保留 |
| workflows | `windows-signed-release.yml:49-53` `web/node_modules` 存在分支 | 可删 | 全新 runner 上永远走不到这个分支，是死代码 |
| workflows | `windows-desktop` / `windows-signed-release` 整体作为「门」 | 假绿 | 不跑 `desktop npm test`（见 D3），绿只代表能编译能打包 |

**保留（不建议动）**：components.test 全部（含 `build_components.mjs` 实跑契约、并发共享、坏 hash、跨版本 tar），bundle-paths 全序与 packaged 不回落，ipc-trust / window-guard / url-policy（安全边界），browser-host、dev-ui、layout-file、popout-bounds、recents、lap-path、parse-user-host、handshake-url；scripts 中 `package_win.ps1:152-162`、`build_components.mjs:932-935`（pinned sha）；release 中版本三角检查和 `fail_on_unmatched_files`。

**覆盖缺口（简）**：① CI 不跑 desktop 测试（D3）；② sidecar 运行中崩溃、隧道断开都没有测试（D1/D2）；③ `.tmp` 残留清扫、下载取消（D6/D7）；④ Open Remote 重入/双击（D4）；⑤ `SshSession.installTar/startTunnel` 只测了参数构造，失败清理没测；⑥ `stopSidecar` 在 Windows 上的 taskkill 分支没覆盖。

**统计**：Part 1 风险 **16 条：高 3 / 中 7 / 低 6**（其中「假设」3 条），已缓解 8 项。Part 2 共 **18 条**：desktop/src 11 条（可删 2 / 可放宽 4 / 重复 3 / 假绿风险 2）；scripts 2 条（重复 1 / 假绿 1）；workflows 5 条（可删 4 / 假绿 1）。基线 109/109 绿。

### 交叉审（web）

> 审查人：前端-web · WP-029 · 2026-10-12 15:20 HKT · 逐条打开 文件:行号 只读核对，未改代码、测试和 desktop 正文。

**总评**：质量高，行号基本准确（偏差 ≤3 行）。16 条可疑实现中，13 条成立，2 条部分成立（D10、D16），1 条不成立（D15）。测试裁剪 18 条中 15 条合理，3 条有异议（见下）。

**逐条结论（Part 1）**

| # | 核对结果 | 备注 |
|---|---|---|
| D1 | 成立 | `ssh-session.ts:638-658` 的 spawn 没有挂 exit/error 监听，也没有 ServerAlive 参数；`main.ts`、`ssh-session.ts` 里 rg 不到任何 tunnel `.on(` |
| D2 | 成立 | `sidecar.ts:177-178` 的 `if (settled) return`；main 里没有 post-ready exit 监听 |
| D3 | 成立 | 3 个 workflow 都 rg 不到 `npm test`；`windows-sidecar.yml:7-24` 的 paths 不含 `desktop/**`；`package_win.ps1:107` 只跑 build |
| D4 | 成立 | `main.ts:1360-1364` 只清理已登记的 pending，没有单飞锁；`:875-880` 的 completeRemoteSession 不幂等 |
| D5 | 成立 | `main.ts:1051-1072` 没有锁，`sidecar` 会被后赋值覆盖（与 WP-018 F11 一致） |
| D6 | 成立 | `components.ts:688-719` 只在进程内 finally 里 rmSync，没有启动清扫 |
| D7 | 成立 | `components.ts:405-418` 中 ctrl 是局部变量；`:431-443` 每次重试从 0 开始，退避 800×attempt；`main.ts:1596-1607` 的 before-quit 不 abort |
| D8 | 成立 | `main.ts:741-751` 先给 managedRemote 赋值再 connectRemote；catch 在 `:752-756` 停了句柄，但没有置 null |
| D9 | 成立（假设证实） | 见下方「假设结论」 |
| D10 | 部分成立 | 见下方「假设结论」 |
| D11 | 成立 | `components.ts:751-760` 先 rm checksum；`main.ts:675` 的 readFileSync 遇 ENOENT 直接抛错。多实例才会触发，低风险定级合理 |
| D12 | 成立 | `components.ts:737-744`，可自愈，定级低合理 |
| D13 | 成立 | `ssh-session.ts:422` 的 scp 在 try（`:440`）之外 |
| D14 | 成立 | `:434` 直接 `tar -xf` 到已有 target，没有先清空 |
| D15 | 不成立 | 见下方「假设结论」 |
| D16 | 部分成立 | 见下方「假设结论」 |

**假设结论**
- **D9：成立**。`assemble_product.ps1:37-38` 的 npm ci / build 之后没有检查 `$LASTEXITCODE`。同一脚本在 `:68-75` 对 cargo 显式做了检查，`package_win.ps1:29-31,105-108` 也对 npm 做了检查，可见作者清楚 native 退出码不受 `ErrorActionPreference` 约束。pwsh 7.4+ 中 `PSNativeCommandUseErrorActionPreference` 默认是 `$false`，脚本和 workflow 都没有设置它。所以 web build 失败时，如果旧的 `web/dist` 还在，就会被照样打包（`:30` 只清理 OutDir，不清理 web/dist）。
- **D10：部分成立，仍需运行验证**。`windows-signed-release.yml:9-14` 同时监听 `release: published` 和 `push tags v*`，全仓 rg 不到 `concurrency`，所以「先推 tag 再发布 Release」会跑两次，这一点已从代码证实。两个 run 同时上传会不会失败，取决于 `softprops/action-gh-release@v2` 遇到同名资产时的行为（覆盖还是报 already_exists）以及时序，需要实跑或查 action 源码确认。不管哪种结果，加 `concurrency` 都没有坏处，建议照做。
- **D15：不成立**。`hub.ts:414-416` 失败时 catch 执行 `setStep("step-auth")`，进度条所在的 deploy 步骤随之隐藏，用户看不到停住的比例。重试时 `:399-401` 会把进度条先置为 4% 并显示 "Starting…"，返回首页时 `:334-335` 会清零。陈旧进度不会出现在用户眼前。可以保留一条改进项：error 阶段被过滤（`remote-progress.ts:53`），失败原因只靠 `message()` 显示，这本身是合理的。
- **D16：部分成立（潜在风险，当前不触发）**。`desktop/package.json:48` 当前的字面量正是 `"signExecutable": false`，与 `windows-signed-release.yml:124` 的 `-replace` 精确匹配，所以现在能正确开启签名。但如果格式变了（换空格、换行或改成 true/删除字段），替换会静默失效，而 `:123` 照样打印 "signing enabled"。断言建议成立，风险定级维持「低」。

**Part 2 裁剪建议核对**：18 条中 15 条合理，3 条有异议，2 条附带补充。
- 合理（15 条）：
  - `slim-packaging.test.ts:36-45` 和 `:47-52`：确实是源码文本 regex，含注释匹配（`Default = slim`、`Default is slim`）。
  - `bundle-paths.test.ts:84/91/99` 被 `:120` 和 `:193` 覆盖：成立。
  - `bundle-paths.test.ts:242` 锁定数组顺序：成立。
  - `formatSidecarBootFailure`（`sidecar.test.ts:22-50`）：放宽合理。
  - `ssh-credential-material.test.ts:40`：与 withMaterializedKeyDirectory 的用例重叠，成立。
  - `package.json` 枚举了 18 个 test.js，`src` 下也恰好 18 个 `*.test.ts`：现状一致，风险描述准确。
  - `smoke_product.sh` 没有被任何 workflow 调用，`windows-sidecar.yml:66` 只调 ps1：成立。
  - `smoke_product.ps1:115-119` 两个分支都只 Write-Host：成立。
  - `windows-sidecar.yml:49-62` 的 cache 加 echo：成立。
  - `windows-desktop.yml:54-58` 的 CSC echo：成立，行号实为 `:50-58`。
  - Cache embed models 冗余：**假设证实**。`models/ibm-granite/**` 的 `.onnx`（356 KB）和 `.onnx.data`（65.9 MB）都是 git 普通 blob，`filter: unspecified`，不是 LFS 指针。
  - `windows-signed-release.yml:49-53` 的 `web/node_modules` 分支：setup-node 的 cache 只缓存 `~/.npm`，不缓存 node_modules，所以这是死分支，成立。
  - 两个 workflow 作为门存在假绿：与 D3 一致，成立。
- 异议 1：`sidecar.test.ts:56`「no-op for a null handle」**不建议删**。`main.ts:1053`、`:1077`、`:1601` 三处都会以 null 调用 `stopSidecar`（启动前、远程模式、退出时），这条测试守的是 before-quit 不抛错的契约，成本几乎为零。
- 异议 2：`ssh-credential-material.test.ts:47`「no-op for empty key directory」**建议保留**，理由同上：远程 key 路径为空是真实调用路径。如果要删，应先确认 `removeMaterializedKey("")` 不会退化成删除 cwd，而这正是这条测试防的。
- 异议 3：`remote-progress.test.ts` 中 `remoteDownloadRatio` 和 `mapComponentProgressToRemote` 的用例**不算重复**。`:14-23` 覆盖了 total 为 0、undefined、received>total 截断等边界，`:64`、`:107` 的端到端用例不覆盖这些。可以合并成表驱动，但不应删减用例。
- 补充 1：把「`remote-progress.test` 整体假绿风险」定为整文件假绿，定性偏重。`:107`「propagates ensure failures and still removes the listener」真实地守住了 finally 退订。缺口只在 `main.ts:442-458` 的接线，建议改称「接线未覆盖」，而不是「假绿」。
- 补充 2：`slim-packaging` 放宽后，建议 C1（CI 上解包检查 win-unpacked）真正落地再删 regex 用例，否则在过渡期会失去这道门。


## 汇总（前端中层）

子包：WP-029（前端-web）、WP-030（前端-desktop），均已验收为 done。双方已互审对方的节；我抽查了 web 的 W1/W5/W6 和 desktop 的前 3 条高风险项，证据都对得上。不 commit，没有改产品代码，也没有改测试。

### 1. 可疑实现（高优先）
- web W1：`agentWs.ts` 在断线或换代时不 reject 在途的 RPC，请求要等 30 秒超时才结束。
- web W4/W5/W6：同一会话后到的 Ask 会覆盖前一个；PermissionCard 没设 key，旧卡上的答案会带到新卡；`turnStore.ts:677` 的回滚会把已取消的卡又恢复出来。互审建议把 W5、W7 降为中，W9 降为低中；W10 需要协议域配合。
- desktop：CI 里没有任何 workflow 跑 desktop 的 npm test，ISSUE-028 的三道门只在本地生效；`ssh-session.ts:638-659` 的隧道没有退出监听，断线后静默挂死；`sidecar.ts` 在 READY 之后不再处理退出，崩溃时既不提示也不重启。
- desktop 16 条经 web 互审：13 条成立；D10、D16 部分成立（D10 待实跑验证）；D15 不成立。正文标了「假设」的实际是 4 条。

### 2. 已修
- 无。本包只做排查，修复要另开包。

### 3. R2（web death-list 中 basename 自指的弱断言）
- 共 5 处，web 给了加强方案，还没改。desktop 互审意见：一是不要让产品门统一不扫测试文件，否则 r5/r6/r9/model-selection 会漏掉 mock/fixture；二是自测临时文件写到 os.tmpdir()；三是 `files.length > 50` 改为「大于 0 且包含 stores/turnStore.ts」。

### 4. 测试裁剪建议（只列不删，交 Andy 拍板）
- web 9 条：desktop 同意 6 条；暂缓 2 条（T2 先在 R9 注明它包含 R5，T7 和 flex-col 一起处理）；反对 1 条（T6 是防旧登录界面重新出现的负向门，应保留）。
- desktop 18 条（可删 6、可放宽 4、重复 4、假绿 4）：web 有异议的 3 条，sidecar null 句柄 no-op、ssh 空 key 目录 no-op 这两条应保留，remote progress 边界用例改成表驱动合并，不删。
- 建议：有异议的条目默认保留；没有异议的条目由 Andy 拍板后再开新包执行。

### 5. 建议后续包
- 修 web W1/W4–W6；desktop 的 CI 接入 npm test，ssh/sidecar 补退出监听；按互审意见落地 R2；裁剪测试等批准后再做。
