# WP-085-session · Session 域回报（A2/A3/A4/B1/B2/B3/B5）

- 角色：Session；只读，未改代码，未 commit。时间：2026-10-13（HKT）。格式按 WP-084-research.md §5。

## A2 client_protocol RPC/通知
- 结论：**部分**。
  - 查消息：能。`session/subscribe`（connection.rs:643）、`session/snapshot`（:834）、buffer load（:1305-1330，返回 `subagent_bindings`）对子 session 不区分，都能用。
  - 发消息：能（未拦截）。`agent/run`（:181）和 `session/pending/enqueue`（:418）都不检查 `is_child_session`。只有 `agent/retry` 会显式拒绝（:334-340，"cannot retry a subagent session"）。`agent/set_primary` 对子返回错误（session_lifecycle.rs:512）。
  - 停止 turn：能。`agent/cancel`（:385-404）调 `cancel_turn`（manager.rs:1122-1138），只停这一个 sid，并且总是回 `{"cancelled":true}`，即使当时没有在跑的 turn。
  - 删除：能。`session/delete`（connection.rs:738）调 `delete_session`（session_lifecycle.rs:357），不区分子。
  - 通知：`agent/subagent_bound`（protocol.rs:25、project.rs:471-479）；会话信息带 `parent_session_id`/`parent_call_id`（protocol.rs:203-208）。
- 缺口：父 agent 通过工具（subagent_send/wait）驱动子，用户也可以直接 `agent/run` 子，两个入口之间没有协调；`agent/cancel` 回包不能反映到底停没停。
- 建议修法：先由 Andy 定子 session 是否允许用户直接发消息（见待定）。之后要么在 `agent/run`/`enqueue` 加子 session 策略，要么明确放行；`agent/cancel` 回包改为 `cancelled: cancel_turn_sync(..)` 的真实结果。
- 档位：直接发消息 = 1（产品未定）；cancel 回包不准 = 5。
- 未核实：用户直接对子 `agent/run` 后，父的 wait/status 和 reminder 会怎样（B4/B7，归 Runtime/Tools）。

## A3 serve HTTP 路由
- 结论：**不能（没有 HTTP 入口）**。router.rs:69-75 只有 `/health`、`/ws`、`/api/workspace`、`/api/settings`、`/api/media`；settings.rs:87-127 全是配置类。四类操作都只能走 `/ws` 上的 JSON-RPC（A2），HTTP 层不区分子 session。
- 缺口：无（这是设计如此：会话操作统一走 WS）。
- 建议修法：不加 HTTP 路由。
- 档位：不适用。
- 未核实：`workspace_router`（crate::workspace）里每条路由我没有逐条列出，按 rg 结果，其中没有 session/subagent 相关路由。

## A4 CLI
- 结论：**基本不能**。子命令只有 `list-sessions`、`resume <id>`、`serve`（cli.rs:46-58、:696）。src/cli 里没有任何 parent/child/subagent 逻辑（rg 无命中）。理论上可以 `resume <子id>` 打开子 session 并对它对话，但没有列出子、停止子、删除子的入口，也没有父子关系展示。
- 缺口：CLI 看不出父子关系；`resume` 子 session 后的行为没有定义。
- 建议修法：取决于 Andy 是否要求 CLI 支持子 session。最小做法是 `list-sessions` 标出 parent，`resume` 子时进入只读或给出提示。
- 档位：1。
- 未核实：`resume <子id>` 实际运行时的表现，我没有跑。

## B1 停父是否级联停子（Session 侧证据，Runtime 主答）
- 结论（Session 侧）：**session 层不级联**。`agent/cancel` 只调 `cancel_turn(sid)`（connection.rs:396-399），`cancel_turn_sync` 只取消这一个 record 的 token（manager.rs:1128-1137），不会遍历子。只有 `remove_session` 会遍历子并取消（manager.rs:1882-1898）。子 turn 能否从父的 cancel token 派生而被停掉，由 Runtime 回答。
- 缺口：如果 Runtime 也没有传播，停父后子会继续跑，父的状态停在 `running_with_subagent`（protocol.rs:192）。
- 建议修法：如果 Andy 定为"停父即停子"，在 `cancel_turn` 里复用 `collect_child_ids_blocking` 来级联（参照 manager.rs:1884-1898）。
- 档位：1（待定）；Runtime 确认不传播并且定为级联之后，属于 2。
- 未核实：Runtime 的 token 派生链。

## B2 删父
- 结论：**在跑的子不会被先停掉，而是整个删除被拒绝**。
  - 门控：`try_begin_operation(Delete)` 遇到任何非 idle 的后代就返回 `AgentAlreadyRunning`（manager.rs:1614-1616，测试在 :3816-3840）。`delete_session` 先拿这个 lease（session_lifecycle.rs:366-369）。所以 `remove_session` 里取消子 turn 的代码（manager.rs:1892-1898）在这条路径上实际不会碰到在跑的子，只是防御。
  - 磁盘：SQLite 级联删除，先删子（manager.rs:1901-1907，子的查询在 data/sqlite/session.rs:1534）。同时删 `session_operations`、blob refs（session.rs:~1565-1569），`cleanup_deleted_files` 删 `sessions/<id>` 目录和工作区快照（session.rs:1574-1586）。每个被删的子都会发 `SessionRemoved`（manager.rs:1909-1917）。
  - 内存：`purge_parent` 只清 CompletionInbox 里父的队列（hub.rs:53、jobs.rs:64-69）；`forget_child(id)` 只清以这个 id 为 child 的条目（jobs.rs:71-77）。删父时只对父 id 调了 `forget_child`，**被级联删掉的子孙 id 没有调用 forget_child/purge_parent**（session_lifecycle.rs:376-377）。
  - 用户侧：父删除失败时返回 -32000 "agent already running"，用户不知道是子在跑。
- 缺口：①子在跑时父删不掉，提示信息不准；②子孙在 hub/jobs 里的残留不会清理（孙子作为父的队列会留下来）；③`cleanup_deleted_files` 的错误被 `let _` 吞掉。
- 建议修法：①由 Andy 定"先停子再删"还是"拒绝"；如果保持拒绝，就返回专门的 StructuredError（如 `descendant_running`）。②`delete_session` 对 `remove_session` 返回的所有被删 id 逐个调用 purge_parent + forget_child（需要 remove_session 返回这个列表）。③失败时至少 warn。
- 档位：①1；②5；③5。
- 未核实：除 `sessions/<id>` 和快照以外，子是否还有别的磁盘产物（如 media 或 bash 输出目录）。

## B3 删子（forget_child）后的父绑定
- 结论（后端侧）：**绑定会消失，父 transcript 里的 subagent_launch 行变成悬空调用**。
  - `subagent_bindings` 和 `child_session_id` 都是按 sessions 表实时查出来的（read.rs:338、:350；manager.rs:2010-2020；controller/mod.rs:107-111）。子的行被删以后，buffer load 的 `subagent_bindings` 里就没有这个 call_id，重放时 function_call 行也不带 `child_session_id`（mod.rs:165-166、:483-484、:737-746）。
  - 父的事件不会被改写，launch 和 output 都还在。
  - 删子不检查父的状态：父如果正在 `subagent_wait` 这个子，子的行会被删掉。子自身在跑时，lease 已经拦住了删除。
  - 没有发给父 session 的"绑定失效"通知，只有 session_list 刷新。
- 缺口：前端无法区分"还没绑定"和"子已被删"；父如果正在等待这个子，结果未定义。
- 建议修法：保留墓碑，例如 bindings 返回 `{call_id: {child_id, deleted:true}}`，或者在 SessionRemoved 时给父推 `agent/subagent_unbound`；删子时如果父在跑，就拒绝或者先通知父。
- 档位：显示语义 1；父在等时删子 2（待 Runtime 核实）。
- 未核实：父 `subagent_wait` 遇到子被删时会怎样（Tools/Runtime）。

## B5 重连/重放完整性
- 结论（后端侧）：**部分**。
  - bindings：buffer load 每次都从 DB 重新生成 `subagent_bindings`（connection.rs:1320-1326）；重放时 function_call 行带 `child_session_id`。只要子的行存在，这部分是完整的。
  - SubagentBound：这是一个 live 事件（project.rs:471-479；mod.rs:410-422 会重新标记父的 function_call 行），**不落盘，也不重放**。重连后要靠 bindings 和行上的 child_session_id 补齐，这两者语义上等价。
  - 子的状态：session_list 带 `status`（含 running_with_subagent）、`parent_session_id`、turn progress（session_lifecycle.rs:410-461）；子的事件订阅 `session/subscribe` 子 id 后就能续上。
  - 缺口：child 行是 launch 时创建的，SubagentBound 在这之前就发出了；如果 SubagentBound 发出时客户端正好断开，又在子还没产生第一条事件之前重连，`list_sessions` 会跳过空 session（session_lifecycle.rs:~412 `is_session_empty` → continue），前端短时间内看不到这个子（bindings 有，列表里没有）。
- 建议修法：`list_sessions` 对有 parent 的子 session 不要按空 session 过滤。
- 档位：5。
- 未核实：前端怎样合并 bindings 和 list（前端中层）；`permission_request` 不落盘，重连后无法重建（已知现状，见 WP-081）。

## 需 Andy 定
1. 用户能否直接对子 session 发消息（`agent/run`/enqueue），还是子只读、只能由父驱动？（A2/B4）
2. 停父 turn 是否级联停子？（B1）
3. 删父时子在跑：拒绝（现状），还是先停子再删？（B2）
4. 删子以后，父 transcript 里的 launch 卡片怎么显示（墓碑/"已删除"）？父正在等这个子时是否允许删除？（B3）
5. CLI 是否需要支持子 session（列出/查看/停止/删除）？（A4）
