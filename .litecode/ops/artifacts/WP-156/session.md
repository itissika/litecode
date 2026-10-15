# WP-156 Session 子包：serve 握手与会话侧打开延迟（WP-154）

> **注 1：本文是磁盘回滚后，按 10-15 01:22 前后的最终汇报数字重写的。**原 `.tmp/wp154/session.md` 已丢失；文中数字与那次汇报一致，没有重新测量。
> **注 2：`summary.json` 里的原始聚合数据（每次 run 的原始值、热连 9 次的明细、每次 run 的 mincore 页数）已丢失。**下文只保留汇报中的中位数和区间。
> **注 3：后端中层裁定：发现 1 写进 WP-154 合并报告，先不立包；auth 由前端删掉发送，serve 不加方法。**

二进制：`target/release/litecode`，commit 3ddc6433f53b6349d087ab49bca3a4e8c4753f58（IDE W1）。单位 ms，表中时刻相对 T0，取中位数：首连每档 3 次，热连每档 9 次。测量时间：10-15 01:05–01:22 HKT。
**C2：只有小仓有数据；大仓 C2 是缺口，待定（§5）。**

## 1. 口径与方法
- T0 = 客户端发出 `new WebSocket(/ws?token=)`。客户端 `ws_client.mjs` 的请求顺序按 frontend_mid（HEAD 3ddc643）给出的真实打开顺序，并逐条对照 web/ 源码：
  - 连接打开后立即发 session/list（connectionStore.ts:157-164）；
  - 有恢复的 panel 才发 session/subscribe（AgentPanel.tsx:112-117）；
  - 有 token 才发 auth（agentWs.ts:136-141）；
  - 收到 server/hello 后，并发 GET /api/workspace/engines、GET /api/settings、session/new；
  - session/new 回包后订阅新会话（sessionStore.ts:591-595）；
  - 收到 snapshot 后发 buffer/load（sessionStore.ts:216-229）。
  - 只记录消息类型、时刻和字节数，会话 id 取 sha256 的前 8 位。
- **A（新 profile）**：解锁点 = session/new 回包。**B（恢复布局）**：解锁点 = session/list 回包中出现恢复会话 28396d73（348 条、94 个回合）。Snapshot 只记录，作参考。
- **spawn→READY** 单独列出：从 serve 进程启动到 stdout 打出 `LITECODE_READY`，与 IDE 口径一致。T0 在 READY 之后。
- **C1**：每次都用全新 HOME 和全新工作区。大仓用 .tmp/wp154/big 的副本，**不带代码种子**（method.md 的 C1 定义）；不写 engines.json，即 retrieval 和 LSP 都关，不做 embed。B 路径的副本只放语料。
- **C2（仅小仓）**：沿用建索引时的 HOME 和工作区，会话索引已建完（1044 块）。按真实配置开 retrieval；小仓本来没有 LSP。
- **真冷启动**：每次 run 前按 evict.md 逐出二进制、模型、工作区和 HOME 的页缓存，**每次逐出后读数都是 0 驻留页**。系统共享库不在逐出范围内。
- 鉴权：serve 设置 LITECODE_TOKEN；客户端带 `?token=`，同时发 `auth{token}`。
- 护栏：每次 run 前 MemAvailable ≥ 6GiB；run 前后都确认没有残留进程，残留数全为 0；结束时整组杀掉。
- 语料：.tmp/wp102/corpus/sessions.db（25.6MB，29 个会话、其中根会话 26 个，4336 条，正文 6.80MB）。副本里把最近 5 个根会话改指向被测仓：

| 会话 | 条数 | 正文大小 |
|---|---|---|
| 28396d73 | 348 | 470KB |
| 76173f6b | 360 | 421KB |
| 4fddd4fe | 63 | 44KB |
| 2e8e9fec | 241 | 450KB |
| b9d6c725 | 205 | 426KB |

  session/list 不按工作区过滤（read.rs:235-247），返回 18 个会话。

## 2. 冷启动（首连）中位数
| 仓/路径 | 档 | spawn→READY | 解锁点 | session/list 回包 | subscribe 回包 | Snapshot |
|---|---|---|---|---|---|---|
| 小仓 A | C1 | 89 | 58（session/new 回包） | 15 | 105（订阅新会话） | 57 |
| 大仓 A | C1 | 498 | 61 | 15 | 105 | 60 |
| 小仓 B | C1 | 107 | 239（list 中出现 28396d73） | 238 | 240 | 239 |
| 大仓 B | C1 | 486 | 281 | 280 | 282 | 281 |
| 小仓 A | C2 | 69 | 66 | 64 | 108 | 64 |
| 小仓 B | C2 | 74 | 268 | 267 | 269 | 268 |

## 3. 同进程热重连
A 路径解锁约 57–65，B 路径约 184–191。

## 4. 发现
1. **B 路径：恢复会话拖慢 session/list 回包约 200ms，是最该延后的一项。**
   - 同样 18 个会话，A 路径 list 回包在 64ms 到，B 路径在 238–281ms 才到。
   - B 路径下 list 回包与恢复会话的 snapshot、subscribe 回包相差不到 1.5ms，几乎同时到达。
   - 原因：list 和 subscribe 走同一个会话循环（`src/client_protocol/connection.rs:891` 与 `:715`），list 被恢复会话（348 条）的加载和回放拖住了。
   - 建议：先回 list，再做回放。
   - 按裁定写进 WP-154 合并报告，先不立包。
2. **auth 实测返回 -32601。**
   - token 在 WS 握手时已经校验（`src/client_protocol/ws.rs:52-56`、`src/serve/auth.rs:66-72`）；serve 里没有 JSON-RPC auth 的处理，走到 `connection.rs:1449-1456` 的 Method not found。
   - 前端的 `send()` 发出就不管（`web/src/api/agentWs.ts:252-273`），不在关键路径上。但这条请求排在 list/subscribe 之后处理，每次都会回一条错误回包。
   - 按裁定由前端删掉发送，serve 不加方法。
3. **大仓 spawn→READY 比小仓多约 400ms。**
   - 原因是 IDE 已查明的全新 HOME 下的 git snapshot 和 `load_runtime_bundle`（ide.md）。
   - 这段在 T0 之前，属于壳的启动段，不在 T0→解锁的窗口内。C2 保留 HOME 后，小仓降到约 70ms。
4. **会话索引在后台加载，不影响解锁。**小仓 C2 的解锁时刻（66）与 C1（58）基本相同。

| 步骤 | 关键路径 | 阶段 |
|---|---|---|
| serve 启动→READY | 是（T0 之前） | P0 |
| WS 升级 + hello（hello 在读循环开始前入队，ws.rs:339-341） | 是 | P0 |
| session/new 回包（A 解锁） | 是 | P0 |
| session/list 回包（B 解锁） | 是 | P0 |
| 恢复会话 subscribe + 回放 | 拖住了 list | P1，应延后到 list 之后 |
| buffer/load、GET engines/settings | 否 | P1 |
| auth -32601 | 否 | 删除（前端） |
| session_search 索引加载 | 否（后台 worker） | P2 |

## 5. 建 C2 会话索引（release；retrieval 开，LSP 关；整组 watchdog，门槛 4GiB）
- **小仓：建成。**共 1044 块，01:14:55 HKT 结束；进程树峰值 RSS 2.07GB，MemAvailable 最低 4.32GB。第一次尝试跌破门槛后停掉，第二次续建完成。
- **大仓：没建成，C2 待定（方案 4 缺口）。**
  - 4 次尝试都在 36–50s 内跌破 4GiB：进程树峰值约 3.07GB，MemAvailable 最低 3.39GB；最后一次 01:19:53 结束。
  - 原因：代码索引加载加上会话 embed，单个 worker 就占约 3GB。
  - 要补测，需要起点约 ≥ 7.5GiB 的内存窗口。
- 更早在 4.5GiB 门槛下的尝试也都跌破门槛，停掉了。
- 引擎配置的发现：engines.json 里没有独立的 code_search 开关，`code_search` 直接读 `retrieval.desired`（`src/config/workspace.rs:288`），会话索引也共用同一个 worker。

## 6. 引用的 ISSUE
- **ISSUE-061**：serve 收到 SIGTERM 后，code-search-worker 子进程不退出，成为孤儿继续 embed（约 2.8GB RSS）。harness 已改为整组结束进程，之后没有再出现。
- **ISSUE-062**：会话引用的外部文件不存在时，sparse 维护每 30s 重复报错。语料中 bd209b21:210 就是这种情况；副本里置空后，变成每 30s 报 "unparseable transcript row"。

## 7. 偏差
- 语料副本：bd209b21:210（不在改指向的会话里）的外部引用被置空。
- 会话语义索引要在 engines.json 里设 `retrieval.desired=true` 才会建。小仓只开这一项；大仓建索引期间关掉了 LSP。
- 小仓的会话索引曾被 C1 的 run 覆盖，后来从头重建，不影响结果。
- 小仓 C2 的 A 路径第一轮因内存门槛中断（只跑完 1 次），之后整档重跑了 3 次。
- 规则下达前，曾有一次 debug 编译（10-14 22:5x，1m15s），以及一次因缺 web dist 而失败的小仓 serve 启动；都不计入数字。debug 版只用于自测脚本，包括 B 路径自测（list 中出现 28396d73）。
- 脚本在回滚后重建，见 /workspace/wp156-recover/，与原版的差异见该目录的 README.md。
