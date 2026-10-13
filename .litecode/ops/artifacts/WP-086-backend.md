# WP-086-backend · 后端中层方案（WP-087）

- 角色：backend_mid；父 WP-086（gm，Andy 收敛口径）；本包 WP-087（split，approver=gm）；前端兄弟包 WP-088（frontend_mid）。
- 基线：分支 `ops/team-framework`；中层只拆活/交叉审/汇总；**不写 `src/`、`tests/`**；**不 commit**；不碰 `.github/workflows/windows-*.yml`。
- 时间：2026-10-13（HKT）。依据：`WP-084-research.md`（R1-R11、D2、file:line）+ Andy 收敛口径 + GM 补充（R2 origin 取消）+ 与前端中层已对齐的协议点。
- 本口径**替代** WP-084 D4 推荐。

## 1. Goal
子会话只有五样：看、发、停、删、状态变更提醒。用户：看/停/删（**无任何起子 turn 的路径**）；父：看/发/停/删。机制从简。

## 2. 裁定表（锁定口径）
| # | 项 | 裁定 |
|---|---|---|
| K1 | 用户给子发消息 | **不能**。`agent/run`、`session/pending-enqueue`（含在跑入队路径）对子会话显式拒绝，错误码 `subagent_session_readonly`；消除 "not a primary agent" 半通态（§4.X、F14、F30 随之消失） |
| K2 | 父给子发消息 | `subagent_send`：子空闲→起新 turn；子在跑→**入 pending**，下一个机会（step seam / turn 末 pending_auto_turn）投递；**无 interrupt 旋钮、无 mode 参数** |
| K3 | 停 | 用户 `agent/cancel{child}` 与父 `subagent_stop` 都只停当前 turn；如实回结果（R1）；**pending 不动**，照常有机会就发（不加任何停止相关队列逻辑） |
| K4 | 删 | 在跑不能删（保持）；删父级联删子，任一后代在跑→拒绝，错误码 `descendant_running`；父新增最小工具 `subagent_delete` |
| K5 | 提醒 | 子 turn 完成→照今天唤醒父（不变）。停/删通知（无论用户或父发起）→**只写父 mailbox、不唤醒**；用**通知类型**区分（`ChildStopped`/`ChildDeleted`），**不加 origin/initiator 字段** |
| K6 | 权限 | 不透传。子权限创建时定；运行中未授权直接拒（deny sink），写进 `subagent_launch` 描述；**去掉 `last_permission_sink` 对子会话的隐式依赖** |
| K7 | 不做 | R2 origin、R3（被 K1 取代）、R4 权限透传、R5 interrupt/mode、R9 整树停、revert 相关、CLI 子会话、R10（前端域） |
| K8 | 照修 | R1 cancel 如实、R6 删除清理、R7 墓碑（后端）、R8 重连状态（后端）、R11 Lagged 补投 |

## 3. 子包树
```
WP-086 (gm)
  ├── WP-087 (backend_mid, approver=gm, split)  ← 本包
  │     ├── WP-090 owner=runtime  harness-core      R1 原语 + 停/删通知类型(不唤醒) + R11 + 去 last_permission_sink
  │     ├── WP-091 owner=session  harness-core      K1 拒子 run/enqueue + R1 回包 + R6 + R7 + R8 + 用户停/删写通知
  │     └── WP-092 owner=tools    tools-permission  send 入 pending + stop 原子 + subagent_delete + 描述
  └── WP-088 (frontend_mid)  子会话 停/删 入口与展示（其下 WP-089 web）
```
产物：WP-090 → `WP-086-runtime.md`；WP-091 → `WP-086-session.md`；WP-092 → `WP-086-tools.md`。

## 4. 串并行与文件归属
| 包 | 独占文件 | 顺序 |
|---|---|---|
| WP-090 Runtime | `src/runtime/**`（`subagent_auto_turn.rs`、`pending_auto_turn.rs`、`custom_auto_turn.rs`、`exec.rs`）、`src/agent/**`、`src/types/**`、`src/session/manager.rs` 的 `cancel_turn_sync`/StartingTurn 段（:850-888、:1028-1068、:1122-1139）、`src/tools/subagent/hub.rs`（router/Lagged/通知类型） | **先行**：先交接口（`CancelOutcome`、`ParentNotice::{ChildStopped,ChildDeleted}`、`hub::notify_parent_no_wake`、`SessionManager::remove_session` 返回全部被删 id 的签名）写入 `.tmp/wp090_interface.md` 并告知中层，Session/Tools 才开工 |
| WP-091 Session | `src/client_protocol/**`、`src/serve/**`、`src/session/`（除上述 manager 段；含 `remove_session` 实现与 `read.rs` bindings）、`data/sqlite/session.rs` | 接口落地后与 WP-092 **并行** |
| WP-092 Tools | `src/tools/subagent/{send,stop,delete(新),list,wait,mod,launch}.rs`、`src/tools/descriptions/subagent_*.md`、`src/permission/**`、`src/tool/registry.rs` | 接口落地后与 WP-091 并行；删除复用 Session 的 controller/manager 删除入口（只调用不改） |

`hub.rs` 只归 Runtime；`manager.rs` 按段分归属，冲突时以 Runtime 为准、Session 请示中层。

## 5. 域口径与验收

### WP-090 · Runtime（R1 原语、K5 通知、R11、K6）
- R1：StartingTurn 加 `cancel_requested`，start_turn 装 token 时立即生效；`cancel_turn_sync` 返回 `CancelOutcome { Running, Starting, Idle }`（取消即 Running/Starting，Idle 表示无事可停）。
- K3：取消**不碰** pending 队列；pending_auto_turn 现有"有机会就发"保持（不新增停止相关逻辑）。
- K5：hub 新增通知类型 `ParentNotice::ChildStopped{child_session_id}`、`ChildDeleted{child_session_id}`，经 `notify_parent_no_wake` 写入父 mailbox，**不触发** subagent_auto_turn；TurnFinished 路由与唤醒**不变**；父正在 `subagent_wait` 该子时，stop→wait 返回 `cancelled`，delete→`removed`（现有）。
- R11：router `Lagged` 时按 data 重扫未投递 completion 并补投（去重）。
- K6：子会话 auto-turn（subagent/pending/custom）一律 `deny_permission_sink`，不读 `last_permission_sink`。
- done_when 测试：`manager::cancel_during_starting_turn_cancels_on_start`、`manager::cancel_turn_sync_reports_idle_when_nothing_running`、`manager::cancel_leaves_pending_queue_intact`、`subagent::hub::child_stopped_notice_writes_mailbox_without_wake`、`subagent::hub::child_deleted_notice_writes_mailbox_without_wake`、`subagent::hub::child_turn_finished_still_wakes_parent`、`subagent::hub::lagged_router_rescans_and_redelivers`、`pending_auto_turn::child_uses_deny_sink_regardless_of_last_sink`、`subagent_auto_turn::child_uses_deny_sink_regardless_of_last_sink`；`cargo test --lib runtime:: tools::subagent::hub manager::` 全绿；fmt/clippy 干净；未 commit。

### WP-091 · Session（K1、R1 回包、K4/R6、R7、R8、用户停/删通知）
- K1：`agent/run`、`session/pending-enqueue`、idle 竞态分支（connection.rs:496-512）对 `parent_session_id` 非空的会话拒绝：`ErrorCode::SubagentSessionReadonly`（wire `subagent_session_readonly`，JSON-RPC -32000，message `"subagent sessions are read-only: messages can only come from the parent agent"`）。`agent/retry` 现有拒绝改用同码。
- R1：`agent/cancel` 回 `{"cancelled": bool}`（形状不变，值如实：Idle→false；Running/Starting→true）；对子放行；成功取消子时调 `notify_parent_no_wake(ChildStopped)`。
- K4/R6：删在跑会话→现有 `agent_already_running`；删父任一后代在跑→新 `ErrorCode::DescendantRunning`（wire `descendant_running`，-32000，message `"cannot delete: a descendant subagent session is running"`，data `{ "running_session_ids": [..] }`）；`remove_session` 返回全部被删 id，逐个 `purge_parent`+`forget_child`；`cleanup_deleted_files` 错误 `warn!`；删子（非级联根之下）→ `notify_parent_no_wake(ChildDeleted)`。
- R7：lifecycle `deleted` 事件增加 `parent_session_id: string|null`；snapshot / buffer load 新增 `subagent_binding_states: map<call_id, { child_session_id, deleted: bool }>`，删子后条目保留 `deleted:true`；旧 `subagent_bindings` 条目同样保留不删（兼容）。墓碑由删除时落 `deleted_children` 记录（父 meta/表）提供，重载后可见。
- R8：lifecycle `turn_started`/`turn_finished` 增加 `status`（`SessionStatus` 原始值，含 `running_with_subagent`、`stopping`）；聚合态变化（子起/停致父态变）给父也推一次 `turn_finished`/`turn_started` 等价的 `status_changed` 事件（`event:"status_changed"`, `status`）；`list_sessions` 不再跳过有 parent 的空子会话。
- done_when 测试：`client_protocol_run::agent_run_on_child_rejected_readonly`、`client_protocol_run::pending_enqueue_on_child_rejected_readonly`、`client_protocol_run::enqueue_on_running_child_rejected_readonly`、`client_protocol_cancel::agent_cancel_reports_false_when_idle`、`client_protocol_cancel::agent_cancel_child_writes_parent_notice_no_wake`、`session_lifecycle::delete_running_child_rejected`、`session_lifecycle::delete_parent_with_running_child_returns_descendant_running`、`session_lifecycle::delete_parent_purges_all_descendant_inboxes`、`session_lifecycle::cleanup_failure_is_logged`、`read::subagent_bindings_include_deleted_tombstone`、`read::tombstone_survives_reload`、`client_protocol_lifecycle::deleted_event_carries_parent_session_id`、`client_protocol_lifecycle::turn_events_carry_raw_status`、`client_protocol_lifecycle::parent_status_changed_on_child_turn`、`session_lifecycle::list_sessions_includes_empty_child`；wire 快照更新并注明新增字段；`cargo test --lib client_protocol:: session::` 全绿；未 commit。

### WP-092 · Tools（K2、K3、K4、K6 描述）
- K2：`subagent_send` 子在跑→`enqueue_user_input`（入 pending，返回 `queued: true`）；子空闲→起 turn（现有）；删 "already running" 拒绝；无 mode 参数。
- K3：`subagent_stop` 改用 `CancelOutcome`，一次锁内判定（去先查后取消）；结果文案 `stopped`/`nothing running`；成功→`ChildStopped` 通知（不唤醒）。
- K4：新工具 `subagent_delete{session_id}`：只能删本父的直接子；在跑→拒绝；有在跑后代→拒绝（descendant_running 文案）；级联删其后代；成功→`ChildDeleted` 通知（不唤醒）；depth≥1 不可用（与 launch 一致）。
- K6：`subagent_launch.md` 写明"子权限在创建时确定，运行中需审批的操作直接被拒绝"；send/stop/delete 描述同步。
- done_when 测试：`subagent::send::enqueues_while_running`、`subagent::send::queued_message_delivered_next_opportunity`、`subagent::send::starts_turn_when_idle`、`subagent::stop::stop_is_atomic_against_starting_turn`、`subagent::stop::reports_nothing_running_when_idle`、`subagent::stop::does_not_clear_pending`、`subagent::delete::deletes_idle_child_and_notifies_without_wake`、`subagent::delete::rejects_running_child`、`subagent::delete::rejects_when_descendant_running`、`subagent::delete::rejects_non_child`、`registry::subagent_delete_hidden_at_depth_ge_1`、`descriptions::subagent_launch_mentions_runtime_permission_denial`；`cargo test --lib tools::subagent permission::` 全绿；未 commit。

## 6. 协议对齐（WP-087 ↔ WP-088，已与前端中层商定）
| # | 契约点 | 后端 | 前端 |
|---|---|---|---|
| P1 | 子会话发消息 | `agent/run`、`session/pending-enqueue` 对子 → error `code:"subagent_session_readonly"`（-32000），message 见 §5 | 子不渲染输入框（现状）；收到该码显示只读提示 |
| P2 | 停子 | `agent/cancel{session_id:child}` 放行；回包仍 `{"cancelled": bool}`，**形状不变**，无事可停→`false` | 子底栏 + Workers 行停止；`false` 提示"没有在运行的回合" |
| P3 | 停/删后父 | 只写父 mailbox（`ChildStopped`/`ChildDeleted`），不唤醒；父在 wait 该子→`cancelled`/`removed`；pending 不动 | 无新事件需处理；父态靠 P6 |
| P4 | 删 | 在跑→`agent_already_running`；删父后代在跑→`descendant_running`，`data.running_session_ids` | 在跑置灰；按码给不同提示 |
| P5 | deleted 事件 | `session/lifecycle {event:"deleted", session_id, parent_session_id}`；级联时每个子各一条 | 关 `subagent-<id>` 面板、记已删集合、名单/计数排除 |
| P6 | 墓碑 | snapshot/buffer load 新增 `subagent_binding_states: {call_id: {child_session_id, deleted}}`；`subagent_bindings` 条目保留 | 卡片 deleted→"已删除"不可点；重载后仍正确 |
| P7 | 原始状态 | `turn_started`/`turn_finished` 带 `status`（含 `running_with_subagent`）；父聚合态变 → `event:"status_changed", status` | 用 `status` 而非一律 idle |
| P8 | 重连 | `session/list` 含空子会话、带原始 status | `connected` 时统一调 `session/list` |
| P9 | 不变 | `agent/subagent_bound` 仍为 live 事件，重连靠 P6 补齐；无 origin 字段；无 HTTP 新路由 | 同 |
新增字段均为加法，旧前端忽略即可；新前端对旧后端需容忍字段缺失。

## 7. 不在本包
- R2 origin / initiator 字段（GM 补充：用户无起子 turn 路径，取消）；R3（K1 取代）；R4 权限透传；R5 interrupt/mode；R9 停整棵树、连接级 Cancel 范围（F19，保持现状，另记观察）；revert；CLI 子会话；R10 UI（WP-088）；父查看子增量进度（F12）。
- 档位更新：F7/F16（origin 相关）作废；F13/F14/F30 由 K1 消除；F2/F3/F8/F9 由口径定案；F15/F18 由 K6 定为"拒绝并写明"。

## 8. 验收清单（WP-087 deliver 前）
- [ ] WP-090/090/091 done（或明确 blocked）；§5 测试名全绿，中层抽样复跑
- [ ] §6 与 WP-088 产物交叉审一致
- [ ] wire 快照变更已注明；`cargo fmt --check`、clippy 干净
- [ ] `validate_all.mjs` ok:true；未 commit

## 9. 请 GM
子包 WP-090/090/091 均 drafting。请 `approve --id WP-087`；批后中层 start，Runtime 先交接口，再批 Session/Tools plan。

## 9. 交付汇总（2026-10-13 14:5x，后端中层）
- WP-090 Runtime done：CancelOutcome 单锁、Starting 窗口取消、ParentNotice{ChildStopped,ChildDeleted}+notify_parent_no_wake、R11 Lagged 补投、子会话 deny sink（主会话保留 last_permission_sink）、TurnStatusHook。补丁：auto_turn 测试改用 #[cfg(test)] test_child_session。
- WP-091 Session done：子会话起 turn 入口（run/plan_execution/retry/pending-enqueue）回 subagent_session_readonly；cancel 如实；delete_session_tree 公共删除（单锁、AlreadyRunning/DescendantRunning{ids}），controller 与 subagent_delete 共用；墓碑列 deleted_children_json，snapshot 与 buffer/load 带 subagent_binding_states；R8 status_changed 经 hook。
- WP-092 Tools done：send 在跑入 pending(queued:true)；stop 用 CancelOutcome；新工具 subagent_delete（直属子、depth≥1 隐藏）；subagent_list 带 transcript 路径与行数（GM 加项）。授权单行：config/schema.rs、config/global_db/tools.rs。
- 中层集成复跑（独立 target）：architecture_layers 5/5、session_delete 4/4、subagent_session_isolation 15/15、wp091 11/11、tools::subagent 26/26、pending_auto_turn 10/10、tool::registry 7/8（唯一失败 ISSUE-055 预存）、cargo fmt --check 干净。clippy 未跑（本机未装）。
- ISSUE：053（过时集成测试，已修）、054（重启后可能重复投递旧结果，边角）、055（registry 断言未随检索门控更新，常见 bug，预存）。
- 未 commit。
