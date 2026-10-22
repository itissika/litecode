# WP-208 coverage-map（删减后核对，基准 3acfe31 → tip）

> 依据 `core-chains.md`（六域人工核对 + GM 决议）与 `protected.txt`（764 条）。初稿版 coverage-map 已作废，本文件覆盖。

## 1. 每条链路守护数：删前 → 删后

| 模块 | 链路 | 删前 | 删后 | 一致 |
|---|---|---:|---:|:-:|
| tools | Tools·C1 read：1-based start_line/limit 分页，默认页有界并提… | 8 | 8 | ✓ |
| tools | Tools·C2 grep：结果为有界 hits（limit 触顶进 message、不落 spi… | 7 | 7 | ✓ |
| tools | Tools·C3 glob：无 / 的模式按文件名任意深度匹配；含 .. 的模式 invalid_… | 5 | 5 | ✓ |
| tools | Tools·C4 write：expected_hash 不符 → stale 且不落盘、缺文件带… | 6 | 6 | ✓ |
| tools | Tools·C5 edit/apply_edits（WP-198 契约）：扁平 old/new，无… | 12 | 12 | ✓ |
| tools | Tools·C6 run/wait/kill：短命令前台返回 exit_code；超窗返回 exi… | 11 | 11 | ✓ |
| tools | Tools·C7 LSP 七工具（WP-202）：七个 snake_case 名且仅 Code 面… | 11 | 11 | ✓ |
| tools | Tools·C8 webfetch/websearch（WP-203）：SSRF 拒 loopba… | 9 | 9 | ✓ |
| tools | Tools·C9 run_code 结果面（WP-200）：只渲染 head+logs+resul… | 11 | 11 | ✓ |
| tool | Tools·C10 Code 面投影与路由：Code 工具直调被拒（授权前）并经 run_code … | 9 | 9 | ✓ |
| tool | Tools·C11 嵌套调用取消/超时/grace 封口：取消封住在途嵌套调用与写；超 grace … | 8 | 8 | ✓ |
| tool | Tools·C12 执行器与入参校验：已提交结果优先于超时/迟到取消；阻塞工具被管线超时打断；Cod… | 8 | 8 | ✓ |
| tool | Tools·C13 资源锁：读共享/写独占，等待中的写不被后来读插队，同 key 写按提交序，多 k… | 15 | 15 | ✓ |
| permission | Tools·C14 权限底线：floor（危险命令/敏感写）在 all 预设与 always 授权下… | 28 | 28 | ✓ |
| mcp | Tools·C15 MCP：每连接仅一次 initialize；JSON-RPC 读跳过通知/他 i… | 12 | 12 | ✓ |
| tool | Tools·C16 契约快照：run_code SDK 文本+sha 与 fixture 逐字一致；… | 3 | 3 | ✓ |
| tools | Tools·C17 底层文件原子性（write/create_only 依赖）：并发 create/… | 3 | 3 | ✓ |
| tools | Tools·C18 检索类工具（code_search/session_search）：引擎非 Wa… | 7 | 7 | ✓ |
| ptc | Runtime·C1 PTC 值边界：JS→JSON 时 undefined 在参数中静默丢弃、在结果… | 6 | 6 | ✓ |
| ptc | Runtime·C2 PTC 预算/取消：死循环被 interrupt 打断；等待宿主/审批时暂停计费… | 7 | 7 | ✓ |
| ptc | Runtime·C3 PTC 成功不被改写：程序已 return 后 drain 遇超时/取消保留返回… | 5 | 5 | ✓ |
| ptc | Runtime·C4 PTC 栈/内存封口：栈溢出与 OOM 被 JS catch 后仍封为 Reso… | 8 | 8 | ✓ |
| ptc | Runtime·C5 PTC 沙箱：无 fetch/require/process/setTimeou… | 1 | 1 | ✓ |
| ptc | Runtime·C6 工具错误语义：ToolError 可 catch 且 instanceof；未 … | 6 | 6 | ✓ |
| ptc | Runtime·C7 输出预算：log 保前缀、最终值计入预算、截断只保可容纳前缀；进模型请求的结果受… | 6 | 6 | ✓ |
| agent | Runtime·C8 Agent loop：纯文本即停；function_call 先于 output… | 9 | 9 | ✓ |
| runtime | Runtime·C9 流式投影：流式 item 落到它打开的行；未确认 item 封 incomple… | 7 | 7 | ✓ |
| runtime | Runtime·C10 请求前缀稳定（缓存）：已发出的请求前缀在 turn 内及跨 turn 重载不被改… | 8 | 8 | ✓ |
| runtime | Runtime·C11 排队消息/steer/stop_and_send：pending 在 tool … | 10 | 10 | ✓ |
| runtime | Runtime·C12 自动 turn 唤醒：无订阅者不预留、忙时留 mailbox、UI kill 后… | 6 | 6 | ✓ |
| subagent | Runtime·C13 子会话生命周期/隔离：launch 建持久子会话并挂父链接；父事件通道不收子事件… | 8 | 8 | ✓ |
| subagent | Runtime·C14 子代理并发：并发 send 只有一个赢；忙时排队不起 turn；拒绝未知/他人 … | 8 | 8 | ✓ |
| terminal | Runtime·C15 agent bash 作业：前台超时 promote 不杀、promote 后退… | 8 | 8 | ✓ |
| terminal | Runtime·C16 终端安全/隔离：agent kill 拒绝人类 PTY 与猜测 id；id 为不… | 6 | 6 | ✓ |
| 架构守卫 | Runtime·C17 架构/启动死名单（源码静态约束）：分层不越界；PTY 符号仅在 terminal… | 12 | 12 | ✓ |
| session/data | Session·C1 Session 数据层事务：创建/追加/删除幂等、乐观并发、故障回滚：每次变更走… | 9 | 9 | ✓ |
| session/data | Session·C2 seq 单调与高水位（不复用已删 seq）：next_seq = max+1 持… | 7 | 7 | ✓ |
| session/data | Session·C3 final 不可改 + 崩溃封口（冷恢复）：final 行永不重写；重启时 in… | 5 | 5 | ✓ |
| session/data | Session·C4 compact 检查点与 fold 一致性：compact 只追加 checkp… | 7 | 7 | ✓ |
| client_protocol | Session·C5 线协议 seq 身份 / snapshot 游标：wire 只发新 seq 行，… | 7 | 7 | ✓ |
| session/manager | Session·C6 Turn 生命周期（单 turn 互斥、取消、预约、revert/compact… | 9 | 9 | ✓ |
| session/manager | Session·C7 待发队列（pending messages）：enqueue/claim 不写 … | 5 | 5 | ✓ |
| client_protocol | Session·C8 权限往返与 pending_permissions（WP-186）：grant … | 11 | 11 | ✓ |
| session/manager | Session·C9 子会话只读与删除级联（wp091/wp099）：用户对 child 的 run/… | 7 | 7 | ✓ |
| client_protocol | Session·C10 会话列表/冷启动可见性：空新会话隐藏，有用户消息的会话在冷启动/坏行/缺 blo… | 5 | 5 | ✓ |
| session/manager | Session·C11 set-model（模型切换持久化）：模型切换与 control-plane 变… | 4 | 4 | ✓ |
| serve | Session·C12 serve 绑定与 HTTP/WS 鉴权：非 loopback 必须 requi… | 6 | 6 | ✓ |
| config | Session·C13 全局配置 DB：迁移/seed/修复：迁移 v0→current 幂等且只增不删… | 7 | 7 | ✓ |
| config | Session·C14 Settings 写入：turn 期间阻塞、混合提交回滚、密钥掩码、即时生效：t… | 6 | 6 | ✓ |
| context_pipeline | 上下文·C1 装配 / view：内存工作集必须和 DB fold 一致；pending 尾巴… | 3 | 3 | ✓ |
| context_pipeline | 上下文·C2 view 补齐未答 call：只在 LLM 视图里补，不落盘… | 2 | 2 | ✓ |
| context_pipeline | 上下文·C3 budget：优先用 provider bill，前缀之外的后缀才估算；comp… | 3 | 3 | ✓ |
| context_pipeline | 上下文·C4 触发 auto compact：阈值按真实用量判断，低 bill 不会因为本地估… | 3 | 3 | ✓ |
| context_pipeline | 上下文·C5 manual compact 门槛：30% 含边界；manual 绕过 auto… | 2 | 2 | ✓ |
| context_pipeline | 上下文·C6 keep_recent 切点：不拆开 tool 对和 assistant 段，切… | 4 | 4 | ✓ |
| context_pipeline | 上下文·C7 compact 只吃已持久化的前缀，保留未提交尾巴；重建时 call_id 不重… | 3 | 3 | ✓ |
| context_pipeline | 上下文·C8 compact 请求：超时、取消、关 thinking、用调用方给的 cap；只… | 4 | 4 | ✓ |
| context_pipeline | 上下文·C9 compact 失败回滚：transcript/工作集恢复到快照，lease 释… | 5 | 5 | ✓ |
| context_pipeline | 上下文·C10 compact 已持久化但 reload 失败：以 DB 为准，报 Succee… | 1 | 1 | ✓ |
| context_pipeline | 上下文·C11 summary / rebased：summary 是 assistant 消息… | 4 | 4 | ✓ |
| context_pipeline | 上下文·C12 media_budget：工具 media 被模型看过一次就丢；用户 media… | 5 | 5 | ✓ |
| reminder | 上下文·C13 reminder 注入：compact 后 restore 只触发一次；remi… | 5 | 5 | ✓ |
| reminder | 上下文·C14 mentions reminder：user 行后面跟一条隐藏 reminder… | 2 | 2 | ✓ |
| context_pipeline | 上下文·C15 system/env 装配：AGENTS.md 和 CLAUDE.md 的拼接顺… | 3 | 3 | ✓ |
| engines/session_search | 上下文·C16 session_search 索引：stale 索引在查询前被追上；索引坏了要报… | 4 | 4 | ✓ |
| engines/session_search | 上下文·C17 session_search 检索/呈现：能命中种子文本，scope 和 exc… | 4 | 4 | ✓ |
| engines/code_search | 上下文·C18 code_search 索引持久化：多文件原子写，崩溃后可恢复；commit 和… | 3 | 3 | ✓ |
| engines/code_search | 上下文·C19 code_search IPC：worker 全生命周期；fs 通知后能搜到新文… | 4 | 4 | ✓ |
| engines/code_search | 上下文·C20 code_search 可选降级：engine 关掉或 worker 崩溃时降级… | 5 | 5 | ✓ |
| engines/code_search | 上下文·C21 code_index tick：tick 只做同步，不 build 也不 coo… | 4 | 4 | ✓ |
| knowledge | 上下文·C22 knowledge 加载：增量结果等于从零重建；turn 只读快照；状态分类… | 5 | 5 | ✓ |
| context_pipeline | 上下文·C23 revert 与上下文一致：revert 后提交会丢弃未提交 delta，不重放… | 3 | 3 | ✓ |
| llm | Provider·P1 SSE line reader (`src/llm/codec/sse.rs`)… | 6 | 6 | ✓ |
| llm | Provider·P2 Stream contract — function-call event ga… | 5 | 5 | ✓ |
| llm | Provider·P3 Stream contract — accumulator, terminals… | 8 | 8 | ✓ |
| llm | Provider·P4 HTTP send — retry, cancel, empty-stream … | 6 | 6 | ✓ |
| llm | Provider·P5 Reconnect UI — compaction turn (`src/llm… | 1 | 1 | ✓ |
| llm | Provider·P6 Responses codec — streaming product path… | 7 | 7 | ✓ |
| llm | Provider·P7 Responses codec — request encode & reaso… | 3 | 3 | ✓ |
| llm | Provider·P8 Responses stream hardening (`src/llm/cod… | 3 | 3 | ✓ |
| llm | Provider·P9 Messages codec — wire encode & tool pair… | 3 | 3 | ✓ |
| llm | Provider·P10 Messages codec — streaming (`messages.rs… | 5 | 5 | ✓ |
| llm | Provider·P11 Messages SSE → Responses events synth (`… | 3 | 3 | ✓ |
| llm | Provider·P12 Chat completions codec — encode, replay,… | 6 | 6 | ✓ |
| llm | Provider·P13 Chat SSE synth (`chat_synth.rs`)：Chat co… | 1 | 1 | ✓ |
| llm | Provider·P14 Cross-item replay compatibility (`replay… | 5 | 5 | ✓ |
| provider_catalog | Provider·P15 Provider catalog — load, resolve, strict… | 6 | 6 | ✓ |
| 集成（provider 产品路径） | Provider·P16 Agent e2e — Responses codec on product r… | 3 | 3 | ✓ |
| 集成（provider 产品路径） | Provider·P17 Turn prefix stability — prompt-cache ide… | 1 | 1 | ✓ |
| session/snapshot | Session·C15 snapshot 文件回滚（issue_032，GM 定为核心）：只恢复 pat… | 4 | 4 | ✓ |
| workspace | IDE·L1 Agent/LSP/人工 API 路径解析边界：相对路径带 `..` 时所有模式… | 7 | 7 | ✓ |
| workspace | IDE·L2 symlink 逃逸（目标文件还不存在时）：Safe 模式按"最深的已存在祖先"… | 3 | 3 | ✓ |
| workspace | IDE·L3 Windows 下的 Unix 风格路径：`/etc/passwd` 不能被误判… | 3 | 3 | ✓ |
| workspace | IDE·L4 人工 HTTP 和 git 接口越界返回 403：Sandbox::resolv… | 6 | 6 | ✓ |
| workspace | IDE·L5 复制 symlink 不外泄：复制或保留 symlink 时，被指向的对象要过沙… | 3 | 3 | ✓ |
| workspace | IDE·L6 引用/预览不泄露文件内容，越界引用被隐藏… | 3 | 3 | ✓ |
| workspace | IDE·L7 LAP 统一形态（verbatim、UNC、canonicalize 白名单）：… | 18 | 18 | ✓ |
| lsp | IDE·L8 LSP 输出路径：工作区内的路径显示为相对路径，工作区外保留绝对路径；displ… | 5 | 5 | ✓ |
| lsp | IDE·L9 LSP URI 与盘符：file URI 去掉 verbatim 前缀、盘符不区… | 5 | 5 | ✓ |
| lsp | IDE·L10 诊断时效（不回显过时的错误）… | 13 | 13 | ✓ |
| lsp | IDE·L11 ContentModified 处理：只认错误码 -32801，最多取消一次；透… | 5 | 5 | ✓ |
| lsp | IDE·L12 LSP 进程生命周期：先 sweep 再 spawn；hub 被 drop 时杀… | 11 | 11 | ✓ |
| lsp | IDE·L13 冷启动与 hub 激活：probe 推迟到第一次真正需要时才跑；probe 失败… | 8 | 8 | ✓ |
| lsp | IDE·L14 项目根选择… | 4 | 4 | ✓ |
| optional | IDE·L15 可选引擎懒构建与 reconcile… | 10 | 10 | ✓ |
| workspace | IDE·L16 原子写：写入时保留 symlink 节点、权限位和硬链接；create-only… | 5 | 5 | ✓ |
| workspace | IDE·L17 工作区进程契约：不 chdir；同一个根只能加一把锁；agent 的 cwd 取… | 4 | 4 | ✓ |
| workspace | IDE·L18 watcher 与 git 环境隔离… | 6 | 6 | ✓ |

- 守护合计：665 → 665（全部一致）
- 回归合计：183 → 183
- protected.txt：764 → 764

## 2. 各域测试数（源码计 `#[test]`/`#[tokio::test]`）

| 域 | 删前 | 删后 | 删除 |
|---|---:|---:|---:|
| Tools | 616 | 191 | 425 |
| Runtime | 263 | 119 | 144 |
| Session | 613 | 155 | 458 |
| 上下文 | 567 | 94 | 473 |
| Provider | 242 | 80 | 162 |
| IDE | 364 | 126 | 238 |
| 范围外 | 4 | 4 | 0 |
| **合计** | **2669** | **769** | **1900** |

`cargo test -- --list`（lib + tests）：2714 → 822；lib：2212 → 535。全量 `--no-fail-fast`：822 passed / 0 failed（`cargo-full-post.txt`）。

说明：源码计数与 `--list` 的差来自 `tests/common` 中测试被每个集成二进制重复计入、以及 `#[cfg(windows)]` 测试在 Linux 上不列出。范围外仅 4 条（tests/common/permission.rs 2、tests/session_data_consistency.rs 2），未动。

