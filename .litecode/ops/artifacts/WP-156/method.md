# WP-156 Session 子包测量方法（只读调研，父包 WP-154）

## 范围
serve 连接与握手（`src/serve/router.rs` `/ws` → `client_protocol/ws.rs::ws_handler/handle_socket`）、client_protocol 首批消息（ServerHello、SessionSnapshot/TurnSnapshot 等，`protocol.rs`）、会话列表（ListSessions）与最近会话加载/回放（`src/session` 读库 + transcript replay）、session_search 索引 load（`load_session_index`、`check_meta_or_drop`、迁移、ANN 内存重建）。

## 口径（沿用 .tmp/wp154/plan.md）
- T0 = 模拟前端发起首个握手请求（WS upgrade 请求发出时刻，客户端单调时钟）。
- 终点 T_int（后端视角"输入框可用"）：客户端收到"可发起 Start 的最后一条前置消息"的时刻。**建议**以活动会话的 SessionSnapshot（含 TurnSnapshot 且 phase=Idle）送达为准；若前端实际还等 ListSessions 回包，则取两者较晚者。确切消息待与 frontend_mid 核对（见待决问题 1）。另记一个"首个操作被响应"点：T_int 后立即发一个 ListSessions，取回包时刻。
- 冷启动（无 root，不能 drop page cache）分两档：
  - C1 = 新 serve 进程 + 首次打开该工作区：HOME 指向新建空目录，工作区 `.litecode/session-index` 拷到空副本（无索引、无 job/queue），sessions.db 用语料副本。
  - C2 = 新 serve 进程 + 已有索引（真实"重启后打开"）。
  - 页缓存：用 `vmtouch -e`（如可用）或 `dd`+`posix_fadvise(DONTNEED)`（python `os.posix_fadvise`，无需 root）尝试逐出语料与索引文件；能否逐出以 `fincore`/mincore 读数核实并在结果中注明。
- 热启动 W = 同一 serve 进程内断开再重连 WS（新连接重走握手），以及切换到另一会话。
- 每档 ≥3 次，报中位数并附原始数据。

## 模拟前端握手
`.tmp/wp156/ws_client.py`（python websockets，或现有依赖不可用时用 Rust/Node 小脚本放 .tmp）：按前端真实顺序发首批请求（顺序从 web 源码/frontend_mid 提供的抓包核对），记录每条收发的单调时间戳和消息类型、字节数，不记录内容。鉴权按 `serve/auth.rs` 的本地 token 流程，token 只在进程内传递、不落盘不打印。

## 后端插桩
- 优先现有 tracing（`config/log_filter.rs` 的环境变量过滤，开 debug 级 serve/client_protocol/session/engines::session_search 目标），用时间戳对齐客户端时间线。
- 现有 span 不覆盖的步骤：不改产品代码；用外部手段分段——(a) 单独计时的 .tmp 小程序调用公开函数（如 `load_session_index`、会话列表/回放的读接口）测各步耗时，作为分解；(b) `strace -f -tt -e trace=openat,read,fsync` 观察文件访问时序（可选）。
- 若某关键步骤只能靠新加 span 才能看清，只记录为建议，不加入产品代码。

## 语料与仓库
- 会话库：`.tmp/wp102/corpus/sessions.db`（25MB）。以只读 `mode=ro` 打开统计 sessions/transcript_items 行数和字节数，只报聚合数，session id 用 sha256 前 8 位表示，不输出任何消息文本。测量时用副本，不改原件。
- 大仓：`/workspace/litecode`（含全量索引）；小仓：`.tmp/wp156/small_repo`（新建的十余个文件小 git 仓库）。两仓使用同一份会话库副本（会话按工作区过滤的影响单独注明）。

## 每步记录
源码位置、冷(C1/C2)/热耗时中位数、是否在 T0→T_int 关键路径上、阶段 P0/P1/P2 及理由、复跑命令。挡在关键路径上但应延后的步骤单独列表。

## 护栏
MemAvailable ≥ 6GiB 且无其他 cargo/rustc 时才编译/测（每分钟轮询，最多约 30 分钟，否则上报）；-j2；共享主 target；HOME=$PWD/.tmp/home，unset RUSTUP_HOME/CARGO_HOME；与 IDE/前端错开编译；大仓运行时 RSS watchdog（`.tmp/wp102/mem_sample.py` 同类脚本）；只读，不改产品/测试代码，临时脚本只放 .tmp/wp156，封包时清理；不 commit。产物写 `.tmp/wp154/session.md`。

---
# W4 运行清单（后端中层裁定后，10-14 22:56）
前提：IDE 给出 W1 release 二进制路径和 commit（`WP156_BIN`），W3 后给出大仓种子副本路径。debug 版只用于自测脚本，不进数字。每步开始前：MemAvailable ≥ 6GiB、无其他 cargo/rustc；W4 内不编译。冷启动逐出见 `.tmp/wp154/evict.md`（harness 每次运行自动逐出二进制、模型、工作区，并把 mincore 读数写进结果）。
公共环境：`cd /workspace/litecode/.tmp/wp156; export WP156_BIN=<IDE release>; export WP156_TOKEN=$(python3 -c 'import secrets;print(secrets.token_hex(16))')`（token 只在本 shell 中，不落盘；serve 要求鉴权，客户端带 `?token=` 并发 `auth{token}`，记录 auth 回包）。

0. **自检**（不计数）：`git -C /workspace/litecode rev-parse --short HEAD` 与 IDE 的 commit 核对；`$WP156_BIN --version`。
1. **建 C2 会话索引**（先建完再计时）：
   - 小仓：`python3 harness.py corpus C1 1 0`，用这一次生成 ws_corpus；之后单独起一次 serve，轮询 `ws_corpus/.litecode/session-index/meta.json`，直到 `building=false` 且 `job.json` 显示 done，再 SIGTERM。记录耗时和 RSS。empty 组同理（索引很小）。
   - 大仓：在种子副本上同样起 serve 建会话索引（代码索引用 IDE 种子），记录耗时和峰值 RSS。
2. **小仓 A/B**（每档 3 次，每次加 3 次热重连）：
   - A（新 profile，解锁 = session/new 回包）：`WP156_PATH=A python3 harness.py empty C1 3 3`；`WP156_PATH=A python3 harness.py empty C2 3 3`
   - B（恢复布局，解锁 = session/list 回包中出现 28396d73）：`WP156_PATH=B python3 harness.py corpus C1 3 3`（C1 每次重新复制，不含索引）；`WP156_PATH=B python3 harness.py corpus C2 3 3`（使用第 1 步建好的索引）
   - 注意：C1 会重建工作区，C2 必须在第 1 步之后、且不再跑 C1 时进行；建议顺序 C2→C1，或 C1 跑完后重做第 1 步。
3. **大仓 A/B × C1/C2**：`WP156_WS=<种子副本> WP156_FRESH_CMD='<把种子副本还原为"无会话索引"状态的命令，由 IDE 种子说明确定>'`，然后同第 2 步四条命令（group 用 corpus 或 empty 视副本中 sessions.db 而定）。运行期间开 RSS 监控（harness 记录 VmHWM，另用 `.tmp/wp102/mem_sample.py` 同类脚本每秒采样）。
4. **auth 记录**：每次运行的 events 中 `reply:auth` 行（预期 err=-32601，见 connection.rs:1449-1456）。结果中给出：相对 T0 的时刻；前端是否等它（web/src/api/agentWs.ts:136-141 是发完即走，不登记等待回包：核对 sendRpc/send 的区别）；以及它是否排在 session/list 或 new 的回包之前，从而占用关键路径时间。
5. **汇总**：每步中位数和原始 JSON（logs/*.json），serve 日志中的 tracing 时间（LITECODE_LOG=debug）与客户端时间线对齐；结果写入 `.tmp/wp154/session.md`。
6. **收尾**：删除 ws_*、logs 中的临时副本和 home；保留 `.tmp/wp154/mincore.py`、`evict.md`。
