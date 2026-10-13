# WP-085 · Runtime 回报（A6 / B1 / B4 / B7 / C2）

- 作者：runtime；基线 `ops/team-framework`，2026-10-13 HKT；只读，未改码、未 commit、未动 wp 状态。
- 格式：结论 → 证据 → 缺口 → 建议修法 → 档位（1 产品未定 / 2 产品缺陷 / 3 架构 / 4 常见 bug / 5 边角 bug）→ 未核实。

## A6 子 turn 能否从外部取消（auto-turn 调度）
**结论：部分能。** 子 session 与普通 session 共用 `SessionManager` 取消通道；任一持 sid 的入口都能取消运行中的子 turn，但 `StartingTurn` 预约窗口内的取消会静默丢失。
**证据**
- 唯一取消原语 `cancel_turn_sync` 只处理 `RunningTurn`：`src/session/manager.rs:1128-1139`；`cancel_turn` 透传：`:1122-1124`。
- 外部入口：RPC `AGENT_CANCEL` 对任意 sid（含子）调用 `cancel_turn`，且无条件返回 `{"cancelled":true}`：`src/client_protocol/connection.rs:384-401`；连接级 Cancel/Quit 对本连接所有 projection 逐个取消：`connection.rs:1636-1660`、`:1780-1805`；父工具 `subagent_stop`：`src/tools/subagent/stop.rs:47-54`；revert 先取消后代：`manager.rs:1475-1498`；进程退出：`manager.rs:1721-1725`。
- 子 turn 启动路径 reserve→spawn→start：`src/tools/subagent/turn.rs:13-54`（launch `launch.rs:91`、send `send.rs:86`）。预约态 `StartingTurn` 不带 cancel token：`manager.rs:850-888`；`start_turn` 才装入 token：`manager.rs:1028-1068`。
- auto-turn：父的 idle auto-turn 由子 TurnFinished 触发（`src/runtime/subagent_auto_turn.rs:192-199` 退出 handler、`:207-219` flush 线程），只对**父**预约+spawn（`:29-83`、`:85-167`），要求有 UI 订阅者（`:43`）；pending flush 对任意 session（含子）在 TurnFinished 后起 turn，无 UI 门：`src/runtime/pending_auto_turn.rs:1-13`、`:46-110`。auto-turn 起的 turn 与人工 turn 同一 `RunningTurn` 结构，因此同样可取消。
**缺口**
1. `StartingTurn` 窗口取消丢失：`cancel_turn_sync` 返回 false，RPC 仍报 cancelled:true，turn 随后照常开跑（含子 auto-turn / pending flush 的 reserve→start 间隙，跨线程 `subagent_auto_turn.rs:126-158`）。
2. `subagent_stop` 先 `get_cached_progress` 再 cancel，非原子；StartingTurn 时 fallthrough 报 “already ended/idle”：`stop.rs:47-75`。
3. 取消后 pending 队列会被 `pending_auto_turn` 立刻再起一个 turn（"停了又跑"）；子也一样。
**建议修法**：`StartingTurn` 增 `cancel_requested` 标志，`cancel_turn_sync` 置位；`start_turn` 装 token 时若已置位立即 cancel；返回值区分 running/starting/idle，RPC 如实回报。stop 工具改用一次锁内判定。是否"取消即清/保留 pending 队列"交 Andy。
**档位**：缺口 1/2 → (5) 边角 bug；缺口 3 → (1) 产品未定。
**未核实**：web 停止按钮对子 sid 实际发 `AGENT_CANCEL` 还是连接级 Cancel（前端域）；HTTP serve 是否另有 cancel 路由（Session 域）。

## B1（主答）停父 turn 是否级联停子；running_with_subagent 状态
**结论：不级联（设计如此）。** 停父只取消父自己的 token；子继续跑，父状态 Running→Stopping→Idle 后被聚合提升为 `running_with_subagent`。
**证据**
- 明确不传播：`launch.rs:129-132` 注释 “Parent turn cancellation is deliberately not propagated into a child turn; `subagent_stop` is the explicit session-level cancel path”；`parent_cancel` 仅为兼容保留（`launch.rs:318`）。
- 父 `AGENT_CANCEL` 只取消父 sid：`connection.rs:384-398`。例外：连接级 Cancel/Quit 遍历本连接全部 projection（`connection.rs:1645-1660`），子若在同连接被打开过则会被一起停——**结果取决于 UI 是否打开过子**。
- 唯一真级联：revert `cancel_descendant_turns`（`manager.rs:1475-1498`）；删除被活跃后代 fence（`manager.rs:1614`、`:1757`）。
- 父等待中：`subagent_wait` 受父 token 控制，返回 error “subagent_wait cancelled”，子不受影响：`src/tools/subagent/wait.rs:114-115`。
- `running_with_subagent` **不是存储态**，是读时聚合：`manager.rs:81-89`（枚举）、`:1342-1369`（own≠Idle 直接返回；own Idle 且任一后代非 Idle → RunningWithSubagent）；`StartingTurn` 计为 Running（`:137`），子 Stopping 也提升父（测试 `manager.rs:3845-3884`）。无 set/clear 点，转换由子 reserve/start_turn/finish_turn/release_reservation 驱动。递归到全部后代（`descendant_session_ids` `:1395`）。
- 父停后子完成：子 TurnFinished → completion 入父 inbox（`src/tools/subagent/hub.rs:94-108`）→ 若父有 UI 订阅则 idle auto-turn 唤醒父（`subagent_auto_turn.rs:40-48`）。即"停父"后父可能被子结果**自动再唤醒**。
**缺口**
1. 无"停整棵树"能力（用户无法一键停父+子）。
2. 连接级 Cancel 是否带走子取决于 projection 集合，语义不确定。
3. 状态推送：聚合态只在读时计算；子状态变化是否给父推 `session/status` 通知需 Session 核（protocol.rs:192）。若不推，父列表项会停留在旧状态——竞态。
4. 停父后子结果自动唤醒父 turn，用户可能感知为"停不下来"。
5. hub router `Lagged` 只 warn，丢 completion（`hub.rs:114-116`）→ 父 wait 已靠 data 兜底，但 idle auto-turn 唤醒会丢。
**建议修法**：保留"默认不级联"，新增显式 `cancel_subtree`（复用 `cancel_descendant_turns` 公开化 + 先停子后停父）供 UI"全部停止"；连接级 Cancel 改为只停用户指定 sid 或明确树；用户主动停父后为父设 "suppress auto-wake until next user input" 标志（待 Andy 定）；router Lagged 时按 data 重扫补投。
**档位**：不级联本身 (1)；缺口 2 → (3) 架构（取消范围绑 projection）；缺口 3 → 待 Session 核，暂 (4)；缺口 4 → (1)；缺口 5 → (5)。
**未核实**：聚合状态变化是否推通知（Session）；web 对 running_with_subagent 的停止按钮行为（前端）。

## B4（Runtime 侧）子运行时发消息：排队/拒绝/打断
**结论：两条入口语义不一致。** 父工具 `subagent_send` → **拒绝**；用户经 client_protocol 直接发 → **排队并在下一 step seam 注入（steer，不打断当前 step）**；均无"打断"。
**证据**
- 工具拒绝：`send.rs:63-67`（`is_turn_running_blocking`，含 StartingTurn，`manager.rs:1577-1582`）与 `:98-101`（reserve CAS 失败兜底），文案 “already running a turn. Its result will be delivered when that turn settles.”
- 用户发送：`connection.rs:496-500` `enqueue_user_input`，未检查 `is_child_session`（对比 retry 显式拒子 `connection.rs:333-344`）；idle 竞态时直接起 turn：`:501-512`。
- 注入：`src/runtime/exec.rs:135-156` 每个请求 seam 原子 claim 队列并作为 user 行追加；turn 结束仍有残留 → `pending_auto_turn.rs:46-110` 起新 turn（无 UI 门）。
**缺口**：父模型无法 steer 运行中的子（只能 stop 再 send）；用户 steer 会混进父派发的 turn，父拿到的结果含用户干预却无标注（见 B7）。
**建议修法**：给 `subagent_send` 加 `mode: queue|interrupt`（对齐 Codex V1 `interrupt`），queue 复用 `enqueue_user_input`，interrupt = cancel + 入队（依赖 A6 的 starting 修复）；入队消息带来源 (parent/user) 便于 B7 标注。
**档位**：(1) 产品未定（语义选择）；不一致本身 (2) 产品缺陷。
**未核实**：Tools 侧对文案/描述的期望；web 子视图是否允许输入（A5 显示只读，但协议层不拦）。

## B7（Runtime 侧）用户直接干预子后，父 wait/status 拿到什么
**结论：父拿到"子最近一个结束的 turn"的结果，不区分是否用户发起。**
**证据**
- wait 开始时快照每个子当前 `turn_id`（`wait.rs:73-76`），只等该 turn 的 TurnFinished（`:119-127`）；子空闲时显式 id 走 `latest_turn_result_blocking`（`:80-88`）——即用户干预后的最新 turn。
- 若用户的 steer 在父派发 turn 内注入（B4），该 turn 结果即含用户影响。
- 每个子 TurnFinished（**含用户直接发起的 turn**）都推 completion 到父 inbox 并可能唤醒父 auto-turn：`hub.rs:94-108`、`subagent_auto_turn.rs:192-199`；reminder 写入 `src/reminder/sources/subagent.rs`（未逐行核）。
- `subagent_stop` 对已结束子同样返回 latest result（`stop.rs:56-68`）；`subagent_list` 用聚合状态（`list.rs:43`、`:64`）。
**缺口**：父无法分辨"自己派的活"与"用户另起的活"；用户与子的私聊会自动唤醒父并被当作任务结果；wait 对用户 turn 运行中的子也会把用户 turn 当目标。
**建议修法**：TurnFinished/CompletionRef 增 `origin: parent_tool|user|auto` （spawn 时由 TurnOptions 标记）；父 inbox 默认仅收 parent_tool 来源，用户来源改为非唤醒的通知 reminder；wait 只匹配父派发的 turn_id。
**档位**：(1) 产品未定为主；"用户 turn 唤醒父"若 Andy 判不该发生则 (2)。
**未核实**：`reminder/sources/subagent.rs` 的渲染文案；Codex 对此的取舍见 C2。

## C2 Codex CLI 参考（2026-10 检索，以上游代码/PR 为准）
- V1 工具：`spawn_agent / send_input / wait_agent / close_agent / resume_agent`；`send_input.interrupt=true` 停当前任务立即处理，默认排队；`close_agent` 关闭目标**及其后代**。来源 https://github.com/openai/codex/blob/35aaa5d9/codex-rs/tools/src/agent_tool.rs 、PR #23475 https://github.com/openai/codex/pull/23475
- V2：`send_message`（排队不触发 turn）/`followup_task`（触发 turn）/`interrupt_agent`（只打断当前 turn，代理保留可复用）/`list_agents`；无 close/resume。来源 commit 8d41505 https://github.com/openai/codex/commit/8d415050fce4b4ebc6da1ba247379844235fa453 ；二手综述 https://cdn.jsdelivr.net/npm/pi-subagent-manager@0.14.0/docs/research-codex.md （未逐条对源码核）
- interrupted 为**非终态**：wait 不因中断返回、不通知父——理由是用户中断子时不希望父立刻让子重启。来源 #13850 https://github.com/openai/codex/commit/3f266bcd68c78ac043969f8a7a916c7ee30df112
- 用户直接介入：V2 子线程在 TUI 只读，app-server `turn/start`、`turn/steer` 对子拒绝（PR #33841 https://github.com/openai/codex/pull/33841 ）；V1 子可写。有 open issue 要求放开（#33885 https://github.com/openai/codex/issues/33885 ）。
- 停父不级联停子；父被中断后子迟到结果不可靠回流（用户反馈 issue #31178 https://github.com/openai/codex/issues/31178 ）。
- wait 带 timeout，返回各目标状态+timed_out：https://github.com/openai/codex/blob/53b50197/codex-rs/core/src/tools/handlers/multi_agents/wait.rs
**对照**：我们 ≈ Codex V1 + 默认不级联；差异：无 interrupt 模式、无关闭后代、用户可直接发子（协议不拦）、取消后子结果仍唤醒父（Codex 中断非终态不通知）。
**未核实**："停父不级联"来自 issue 描述与搜索摘要，未在 Codex 源码定位到具体 handler；V1 `close_agent` 关后代仅据工具描述。

## 需 Andy 定
1. 停父是否级联停子？（建议默认不级联 + 提供"停整棵树"）
2. 用户能否直接对子发消息/steer？（Codex V2 选只读）若允许，用户 turn 结果是否回流/唤醒父？
3. `subagent_send` 遇运行中子：拒绝 / 排队 / 打断 三选（或参数化）？
4. 取消 turn 时是否保留 pending 队列并自动再起 turn？
5. 用户停父后，子完成是否还自动唤醒父？
6. 被中断的子是否算"完成"通知父（Codex：不算）？
