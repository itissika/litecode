# WP-094-backend · 后端中层方案（WP-095）

- 角色：backend_mid；父 WP-094（gm，Andy 已批口径）；本包 WP-095（split，approver=gm）；前端兄弟包 WP-096（frontend_mid）。
- 基线：分支 `ops/team-framework`；中层只拆活/交叉审/汇总；不写 `src/`、`tests/`；不 commit；不碰 `.github/workflows/windows-*.yml`。
- 时间：2026-10-13 19:3x HKT。依据：GM 口径（Andy 批）+ WP-087 产物（WP-086-backend/runtime/session/tools.md、`.tmp/wp090_interface.md`）+ 代码核对。
- 本口径**替代** WP-087 K2（send 只入 pending）、K5（停通知不加来源字段）中与下表冲突的部分。

## 1. Goal
父发子消息有三种方式；父邮箱每个子只留最新结果；停止只告知不唤醒；停/删通知标明是用户还是父做的；补测试、更新知识库。

## 2. 裁定表（锁定口径）
| # | 项 | 裁定 |
|---|---|---|
| L1 | `subagent_send.mode` | `queue`（默认）：子在跑→消息作为**当前 turn 结束后的新 turn** 运行，**不在 step seam 注入**；子空闲→直接起新 turn。`steer`：保持今天行为（入 pending，step seam 注入当前 turn）。`stop_and_send`：停当前 turn→起新 turn；这次停**不发 stopped 通知、不唤醒父**。`subagent_stop` 保留。 |
| L2 | 主会话 | 人类主会话 steer 语义**不变**：pending 是共享的，主会话 step-seam 注入不许被改；用户对子会话仍只读（WP-091 K1 不变）。 |
| L3 | 最新结算 | 父邮箱每个子只留一条结果；去重键 (child, turn) → child；新结果覆盖未读旧结果；不计 turn 数；`subagent_wait` 返回最新。 |
| L4 | 停止不唤醒 | 被停 turn 的结果**只进邮箱**，不触发父 auto-wake；用户停与父停相同。删除不变。 |
| L5 | 来源 | stopped/deleted 通知带 `by: user|parent`（参照 bash `user_killed`）；提醒文案如 `stopped by user` / `deleted by parent`。父自己用工具停/删→**不给自己发通知**（实际只剩 by=user 进父 mailbox；by=parent 只用于状态字段，见 P2）。单一格式，不做新旧双格式兼容。协议字段先告知前端中层。 |
| L6 | 测试/KB | 各域补测试；subagent 相关 KB 节点改为新语义（enabled 节点可润色，**不改 status**）。 |

## 3. 代码定位（核对结果）
- 现 send 在跑走 `enqueue_user_input`（send.rs:63-66）→ 共享 `pending_messages` → `exec.rs:131-157 sync_request_seam` 的 `claim_pending_messages_for_turn` 注入；`agent/core.rs:166-174` 有 pending 时 turn 不结束。queue 模式必须**不进这条路**。
- 停止唤醒父：`hub.rs` Router 对任意 `TurnFinished` 都 `deliver`（push + exit_handler 唤醒），不区分 Cancelled。
- 去重：`jobs.rs CompletionInbox::push` 按 (child, turn) 去重、VecDeque 累积。
- 通知：`jobs.rs ParentNotice::{ChildStopped,ChildDeleted}{child_session_id}`；文案 `exec.rs:1066 parent_notice_reason`。写入点：用户停 `connection.rs:454`、用户删 `session_lifecycle.rs:370`、父停 `stop.rs:61`、父删 `delete.rs:64`。
- `SessionInfo.last_turn_reason`（protocol.rs:217）由 `session_lifecycle.rs:439-449` 从 `data().latest_turn_end_reason_blocking` 现取；运行中或主会话为空串。

## 4. 子包树
```
WP-094 (gm)
  ├── WP-095 (backend_mid, approver=gm, split)  ← 本包
  │     ├── WP-098 owner=runtime  harness-core      queue/steer/stop_and_send 原语 + 最新结算邮箱 + 被停不唤醒 + 通知 by + cancelled_by 落盘
  │     ├── WP-099 owner=session  harness-core      用户停/删写 by=user + SessionInfo/lifecycle 协议字段 + 子只读保持
  │     └── WP-100 owner=tools    tools-permission  send mode schema + 父停/删不给自己发通知 + wait 最新 + 描述 + KB
  └── WP-096 (frontend_mid)  侧栏用户停止标记
```
产物：WP-098→`WP-094-runtime.md`；WP-099→`WP-094-session.md`；WP-100→`WP-094-tools.md`。接口冻结：`.tmp/wp098_interface.md`（WP-098 goal 文字里写成 wp097 为笔误，以本文为准）。

## 5. 串并行与域口径
**顺序**：WP-098 先交接口（冻结 v1）→ WP-099 与 WP-100 并行 → 中层集成复跑 → deliver。

### WP-098 · Runtime（src/runtime、src/agent、src/types、hub.rs/jobs.rs、manager.rs pending/cancel 段）
- R-Q 队列：新增 per-session **follow-up 队列**（与共享 `pending_messages` 分开），`SessionManager::enqueue_follow_up(session, input)`；step seam **不认领**；turn 结束由 pending_auto_turn 同路径起**一个新 turn**（多条按序合并或逐条，接口中写明，建议逐条 FIFO）。`has_pending_user_messages`/core.rs:166-174 只看 pending，不看 follow-up。主会话不用此队列 → 主会话行为零改动。
- R-S steer：保持 `enqueue_user_input` 原路径。
- R-SS stop_and_send：`cancel_and_follow_up(session, input, by)` 单锁：标记取消 + 把消息放 follow-up，取消结束后起新 turn；此次取消**不写 ChildStopped、不唤醒**（结果照 L3 覆盖进邮箱）。
- R-L 最新结算：`CompletionInbox` 改为 `HashMap<parent, IndexMap<child, CompletionRef>>`，push 覆盖；`restore_front` 不覆盖更新的条目；R11 rescan 随之按 child 去重（ISSUE-054 自然缓解）。
- R-W 不唤醒：Router 对 end reason=cancelled 的 TurnFinished 只 push 不调 exit_handler；wait 若在等该子照常返回 cancelled。
- R-B 来源：`ParentNotice::{ChildStopped,ChildDeleted}{child_session_id, by: StopOrigin}`，`enum StopOrigin{User,Parent}`；`cancel_turn_sync(session, by)` 把 by 记到 turn 结束记录（`cancelled_by`，与 end reason 同行落盘）；提醒文案 `stopped by user|parent` / `deleted by user|parent`。
- 测试（done_when）：`runtime_queue_while_running_runs_new_turn_after_end`、`runtime_queue_not_drained_at_step_seam`、`runtime_steer_still_injects_at_step_seam`、`runtime_main_session_pending_still_injects_at_seam`、`runtime_stop_and_send_cancels_then_runs_new_turn_no_notice`、`hub_latest_only_one_result_per_child`、`hub_newer_result_overwrites_unread`、`hub_stopped_turn_result_mailbox_only_no_wake`、`hub_notice_carries_by_user_parent`、`settled_reminder_text_stopped_by_user`；fmt 干净；未 commit。

### WP-099 · Session（src/session（除 Runtime 段）、src/client_protocol、src/serve）
- 用户 `agent/cancel{child}` → `cancel_turn_sync(child, User)` + `ChildStopped{by:User}`（不唤醒）；用户删 → `ChildDeleted{by:User}`。
- 协议字段 P1-P3（见 §6）；wire 快照注明；用户对子仍只读。
- 测试：`user_cancel_child_notice_by_user`、`user_delete_child_notice_by_user`、`user_stop_does_not_wake_parent`、`child_still_readonly_for_user`，另补 `session_info_last_turn_cancelled_by_user_parent_null`、`last_turn_cancelled_by_survives_reload`、`turn_finished_event_carries_cancelled_by`；未 commit。

### WP-100 · Tools（src/tools/subagent、descriptions、registry、knowledge/）
- `subagent_send` 新参数 `mode: "queue"|"steer"|"stop_and_send"`，缺省 queue，未知值报错；结果文案标明 queued/started/steered。
- `subagent_stop` 调 `cancel_turn_sync(child, Parent)`，**不再写 ChildStopped**（不给自己发）；`subagent_delete` 同理不写 ChildDeleted。
- `subagent_wait` 返回最新结果（随 L3）。
- 描述：subagent_send/stop/delete/wait .md 更新；KB：`knowledge/contracts/subagent-lifecycle.md`、`knowledge/framework/session.md`、`knowledge/framework/featured-tools.md`、`knowledge/contracts/tools/tool-authorization.md`（仅涉 subagent 部分），不改 status，`kb_check` 绿。
- 测试：`send_mode_defaults_to_queue`、`send_queue_while_running_returns_queued`、`send_steer_injects_current_turn`、`send_stop_and_send_no_stopped_notice`、`send_mode_rejects_unknown`、`stop_by_parent_no_notice_to_self`、`delete_by_parent_no_notice_to_self`、`wait_returns_latest_only`；未 commit。

## 6. 协议（告知前端中层 WP-096，单一格式）
| # | 字段 | 形状 |
|---|---|---|
| P1 | `SessionInfo.last_turn_cancelled_by` | `"user" \| "parent" \| null`，紧邻 `last_turn_reason`；出现在 `session/list`（不进 snapshot，2026-10-13 更正）；随每次 turn 结束与 `last_turn_reason` 一并更新；`last_turn_reason != "cancelled"` 时为 null；**持久化**（turn 结束记录里与 end reason 同行），重连不丢。规则与 `last_turn_reason` 一致（主会话/运行中为 null）。 |
| P2 | `session/lifecycle {event:"turn_finished", ..., reason, cancelled_by}` | 沿用既有键 `reason`，新增 `cancelled_by`，取值同 P1（2026-10-13 更正）。进程关闭 shutdown_cleanup 取消记为 `user`。 |
| P3 | 无新事件 | 父 mailbox 通知 by 只进模型提醒，不上 wire。 |
采纳前端中层提议（代码核对：`last_turn_reason` 已从 turn 结束数据现取，加同行列即可持久化，无障碍）。

## 7. 不在本包
用户给子发消息（仍只读）；删除语义（不变，仅加 by）；turn 计数；新旧双格式兼容；R9 整树停；ISSUE-053/055。

## 8. 风险
- 主会话 steer 被误改：follow-up 队列与 pending 物理分开 + `runtime_main_session_pending_still_injects_at_seam` 守护。
- stop_and_send 竞态（Starting 窗口）：复用 WP-090 `cancel_requested`，单锁内完成。
- 覆盖丢信息：被覆盖的旧结果仍在子 transcript，可经 subagent_list transcript 路径读取。

## 9. 待 GM 裁（档位）
- Q1（档位 3，可中层定，默认采纳）：stop_and_send 的内部停 `last_turn_cancelled_by` = `"parent"`（被停 turn 状态如实），但不发通知。请确认。
- Q2（档位 3）：queue 模式多条排队时逐条各起一个 turn（FIFO）还是合并为一个 turn？建议逐条。
- Q3（档位 2，Andy 档）：父正在 `subagent_wait` 该子时用户停子——wait 返回 cancelled 结果（父在 wait 即醒，属"已在等"不算唤醒）。建议保留。
- Q4（档位 3）：已读后到来的新结果照常唤醒父（仅 cancelled 不唤醒）。默认如此。

## 10. 验收清单
- [ ] WP-098/099/100 done 或明确 blocked；测试名全绿，中层集成复跑
- [ ] §6 与 WP-096 交叉审一致；wire 快照已注明
- [ ] kb_check 绿；`validate_all.mjs` ok:true；未 commit

## 交付汇总（后端中层，2026-10-13 20:0x HKT）
- 子包 WP-098（Runtime）、WP-099（Session）、WP-100（Tools）已完成。
- Runtime 补了端到端用例 stop_and_send_cancels_then_runs_follow_up_without_waking_parent，覆盖以下几点：
  - cancel 返回 (Running, Queued{1})
  - 旧回合的 cancelled_by=parent，落盘与 live 一致
  - 新回合自动起来
  - 父只在新回合结束时被唤醒一次
  - 收件箱只有一条
- 补测时发现并修复了一个真 bug（产品缺陷），只改了 src/runtime/pending_auto_turn.rs：
  - 问题：try_begin_pending_flush 用主会话的 agent 白名单校验子会话，校验不过就静默放弃；它还用主会话身份起回合。结果是子会话的 follow-up 和残留 steer 都不会自动起回合。
  - 修法：子会话改用自己的 agent，并以 TurnOptions::child 起回合。主会话路径不变。
- 中层复跑结果：
  - cargo fmt --check 全仓库干净
  - wp098_follow_up_queue 7/7
  - pending_auto_turn 14/14
  - subagent_send 10/10
  - subagent_session_isolation 15/15
  - pending_messages 5/5
  - architecture_layers 5/5
  - lib tools::subagent 38/38
- 遗留问题：ISSUE-054（重启后重复投递，边角 bug，已部分缓解）、ISSUE-056（重启后丢失已告知父会话排队的 follow-up，产品缺陷）、ISSUE-055（预先存在的失败）。
- 本包未 commit，未动 windows-*.yml。
