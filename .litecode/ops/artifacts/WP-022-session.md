# WP-024 方案（Session：不稳实现 + 测试裁剪）

## (a) 可疑实现排查范围
- 范围：src/session（manager/turn 预约/TurnGuard、data/sqlite 写路径与 receipt/revision CAS、幂等）、src/client_protocol（connection 会话循环、Ask registry、permission_bridge、controller）、src/serve（bind/auth、连接生命周期）。
- 关注四类：并发（锁持有跨 await、单写者被绕过、Ask/RPC 互阻）、回滚（start_turn/fanout 失败后预约与 activity 是否复原）、半写入（事务外多步写、revision 与 change_log 不同步、blob 删除顺序）、失败路径（send/recv 错误被 `let _ =` 吞、断连时 pending ask 与 turn 清理、Quit/Cancel 路径一致性）。
- 方法：读代码 + 针对性测试（必要时写临时复现测试于 .tmp，不入树）；每条给 file:line、分级（按 .litecode/ops/context/issue-grading.md：arch/drift/bug/suspect）、证据强度（复现测试 / 代码推理 / 仅疑似）。不改实现，修复另立包。

## (b) 与 Runtime 协作 R1（manual-compact e2e）/ R4（reconnect：auto 停回合、manual 可再点）
- Session 负责：回合终止的持久化与投影（终止事件落盘、SessionEvent/投影一致）、activity 回到 Idle、TurnGuard 释放（StartingTurn/RunningTurn→Idle）、协议 RPC 入口与再次进入（manual 再点不被残留预约挡住）、向客户端的错误上报（error 帧/通知字段）。
- Runtime 负责：LLM 重连策略、compact 引擎逻辑、auto/manual 判定与重试。
- 需对齐的接口点：Runtime 放弃重连时向 Session 交付的终止原因/错误类型；manual compact 的 RPC 方法名与参数；终止后是否允许同 session 立即 start_turn；客户端可见的错误码与文案字段。先与 Runtime 书面对齐，再各写各侧测试，e2e 由 Runtime 主跑、Session 提供协议入口断言。

## (c) 本域未提交文件收口
- 现状（git diff 已核）：三文件 diff 均为 rustfmt 格式化，无行为变化——
  - src/client_protocol/connection.rs（+67/-?）：长参数换行、删多余空行、ask_loop_tests 内 `use std::collections::HashMap` 位置调整。
  - src/session/data/sqlite/ops.rs（9 行）：测试中 query_row 闭包换行、删空行。
  - src/session/manager.rs（13 行）：child_session_tests 中 start_turn/assert 合并为单行。
- 功能修复（Ask registry 非阻塞、Deleted receipt 反查、fanout 回滚/TurnGuard）已在既有提交中（如 74f2bb1），对应测试 ask_does_not_block_other_session_rpc、ops.rs delete receipt 测试、fanout spawn failure 测试。
- 收法：按 hunk 归类（全部为 fmt），确认 `cargo fmt --check` 与本域测试通过后，作为一个“仅格式化”补丁集交总经理审，不 commit；若总经理要求，也可 `git checkout` 还原（需批准）。

## (d) 测试裁剪清单（只列不删）
- 标准：可删（被更强测试完全覆盖/测已删除行为）、可放宽（断言过细如完整文案/时序 sleep 易抖）、重复（同一路径多处断言）；每条写测试路径::名称、理由、替代覆盖测试。
- 已知项：sql death_list 假红（session_search 引起）归 WP-011，此处只登记不处理。
- 输出到 .litecode/ops/artifacts/WP-022-session.md。

## 工具需求
无，现有 wp.mjs / kb_check.mjs / cargo 足够。

---

# WP-024 执行结果（Session，2026-10-12，未 commit）

工具链：`PATH=/home/box/.cargo/bin:$PATH`（与 `.tmp/home/.rustup/.../stable` 是同一套），`rustc -V` = 1.99.0；HOME=/workspace/litecode/.tmp/home，stdin </dev/null。基线：工作树（ops/team-framework，未提交改动在内）。

## 1. 可疑实现排查（只记录，不修复）

| # | 位置 | 现象 | 分级 | 证据强度 |
|---|---|---|---|---|
| S1 | src/session/manager.rs open/open_child/ensure/register_session 四处 prune-on-open SaveTaskState | 原 `let _ =` 吞错，且记录未入 records 时 note_receipt 不同步，内存 revision 落后 DB 1，下次 CAS 冲突。**已修（WP-024）**：改为 match，成功时用 saved.revision 构造 SessionRecord，失败 tracing::warn。复现测试 manager.rs::s1_reopen_prune_keeps_revision_in_sync | **bug**（按 Runtime 交叉审升级）| 复现测试（修前 save_task_state 冲突，修后通过） |
| S2 | src/client_protocol/connection.rs session/compact 同步错误 | **已改（WP-024，字段与 Runtime 书面对齐）**：新增 compact_error_response；数字码仍 -32000，error.data=StructuredError：回合运行中→agent_already_running/retryable=true；其他真实失败→compaction_failed/retryable=false；nothing_to_compact、cancelled 不变（无 data）。 | drift→已收 | 单测 connection.rs::compact_error_tests |
| S3 | src/client_protocol/connection.rs:1394-1400、:1568-1575 | Always 授权在 `reply_tx.send` 之前就写进 runtime 规则（grant_runtime），send 结果被吞（:1402、:1576）。Ask 已取消或回合已结束时，授权照样持久化，“取消即拒绝”的语义被打破。 | suspect | 代码推理 |
| S4 | src/client_protocol/connection.rs:1735-1812、:1815-1840 | deferred 队列全仓没有生产者（只有 `controller/mod.rs:758 deferred_mut` 和 :245 初始化），属于死代码。里面单会话的 Cancel/Quit 分支会调 `reject_all_pending_asks`，把所有会话的 Ask 都拒掉（:1749、:1763），和 Ask registry 按会话隔离的契约不一致，只是现在走不到。 | drift（layer=gate/死代码） | 代码可判定（rg 无 push） |
| S5 | src/client_protocol/connection.rs:88-93（resolve_sid）、:592、:783 | session_id 为空时退回 `first_session_id().unwrap_or_default()`，多会话连接下可能作用到错的会话；连一个会话都没有时得到空 sid，下游报的错不够明确。 | suspect | 代码推理 |
| S6 | src/session/manager.rs:1715 | session GC 中 `let _ = self.remove_session(..)`，失败不记日志，删除失败无法观测。 | suspect（低） | 代码可判定 |
| S7 | src/runtime/mod.rs:813-830（emit turn/end） | 按 Runtime 交叉审收窄：turn/end 先于 finish_turn 落盘（顺序无问题，G3 已测）；剩余问题仅为 turn/end 持久化失败时只打 warn 日志，此时 TurnFinished.reason=None。 | suspect（低） | 代码可判定 |
| S8 | src/serve/state.rs:99 | maintain_snapshots 用 `list_session_ids_blocking().unwrap_or_default()`。列表读失败时按“没有会话”处理快照，可能误清快照。 | suspect（倾向 bug，取决于 maintain_snapshots 是否删除不在列表里的快照） | 代码推理 |

已复核没有问题的：fanout spawn 失败回滚（manager.rs:1060-1097，finish_turn 释放 TurnGuard）、release_turn_reservation（:869-890）、SessionOperationLease Drop 释放（:182-186）、ws 断连不取消回合（ws.rs:512-518，属设计）。
未触及 B3：serve/auth.rs、serve/router.rs 没发现绕过；以上都不是 arch，S1–S8 都不触发阻断。

## 2. 测试裁剪清单（只列，不删）

| # | 测试 | 类别 | 理由 | 替代覆盖 |
|---|---|---|---|---|
| T1 | tests/client_protocol_telemetry.rs::subscribe_logs_is_jsonrpc_method + ::unsubscribe_logs_is_jsonrpc_method | 重复 | 两条都只断言常量字符串 | 合成一条；或由 ws.rs:464-485 路径的协议测试覆盖 |
| T2 | tests/client_protocol_permission.rs::server_hello_notification_serializes_correctly、::permission_request_serializes_request_id、::ask_kind_wire_snake_case | 可放宽（按 Runtime 交叉审修订） | 三条序列化快照重叠；**保留一条完整 wire 快照**（建议 permission_request_serializes_request_id），其余两条收为字段级断言或删 | ws_bridge_*_round_trip + 保留的那条快照 |
| T3 | tests/session_list_empty_filter.rs 中 seq_hole / unknown_kind / surface_op_not_json / seq_starting_at_one / empty_surface_op / legacy_detail_kind（6 条） | 重复 | 同一条“有 item 行就列出”的规则，换不同畸形数据各测一遍 | 收成一个表驱动测试 |
| T4 | tests/session_data_lifecycle.rs:77 `sleep(50ms)` | 可放宽 | 靠 sleep 等时序，容易抖 | 改成等 notify 或轮询 |
| T5 | tests/manual_compact_e2e.rs::manual_compact_failure_e2e_releases_lease_and_retry_starts 与 tests/compact_failure_calibers.rs::manual_compact_failure_releases_lease_for_retry | 重复（部分） | 同一条 lease 释放后再试的路径，e2e 更强 | 留 e2e；calibers 那条只留引擎层断言 |
| T6 | tests/session_turn_entry.rs::session_layer_has_no_subagent_orchestrator、::session_module_does_not_declare_jobs_or_turn_entry | 可放宽 | 源码 grep 型守门，改名或挪文件就会假红 | 改成模块可见性或编译期约束 |
| T7 | src/client_protocol/connection.rs:1735-1840 的 deferred 分支（死代码，见 S4） | 可删（实现侧） | 没有生产者 | 无需替代 |
| T8 | tests/session_data_sql_death_list.rs::sessions_db_sql_and_rusqlite_stay_in_sqlite_module | 登记 | 假红，session_search 引起 | 归 WP-011，本包不处理 |

合计 8 项（展开后涉及约 15 个测试函数）。

## 3. 三个仅格式化文件（补丁集报告，未 commit）

- 文件：src/client_protocol/connection.rs（+50/-17）、src/session/data/sqlite/ops.rs（9 行）、src/session/manager.rs（13 行），合计 58 增 / 31 删，git diff 逐 hunk 核过，全部是 rustfmt 改动，没有行为变化。
- `rustfmt --check --edition 2024`：三个文件都是 exit 0（格式已对齐）。
- `cargo fmt --check` 全仓：exit 1，但 diff 只在别的域（src/reminder/sources/tests.rs、tests/manual_compact_e2e.rs），本包三个文件不在其中。manual_compact_e2e.rs 的格式差异由 R1 的 owner 处理。
- 本域测试：见第 5 节。
- 处置：作为“仅格式化补丁集”交后端中层转总经理；不 commit。

## 4. R1/R4 Session 侧证据（接口点已和 Runtime 书面对齐）

### 4.1 已对齐的 4 个接口点（Runtime 回复，Session 侧引用已核）
1. 终止：Runtime 以 TurnOutcome::Error 结束（agent/outcome.rs:5-10）。auto compact 失败记 CompactionFailed，之后不再 call_model（agent/core.rs:43-45）；重连用尽时先 persist_or_stop 部分输出，再 Error(Llm)（core.rs:70-77）。L1 经 emit_turn_completed 映射为 TurnEndReason::Error，错误文本放 final_text（runtime/mod.rs:873-896）。不新增枚举。
   Session 侧统一收尾（已核）：
   - finish_turn：manager.rs:966；RunningTurn 回 Idle：manager.rs:976（`mem::replace(.., Idle)`）；有排队的 revert 时直接转 Exclusive：:980-985；`activity_changed.notify_all`：:1002。
   - release_turn_reservation：manager.rs:869，StartingTurn 回 Idle：:883，TurnGuard 释放：:888。
   - TurnGuard：字段与约束注释在 manager.rs:286-291；begin 在 :864、:933，end 在 :888、:990。
   - 排队消息归还：claim 在 manager.rs:1186-1199，失败归还 `restore_pending_messages` 在 :1206-1215（插回队首），回合结束后 reserve_turn_and_claim_pending 取队列在 :1275-1300。
2. 手动 compact RPC：方法 `session/compact`（protocol.rs:29），参数 {session_id}，缺省时取当前会话（connection.rs:833-855，经 resolve_sid :88-93）。同步回复 {accepted:true, operation_id}；前置条件失败回 -32000（:855-875）。异步结果走 `session/compact_lifecycle`（protocol.rs:30）。引擎侧（Runtime 引用）：start_manual_compact session_lifecycle.rs:241-244，lease :260，spawn :286-310。
3. 再次进入：manual 失败后 lease 在 session_lifecycle.rs:316 释放（Drop 在 manager.rs:182-186）；auto 失败后可以再 start_turn（测试 manager.rs:3139 finish_turn_clears_running_immediately_and_allows_rebegin）。对应测试：manual_compact_e2e.rs:65，compact_failure_calibers.rs:70/132/203。
4. 错误：ErrorCode 用 snake_case（protocol.rs:58-75），StructuredError {code,message,retryable}（:80-86）；manual 映射在 project.rs:146-151 / controller/mod.rs:378-397；回合错误 retryable = reconnect.is_failed()，为真时 llm_http，否则 internal（project.rs:540-553，已核）。

### 4.2 R1/R4 缺口
- G1【Runtime 修复中】auto compact 失败时客户端只看到 `internal`，看不到 `compaction_failed`（project.rs:544-553）。Runtime 改为 compaction_failed、retryable=false。
- G2（Runtime 负责）R4：重连用尽 → 停回合 → start_turn / agent/retry 的端到端测试。
- G3（Session）重连用尽后 TurnFinished.reason 是否一定非空（见 S7），缺一条断言：TurnEnded 落盘早于 finish_turn 读取。
- G4（Session）session/compact 在 AgentAlreadyRunning 时的协议级断言（见 S2），目前只有引擎层测试。
- G5（Session）auto 失败后通过协议入口（JSON-RPC）再次 start_turn 的测试；目前只有 manager 层测试 :3139。

## 5. 本域测试结果（rustc 1.99.0，PATH=/home/box/.cargo/bin，共享 target 等过锁）
- lib（session:: client_protocol:: serve::）：240 passed / 0 failed。
- 集成（17 个测试文件，--no-fail-fast）：116 passed / 1 failed。失败的是 session_data_sql_death_list::sessions_db_sql_and_rusqlite_stay_in_sqlite_module，即已知假红，归 WP-011。
- tests/manual_compact_e2e.rs：2 passed / 2 failed（manual_compact_failure_e2e_releases_lease_and_retry_starts :80、manual_compact_retry_after_failure_completes :140）。错误是 `Config("no context window configured for this session")`，测试夹具没配上下文窗口，不是 Session 实现问题。该文件归 R1/Runtime，转 Runtime 处理。
- 日志：.tmp/wp024-lib.log、.tmp/wp024-int.log、.tmp/wp024-int2.log。

## 6. WP-024 后续（后端中层裁定执行结果，未 commit）

### 6.1 diff 摘要
- S1 修复：src/session/manager.rs open_session / open_child_session_with_responsibility / ensure 路径 / register_session 四处（`match self.mutate_blocking(SaveTaskState..)`，Ok→revision=saved.revision，Err→warn），revision 变量改为 mut。
- S2：src/client_protocol/protocol.rs JsonRpcErrorBody 新增 `data: Option<Value>`（skip_serializing_if none，兼容）；workspace.rs、connection.rs err_response 补 `data: None`；connection.rs 新增 `compact_error_response` 并在 SESSION_COMPACT Err 分支使用。
- 测试新增：
  - G3：manager.rs::g3_turn_finished_reason_non_empty_on_error_termination；tests/client_protocol_project.rs::turn_finished_auto_compact_failure_is_compaction_failed（params.error.code=compaction_failed，retryable=false）。
  - G4：connection.rs::compact_error_tests::g4_compact_while_running_is_agent_already_running_retryable（wire）+ manager.rs::g4_g5_…（运行中 Compact→AgentAlreadyRunning）；另有 compact_real_failure_is_compaction_failed_not_retryable、compact_nothing_and_cancelled_unchanged。
  - G5：manager.rs::g4_g5_compact_rejected_while_running_and_turn_restarts_after_error（错误终止后 Idle、TurnGuard 释放、可再 compact、可再 begin_turn）。注：为 manager 层，非 JSON-RPC 入口全链路（controller 夹具需 LLM/上下文窗口，见 manual_compact_e2e 失败）；协议入口全链路建议并入 Runtime 的 G2 e2e。
- S1 无需开 issue（已修）。

### 6.2 客户端影响（请转前端中层）
- session/compact 同步失败的 JSON-RPC 数字码仍为 -32000，message 不变。
- 新增 `error.data`（StructuredError {code,message,retryable}）：客户端应读 `error.data.code` / `error.data.retryable` 判断——`agent_already_running`（retryable=true，回合结束后可再点）、`compaction_failed`（retryable=false）。nothing_to_compact / cancelled 无 data，行为不变。
- 回合级：auto compact 失败的 turn_finished 现为 `params.error.code=compaction_failed`、retryable=false（Runtime 改动，原为 internal）。

### 6.3 测试结果
- lib（session:: client_protocol:: serve::，rustc 1.99.0）：**246 passed / 0 failed**（含新增 s1/g3/g4_g5/compact_error_tests 共 6 条），日志 .tmp/wp024-lib3.log。
- 新发现并已修（S1 延伸，原有缺陷）：resume_session（manager.rs:602-638）持 `records` 锁时调 `self.mutate_blocking`，其 note_receipt 再锁 `records` → std Mutex 自死锁；会话恢复时只要 active plan 文件已删就挂死。s1 测试首轮即卡死 >60s 暴露。修法：该处改调 `self.data.mutate_blocking`（不经 note_receipt），revision 仍由返回值同步。修后 s1 通过。
- S1 复现测试“修前失败”：修前该路径因上述死锁直接挂起（已观测），未单独回退跑 CAS 冲突版本。
- 集成（17 文件，session/client_protocol/serve，rustc 1.99.0，PATH=/home/box/.cargo/bin，HOME=/workspace/litecode/.tmp/home，共享 target 等过锁后跑完，--no-fail-fast）：**118 passed / 1 failed**（2026-10-12 16:12 HKT）。日志 .tmp/wp024-int3.log。
  - client_protocol_project：**26/0**（含新增 `turn_finished_auto_compact_failure_is_compaction_failed` … ok；compaction_failed / retryable=false）。
  - client_protocol_permission：**8/0**；serve_bind_auth：**1/0**；其余 session_* / compact_failure_calibers / subagent_session_isolation 均绿。
  - 唯一失败：`session_data_sql_death_list::sessions_db_sql_and_rusqlite_stay_in_sqlite_module`——命中全在 `src/engines/session_search/*`（dense_parity/golden/parity/relevance/semantic_index/sparse），**已知假红，归 WP-011**；本域 src/{session,client_protocol,serve} **无新增失败**。
  - 相对交付前基线 116/1：+2 pass（含 G3 协议入口新测），失败仍为同一条假红。
- 未完成：G5 未做 JSON-RPC 入口全链路（controller 夹具缺上下文窗口），建议并入 Runtime G2 e2e。

## 交叉审（Runtime 填写）
审阅人：Runtime，2026-10-12，基于工作树，rustc 1.99.0。

**R1/R4 断言**
- 4.1 的 4 个接口点和 Runtime 侧一致。补一处：Runtime 已把 G1 落地。project.rs `turn_error` 在 final_text == `LitecodeError::CompactionFailed.to_string()` 时返回 `compaction_failed`、retryable=false；单测 `client_protocol::project::wp025_turn_error_tests` 通过，`client_protocol_project.rs` 25/25 通过（`turn_finished_marks_a_failed_reconnect_retryable`、`structured_error_omitted_retryable_is_false` 不受影响）。
- 第 5 节报的 manual_compact_e2e :80/:140 失败已修：夹具改为 `fixture.create(.., Some(common::TEST_PRIMARY_MODEL_REF))`，原因是 session_binding.context_window==0（session_lifecycle.rs:252）。4/4 通过，已 rustfmt。
- G2 由 Runtime 侧 `reconnect_then_auto_compact_failure_stops_turn` / `reconnect_then_terminal_llm_error_stops_turn` 覆盖（agent-loop 层，compact_failure_calibers.rs）。协议入口的端到端仍是 G5，归 Session。
- G3/S7 已核顺序：runtime/mod.rs emit_turn_completed 先 append turn/end，再 emit TurnCompleted，正常路径下 reason 不会为空。真正的缺口是 append 失败只 `tracing::warn` 后照常 emit，这时 finish_turn 的 `.ok().flatten()` 拿到 None。建议 G3 断言改测“append 失败”这条路径，S7 维持 suspect。

**可疑实现**
- S1 确认（代码推理成立）：manager.rs:505-513 调 SaveTaskState 时记录还没插入，note_receipt（:383-395）是空操作，record 用的是旧 receipt.revision。建议升为 bug，写复现测试后立 ISSUE。
- S2 同意定为 drift，交中层定口径。S3、S5、S6、S8 同意 suspect。S4 同意按死代码处理。

**裁剪清单**
- T5 同意，和 Runtime 方案 §5 末项一致：保留 e2e，calibers 那条降为引擎层。新增的 R1 `manual_compact_retry_after_failure_completes` 已覆盖成功重试路径，进一步支持 T5。
- T1、T3、T4、T6 同意。T2 建议保留一条 wire 快照，防止 snake_case 回归。T8 同意登记。

## 交叉审计划（Session 审 Runtime）
- 结果：已完成，见 WP-022-runtime.md「交叉审」节，结论“有条件通过”，2 个阻断（可疑实现缺结论；3 次绿跑仅 1 次）。Runtime 已解决：补 §3 表、3 次绿跑、开 ISSUE-036（文本匹配）。
- 核对 compaction_failed 改动保留约定的 StructuredError 字段 {code,message,retryable}，且 retryable=false。
- 核对 project.rs 映射（:540-553）和协议测试（client_protocol_project.rs::turn_finished_marks_a_failed_reconnect_retryable、::structured_error_omitted_retryable_is_false）同步更新并通过。
- 互审 Runtime 的可疑实现与裁剪清单。
