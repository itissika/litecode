# WP-084-research · subagent session 四类操作调研（WP-085 计划）

- 角色：backend_mid；父 WP-084（gm，Andy 开）；本包 WP-085（direct，approver=gm）。前端部分由前端中层配合。
- 基线：分支 `ops/team-framework`；**只读**，不改 `src/`、`tests/`、`web/`，不 commit，不碰 `.github/workflows/windows-*.yml`。
- 时间：2026-10-13（HKT）。本文件为调研报告（计划 §1-§3、§5-§7 保留；§4 已填结论；新增 §8 D 汇总）。来源：WP-085-session.md（Session）、WP-084-research-runtime.md（Runtime）、Tools/前端中层消息回报（全文并入本文）。

## 1. Goal
针对 subagent session 的 **查消息 / 发消息 / 停止 turn / 删除** 四类操作：盘清各入口现状与缺口、父子生命周期一致性，参考成熟实现给最小补全集 + 分工 + 可钉死验收，按五档分类并列出需 Andy 定的产品未定项。

## 2. 形式
kind=direct：不拆子包；各域 bot 按下表领题做只读调查，回报给 backend_mid，中层汇总。理由：纯调研、无代码改动、无文件竞态。

## 3. 初扫定位（rg，待各域核实）
| 域 | 位置 |
|---|---|
| Tools（父 agent 工具） | `src/tools/subagent/{launch,send,stop,status,wait,list,turn,jobs,hub,spawn_contract}.rs`、`src/tools/descriptions/subagent_launch.md`、`src/tool/{agent_bindings,authorize,availability,registry}.rs`、`src/permission/{engine,sinks,presets}.rs` |
| Session（client_protocol/serve/session/CLI） | `src/client_protocol/protocol.rs`（:24 AGENT_SUBAGENT_BOUND、:192 running_with_subagent、:203 parent 标记）、`connection.rs`（:340 "cannot retry a subagent session"、:1320 child_bindings）、`controller/session_lifecycle.rs`（:357 delete_session、:376-377 purge_parent/forget_child）、`controller/mod.rs`、`project.rs:471`；`src/serve/state.rs:130-143`、`src/serve/settings.rs`；`src/session/{live,model,manager}.rs`、`src/session/DATA.md`；`src/cli/` |
| Runtime | `src/runtime/subagent_auto_turn.rs`、`pending_auto_turn.rs`、`exec.rs`、`observer.rs`、`mod.rs`；`src/agent/`、`src/types/`；`src/reminder/sources/subagent.rs` |
| 前端中层（web/desktop） | `web/src/components/{SubagentReadOnlyContent,SubagentStatusIcon,SessionItem,SessionList,AgentChatInput,SessionStatusLine,sessionTeardown}.tsx/ts`、`web/src/lib/{subagentUi,sessionPanelNav,centreChat,transcriptProjection}.ts`、`web/src/stores/{sessionStore,messageStore,connectionStore}.ts`、`web/src/api/{types,adapter}.ts`；`desktop/` |

## 4. 结论（档位：1 产品未定 / 2 产品缺陷 / 3 架构 / 4 常见 bug / 5 边角 bug）

### A. 四类操作 × 入口矩阵
| 入口 | 查消息 | 发消息 | 停 turn | 删除 |
|---|---|---|---|---|
| 父 agent 工具 | 部分：只有最终 output+transcript 路径 | 部分：仅子空闲时 | 能（停 turn，保留会话） | 不能 |
| client_protocol /ws | 能 | 部分：子在跑→入队 seam 注入；子空闲→静态推断报 not a primary agent（见 §4.X） | 能（但 cancelled:true 可能失真） | 能（不区分子） |
| HTTP | 无（设计如此） | 无 | 无 | 无 |
| CLI | 仅 `resume <子id>` 理论可看 | 未定义 | 不能 | 不能 |
| Web/desktop | 能（只读视图） | 不能（不渲染输入框） | 不能 | 不能 |

**A1 父 agent 工具（Tools）** — 部分。只有 launch/send/wait/stop/list 五个（`src/tools/subagent/mod.rs:1`），status.rs 仅为输出格式化，无删除工具（`mod.rs:11-27`）。list 返回 id/state/轮次年龄/步数/上轮结束原因，无消息内容（`list.rs:13,55-61`）；wait 等子当前 turn，不取消子（`wait.rs:46,114-147`）；结果带最后一轮 output（>24k 截断）+ transcript 路径与行号（`status.rs:57-112`），无增量。send 仅子空闲时起新 turn，子在跑报 "already running"，不排队（`send.rs:63-67,98-101`）。stop 走 `cancel_turn_sync` 停当前 turn、保留会话（`stop.rs:47-75`）。缺口：父看不到进度、不能 steer、无清理路径。修法：send 加 `mode: queue|interrupt`（Codex V1）；按范围读子 transcript；删除交 Andy。档位：1（删除/排队）+2（无进度）。

**A2 client_protocol（Session）** — 部分。查：`session/subscribe`（connection.rs:643）、`session/snapshot`（:834）、buffer load（:1305-1330，带 subagent_bindings）不区分子。发：`agent/run`（:181）、`session/pending/enqueue`（:418）不查 is_child_session；仅 `agent/retry` 拒子（:334-340），`agent/set_primary` 拒子（session_lifecycle.rs:512）。停：`agent/cancel`（:385-404）→ `cancel_turn`（manager.rs:1122-1138），**无条件回 `{"cancelled":true}`**。删：`session/delete`（:738）→ `delete_session`（session_lifecycle.rs:357）。缺口：父工具与用户直发无协调；cancel 回包失真。修法：Andy 定直发策略后在 run/enqueue 统一放行或拦截；cancel 回真实结果。档位：直发 1；回包失真 5。

**A3 HTTP（Session）** — 无，设计如此：router.rs:69-75 只有 /health、/ws、/api/workspace、/api/settings、/api/media。不加路由。档位：不适用。

**A4 CLI（Session）** — 基本不能。仅 list-sessions、resume、serve（cli.rs:46-58,:696），src/cli 无父子逻辑。缺口：看不出父子，`resume <子>` 行为未定义（未实测；按 §4.X 推断子空闲时发消息会报错）。修法：最小做法 list-sessions 标 parent、resume 子进只读并提示。档位：1。

**A5 Web/desktop（前端中层）** — 能看，不能发/停/删；desktop 复用 web（desktop/src 无相关代码）。只读视图 `web/src/components/SubagentReadOnlyContent.tsx:23-42`；入口 `SubagentLaunchToolView.tsx:17-37`、`SubagentInlineLine.tsx:45-47`、`SubagentRosterPanel.tsx:221`、`transcriptMarks.tsx:160`，经 `sessionPanelNav.ts:122-140`。子不渲染输入框（`AgentChatInput.tsx:576-616`）；`agent/cancel` 只在主会话（`turnStore.ts:638`），`SessionStatusLine.tsx:487,574` 子会话关 Kill；`SessionList.tsx:12` 只列根，无删子入口；父 running_with_subagent/stopping 不能删（`SessionItem.tsx:58-60`）。修法：子底栏 + Workers 行加停止（复用 `agent/cancel{child}`）；Workers 行加删除带确认、在跑置灰；发消息看 D4。档位：1。

**A6 Runtime 外部取消（Runtime）** — 部分。唯一原语 `cancel_turn_sync` 只处理 RunningTurn（manager.rs:1128-1139）；入口：AGENT_CANCEL（connection.rs:384-401）、连接级 Cancel/Quit（:1636-1660,:1780-1805）、subagent_stop（stop.rs:47-54）、revert（manager.rs:1475-1498）。缺口：①`StartingTurn` 预约窗口无 token（manager.rs:850-888、1028-1068），取消静默丢失而 RPC 仍回 cancelled:true；②subagent_stop 先查后取消非原子（stop.rs:47-75）；③取消后 pending 队列被 `pending_auto_turn` 立即再起 turn（"停了又跑"）。修法：StartingTurn 加 cancel_requested 标志、start_turn 装 token 时立即生效；返回 running/starting/idle；stop 锁内判定。档位：①②5；③1。

> **cancelled:true 失真（Session + Runtime 共同发现）**：`agent/cancel` 回包恒为 true（connection.rs:384-404），无 turn 时、以及 StartingTurn 窗口内取消被丢弃时都会误报。合并为一条 5 档 bug（D1-R1）。

### §4.X 冲突裁定：用户直接给子发消息
静态读码（未实测）结论：**两方各对一半，取决于子是否在跑**。
- 子**在跑**：`connection.rs:496-500` `enqueue_user_input` 不拦子，`exec.rs:135-156` 在下一 step seam 注入 → **Runtime 正确**（steer 生效）。
- 子**空闲**：`agent/run` → `launch_turn` → `session_lifecycle.rs:110-113` `resolve_primary_agent` → `runtime/mod.rs:228-237` 校验 role==Primary。子会话 `record.agent_id` 由 `reserve_turn` 写成子 profile（manager.rs:855,875），其 role 为 Subagent → 报 **"agent '<x>' is not a primary agent"** → **Tools 正确**。父工具路径不受影响，因为它用 `AgentIdentity::Named`（runtime/mod.rs:333-341,441-451）绕过校验。
- 附带新缺口：子在跑时入队、但若在 turn 末 seam 之后才到，残留队列由 `pending_auto_turn.rs:61-65` 再走 `resolve_primary_agent`，失败返回 `SkippedSessionGone`——**消息静默滞留**；idle 竞态分支（connection.rs:501-512）同理会失败。档位 2（入口半通半不通）+ 5（滞留）。
- 前端不渲染子输入框（AgentChatInput.tsx:576-616），所以今天 UI 走不到；只有协议直连能触发。
- 未核实：进程重启后子 record 的 agent_id 是否从 meta 恢复为子 profile（若回落为 workspace default primary，则空闲子可被以**主 agent 身份**跑起来，更糟）。需 Session 写测试实测。

### B. 父子生命周期与一致性
**B1 停父是否级联（Runtime 主 + Session）** — 不级联，设计如此（`launch.rs:129-132` 注释；`cancel_turn_sync` 只取消单 token，manager.rs:1128-1137）。父 wait 返回 "subagent_wait cancelled"，子继续（wait.rs:114-115）。`running_with_subagent` 为读时聚合（manager.rs:81-89,1342-1369），无 set/clear 点。父停后子完成仍入父 inbox，父有 UI 订阅则被 auto-turn 自动唤醒（hub.rs:94-108、subagent_auto_turn.rs:40-48）。例外：连接级 Cancel 遍历本连接所有 projection（connection.rs:1645-1660），子被 UI 打开过就会被一起停。router `Lagged` 只 warn，completion 丢（hub.rs:114-116）。缺口/档位：不级联本身 1；无"停整棵树" 1；连接级 Cancel 范围绑 projection 3；停父后被子结果自动唤醒 1；聚合态变化是否推 session/status 未核 4（暂定）；Lagged 丢 completion 5。修法：保留默认不级联 + 公开 `cancel_subtree`（复用 `cancel_descendant_turns`）；连接级 Cancel 只停指定 sid/树；用户停父后抑制 auto-wake 至下次用户输入；Lagged 时按 data 重扫补投。

**B2 删父（Session）** — 子在跑时整个删除被拒（`try_begin_operation(Delete)`，manager.rs:1614-1616；测试 :3816-3840），返回 -32000 "agent already running"，用户看不出是子在跑。磁盘 SQLite 级联删（manager.rs:1901-1907、data/sqlite/session.rs:1534,~1565-1586），每子发 SessionRemoved（manager.rs:1909-1917）。缺口：①提示不准（1 定策略；保持拒绝则改 `descendant_running` 结构化错误）；②级联删掉的子孙未调 purge_parent/forget_child，hub/jobs 残留（session_lifecycle.rs:376-377）5；③`cleanup_deleted_files` 错误被 `let _` 吞 5。修法：remove_session 返回全部被删 id 逐个清理；失败 warn。

**B3 删子后父的显示（Session + 前端）** — 后端：bindings/child_session_id 按 sessions 表实时查（read.rs:338,350；manager.rs:2010-2020；controller/mod.rs:107-111），子行删后 launch 行成悬空调用，无失效通知；只清 inbox（session_lifecycle.rs:357-377、subagent/jobs.rs:64-77）。删子不查父状态，父正在 `subagent_wait` 该子也能删（wait 会报 removed，hub.rs:111-114、wait.rs:121-131）。前端 deleted 处理（sessionStore.ts:388-395）：不关 `subagent-<id>` 面板（5）；卡片回退 accepted 且可点（subagentUi.ts:37-55、SubagentInlineLine.tsx:37-49）（5）；Workers 名单幽灵 idle 行并计数（SubagentRosterPanel.tsx:168-185、SessionStatusLine.tsx:169-175）（**4，每次删除必现**）。修法：后端 bindings 返回墓碑 `{child_id, deleted:true}` 或 deleted 事件带 parent_session_id；前端记已删集合、卡片显示"已删除"不可点、名单/计数排除、关面板。显示语义 1；父在等时删子 2。

**B4 子在跑时外部发消息（Runtime + Tools）** — 两入口不一致：父 `subagent_send` 拒绝（send.rs:63-67,98-101，文案 "already running a turn…"）；用户直发入队、seam 注入（connection.rs:496-500、exec.rs:135-156）；空闲时用户直发见 §4.X 报错。均无打断。修法：父与用户共用一套语义：`queue`（默认）/`interrupt`（cancel+入队，依赖 A6 修复），消息带来源。档位：语义 1；不一致 2。

**B5 重连/重放（Session + 前端）** — 部分。后端：buffer load 每次从 DB 重建 bindings（connection.rs:1320-1326），SubagentBound 是 live 事件不落盘（project.rs:471-479），可由 bindings 等价补齐；`list_sessions` 跳过空 session，刚 launch 的子在首个事件前列表里看不到（session_lifecycle.rs:~412）5。前端：状态补回依赖 SessionList 挂载（SessionList.tsx:37-41），侧栏收起+名单关闭（SubagentRosterPanel.tsx:128-135）时子可能永远显示 running 5；lifecycle 无 running_with_subagent，turn_finished 一律 idle，孙在跑时子显示 idle 5。修法：前端 connected 时统一 listSessions；后端 lifecycle 事件带原始 status；list_sessions 不过滤有 parent 的子。未核实：未实测；permission_request 不落盘、重连不可重建（WP-081 已记既有行为）。

**B6 权限归属（Tools）** — 父 launch/send 起的子 turn 固定 `deny_permission_sink`（turn.rs:6,37；launch.rs:18,91），所有 ask 自动拒绝、无人审批 2。auto-turn 用 `last_permission_sink`，无则 deny（subagent_auto_turn.rs:74-75；pending_auto_turn.rs:99-100），行为取决于人类是否曾直接跟该子说过话（launch_turn 写 last sink，session_lifecycle.rs:99-100）——隐式状态 3。depth≥1 去掉 subagent/plan/todo（runtime/mod.rs:451-458）。用户直发 profile 问题见 §4.X。修法：照 Claude Code 把子的权限请求转到父会话审批卡并标注子（Esc 只拒这一次）；sink 由会话类型决定而非 last sink。

**B7 子结果回流父（Runtime + Tools）** — completion router 对任何子 TurnFinished 都推父 inbox，不区分发起者（hub.rs:75-110），用户发起的子 turn 也唤醒父（subagent_auto_turn.rs:192-199）；wait 开始时快照子 turn_id（wait.rs:73-76,119-127），子空闲时取 latest（:80-88），即可能拿走用户 turn 的结果；用户 steer 注入父派发 turn 时结果无标注。删子时 forget_child、wait 报 removed（hub.rs:111-114、wait.rs:121-131）。修法：TurnFinished/CompletionRef 加 `origin: parent_tool|user|auto`；父 inbox 默认只收 parent_tool，用户来源改为不唤醒的提示；wait 只匹配父派发 turn。档位：1；Andy 判不该唤醒则 2。

### C. 成熟实现参考
**C1 Claude Code（Tools；官方文档 sub-agents/tools/agent-view/agent-sdk，2026-10 抓取，未实跑）**：子 transcript 单独存 `agent-{id}.jsonl`，用户在 /tasks 打开；父只拿最终报告。父用 SendMessage 续跑已完成/已停的子；用户可在子视图直接输入，Ctrl+Enter 让运行中的子尽快读到（≈排队+seam 注入）。停：父 TaskStop、用户 /tasks 按 x；**用户停的子父不能自动续跑**。删：无删除工具，transcript 保留 30 天自动清理。权限：后台子的请求出现在主会话并标注子，Esc 只拒这一次（v2.1.186 之前自动拒绝，同我们现状）。默认最多 3 层、并发 20。

**C2 Codex CLI（Runtime；上游代码/PR，2026-10）**：V1 `spawn_agent/send_input/wait_agent/close_agent/resume_agent`，`send_input.interrupt=true` 打断，默认排队；`close_agent` 关后代（仅据描述）。V2 `send_message`（排队不触发 turn）/`followup_task`/`interrupt_agent`（只打断当前 turn，代理保留）/`list_agents`。interrupted 为**非终态**：wait 不返回、不通知父（#13850）。V2 子线程只读，`turn/start`、`turn/steer` 拒子（PR #33841），社区要求放开（#33885）。停父不级联（#31178，未在源码定位）。

**C3 其他（前端中层，2026-10-13 搜索）**：OpenCode 可进入/切换/返回子会话（opencode.ai/docs/agents、anomalyco/opencode#15972；发/停/删未核）。Roo Code 可跳到/取消子任务并通知父，删父级联并提示数量（Roo-Code PR #10864、#8505）。Cline 只状态行显示，任务级取消（docs.cline.bot/features/subagents）。Cursor 云 agent：follow-up 忙时 409 agent_busy、可取消、UI 仅归档、永久删除只走 API（cursor.com/docs/cloud-agent/api/endpoints）。

## 8. D 汇总（backend_mid）

### D1 最小补全集与分工
"无论 Andy 怎么定都要做"（缺陷/bug）为 P0；依赖 D4 决策的为 P1，按推荐方案列出。
| # | 内容 | owner | 依赖 |
|---|---|---|---|
| R1 | StartingTurn 加 cancel_requested；cancel_turn_sync 返回 Running/Starting/Idle；`agent/cancel` 如实回 cancelled；subagent_stop 锁内判定 | Runtime（主）+ Session（RPC 回包） | P0 |
| R2 | completion 加 `origin: parent_tool|user|auto`；父 inbox/auto-wake 只认 parent_tool；wait 只匹配父派发 turn | Runtime + Tools | P0（字段）；唤醒策略随 D4-5 |
| R3 | 用户直发子：统一 identity——子会话 turn（launch_turn / pending_auto_turn / idle 竞态）用子 profile（AgentIdentity::Named 或 controller 对子跳过 Primary 校验）；修 pending 滞留 | Session + Runtime | P0（消除半通半不通）；是否开放 UI 随 D4-1 |
| R4 | 子权限 sink 由会话类型决定：转父会话审批卡并标注子（Claude Code）；去掉 last_permission_sink 隐式依赖 | Tools（sink）+ Session（协议字段 child_session_id）+ 前端（卡片标注） | D4-2 |
| R5 | `subagent_send` 加 `mode: queue|interrupt`（默认 queue，复用 enqueue_user_input）；文案统一 | Tools（依赖 R1） | D4-3 |
| R6 | 删除清理：remove_session 返回全部被删 id，逐个 purge_parent+forget_child；cleanup 错误 warn；删父遇在跑后代返回 `descendant_running` 结构化错误 | Session | P0 |
| R7 | 删子墓碑：bindings 返回 `{child_id, deleted:true}` + deleted 事件带 parent_session_id；前端关面板、卡片"已删除"不可点、名单/计数排除 | Session + 前端 | P0（4/5 档 bug） |
| R8 | 重连状态：前端 connected 统一 listSessions；lifecycle 事件带原始 status（含 running_with_subagent）；list_sessions 不过滤有 parent 的子 | 前端 + Session | P0 |
| R9 | 停整棵树：公开 `cancel_subtree`（复用 cancel_descendant_turns）；连接级 Cancel 只停指定 sid/树；用户停父后抑制 auto-wake | Runtime + Session | D4-4/5 |
| R10 | UI：子底栏 + Workers 行"停止"（`agent/cancel{child}`）；Workers 行"删除"（确认、在跑置灰）；子视图输入框（若 D4-1 开放） | 前端 | D4-1/6 |
| R11 | router Lagged 时按 data 重扫补投 completion | Runtime | P0（5 档） |
IDE 域本包无项。CLI（list-sessions 标 parent、resume 子只读）为可选，随 D4-7。

### D2 验收（测试名级，均需在 `cargo test` / `pnpm test` 中可跑）
- R1：`manager::cancel_during_starting_turn_cancels_on_start`、`manager::cancel_turn_sync_reports_idle_when_nothing_running`、`client_protocol_cancel::agent_cancel_reports_false_when_idle`、`subagent::stop_is_atomic_against_starting_turn`
- R2：`subagent::hub::completion_carries_origin`、`subagent::hub::user_initiated_child_turn_does_not_wake_parent`、`subagent::wait::ignores_user_initiated_turn`
- R3：`client_protocol_run::agent_run_on_idle_child_uses_child_profile`（不再报 not a primary agent）、`pending_auto_turn::flushes_child_queue_with_child_profile`、`manager::child_agent_id_survives_reload`
- R4：`permission::child_ask_routes_to_parent_card_with_child_id`、`permission::child_sink_independent_of_last_permission_sink`、前端 `AskPermissionCard.test.tsx › shows originating child`
- R5：`subagent::send::queue_mode_enqueues_while_running`、`subagent::send::interrupt_mode_cancels_then_runs`、`subagent::send::rejects_unknown_mode`
- R6：`session_lifecycle::delete_parent_purges_all_descendant_inboxes`、`session_lifecycle::delete_parent_with_running_child_returns_descendant_running`、`session_lifecycle::cleanup_failure_is_logged`
- R7：`read::subagent_bindings_include_deleted_tombstone`、`client_protocol_lifecycle::deleted_event_carries_parent_session_id`、前端 `sessionStore.test.ts › deleted child closes panel`、`SubagentRosterPanel.test.tsx › excludes deleted child from rows and count`、`SubagentInlineLine.test.tsx › renders deleted state non-clickable`
- R8：前端 `connectionStore.test.ts › refreshes session list on connected`、`sessionStore.test.ts › lifecycle keeps running_with_subagent`；后端 `session_lifecycle::list_sessions_includes_empty_child`
- R9：`manager::cancel_subtree_stops_children_then_parent`、`connection::connection_cancel_does_not_touch_unrelated_child_projection`、`subagent_auto_turn::user_stop_suppresses_auto_wake`
- R10：前端 `SessionStatusLine.test.tsx › child footer stop sends agent/cancel with child id`、`SubagentRosterPanel.test.tsx › delete disabled while running`
- R11：`subagent::hub::lagged_router_rescans_and_redelivers`
- 通用：validate_all ok；`cargo fmt --check`、clippy 干净；不改既有 wire 快照以外的字段（新增字段需更新快照并注明）。

### D3 五档分类
| # | 发现 | 档 |
|---|---|---|
| F1 | 删除操作无父工具/子 UI 入口，谁来删未定（A1/A5） | 1 |
| F2 | 用户能否直接给子发消息/steer（A2/A5/B4） | 1 |
| F3 | 子在跑时发消息：拒绝/排队/打断（B4） | 1 |
| F4 | 停父是否级联 / 是否提供停整棵树（B1） | 1 |
| F5 | 取消后 pending 队列保留并自动再起（A6③） | 1 |
| F6 | 停父后子结果自动唤醒父（B1） | 1 |
| F7 | 用户发起的子 turn 是否通知/唤醒父（B7） | 1 |
| F8 | 删父遇在跑后代：拒绝还是先停（B2①） | 1 |
| F9 | 删子后卡片显示语义（B3） | 1 |
| F10 | CLI 是否支持子 session（A4） | 1 |
| F11 | 停止按钮位置、子被停后父拿到什么 / 父能否续跑用户停的子（A5、C1） | 1 |
| F12 | 父看不到子运行进度、不能 steer（A1） | 2 |
| F13 | 父工具与用户直发语义不一致（B4） | 2 |
| F14 | 用户直发子：在跑可入队、空闲报 not a primary agent（§4.X） | 2 |
| F15 | 子所有权限 ask 被静默拒绝（B6） | 2 |
| F16 | completion 无来源，父误把用户 turn 当自己结果（B7） | 2 |
| F17 | 父正在 wait 某子时可删该子（B3） | 2 |
| F18 | 权限 sink 取决于 last_permission_sink 隐式状态（B6） | 3 |
| F19 | 连接级 Cancel 范围绑 projection，是否停子取决于 UI 是否打开过（B1） | 3 |
| F20 | Workers 名单删子后幽灵 idle 行且计数（B3） | 4 |
| F21 | 聚合态变化是否推 session/status 未核，父列表项可能停旧态（B1，暂定） | 4 |
| F22 | `agent/cancel` 恒回 cancelled:true + StartingTurn 取消丢失（A2/A6，合并） | 5 |
| F23 | subagent_stop 先查后取消非原子（A6） | 5 |
| F24 | 级联删除的子孙 hub/jobs 残留（B2②） | 5 |
| F25 | cleanup_deleted_files 错误被吞（B2③） | 5 |
| F26 | 删子不关面板、卡片回退 accepted 可点（B3） | 5 |
| F27 | list_sessions 跳过空子 session（B5） | 5 |
| F28 | 前端状态补回依赖 SessionList 挂载；lifecycle 无 running_with_subagent（B5） | 5 |
| F29 | hub router Lagged 丢 completion（B1） | 5 |
| F30 | 子在跑末尾入队的用户消息 pending flush 失败静默滞留（§4.X） | 5 |
计数：1 档 11、2 档 6、3 档 2、4 档 2、5 档 9，共 30。

### D4 需 Andy 定（去重）
| # | 决策 | 选项 | 推荐（依据） |
|---|---|---|---|
| 1 | 用户能否直接对子发消息 | a 只读（Codex V2）/ b 可发（Claude Code、Codex V1、Roo、OpenCode） | **b 可发**，用子自己的 profile；结果不唤醒父（见 5）。Claude Code 允许在子视图直接输入。 |
| 2 | 子的权限请求归谁 | a 一律拒绝并写进工具描述 / b 转父会话审批卡并标注子 | **b**（Claude Code v2.1.186 起即此做法，Esc 只拒这一次）；过渡期至少 a 并写明。 |
| 3 | 子在跑时收到消息（父与用户同一套） | 拒绝 / 排队 / 打断 / 参数化 | **参数化 `queue|interrupt`，默认 queue**，父和用户共用（Codex V1 `send_input.interrupt`；Claude Code 输入排队、Ctrl+Enter 尽快读到）。 |
| 4 | 停父是否级联停子 | a 不级联 / b 级联 / c 默认不级联 + "停整棵树" | **c**（Codex 停父不级联、close_agent 关后代；Claude Code 子独立 TaskStop）。 |
| 5 | 子结果回流：用户发起的子 turn、以及用户停父后的子结果，是否唤醒父 | 唤醒 / 只给不唤醒提示 / 不通知 | **只认父派发的结果唤醒父**；用户来源与停父后的结果给不唤醒提示（Codex #13850 理由：用户介入不应让父立刻重启子）。 |
| 6 | 被中断/用户停掉的子：算完成通知父？父能否自动续跑？ | 算完成可续 / 非终态不可自动续 | **非终态、父不自动续跑**（Codex interrupted 非终态；Claude Code 用户停的子父不能自动续）。 |
| 7 | 取消 turn 时 pending 队列 | 保留并自动再起 / 保留不自动起 / 清掉 | **保留但不自动起**，下次显式输入时一并发送（避免"停了又跑"；与 6 一致）。 |
| 8 | 删除：谁能删、删父是否级联 | 父工具 / 仅用户 / 仅保留期；删父遇在跑后代拒绝或先停 | **仅用户删 + 保留期清理**（Claude Code 无删除工具、30 天保留；Cursor UI 归档）；删父级联并提示数量（Roo），后代在跑时**先确认再停后删**（目前直接拒绝且提示不准）。 |
| 9 | 删子后父卡片 | 显示"已删除" / 隐藏 | **显示"已删除"不可点**（保留父 transcript 可读性）；父正在 wait 该子时拒绝删除。 |
| 10 | CLI 是否支持子 session | 不支持 / list 标 parent + resume 只读 / 完整 | **最小：list 标 parent、resume 子只读提示**；完整支持后置。 |

## 5. 回报格式（各域统一）
每题：结论（能/不能/部分）→ 证据 `文件:行` → 缺口 → 建议修法 → 档位 → 未核实项。另附「需 Andy 定」列表。不改代码、不开包、不 commit。

## 6. 时间线
1. GM 批计划后 start，四域并行只读调查并回报；
2. backend_mid 交叉核对 + 与前端中层对齐，合并为本文件报告（矩阵 + 生命周期 + 补全方案 + 分类 + 待定项）；
3. validate_all ok 后 deliver 给 GM。

## 7. 风险
- 外部资料可能过时，以官方文档/开源代码为准并注明日期。
- 前端结论依赖前端中层排期。
