# WP-025 / WP-022-runtime 方案（Runtime，仅方案，未开工）

## 1. R1 手动压缩失败→重试 e2e（牵头）
- 现状：`tests/manual_compact_e2e.rs::manual_compact_failure_e2e_releases_lease_and_retry_starts` 已在 HEAD(c6b24df)，走真实 `SessionController::start_manual_compact`（src/client_protocol/controller/session_lifecycle.rs:241）；`tests/compact_failure_calibers.rs::manual_compact_failure_releases_lease_for_retry` 为 runtime 层口径。
- 计划：先评估现有 e2e 是否只断言"重试能 start"；补 `manual_compact_retry_after_failure_completes`（同文件）：provider 注入首次 Err、第二次 Ok → 断言第二次 compact 完成、transcript 为压缩后形态、lease 释放、无残留 Compact phase。
- seam：沿用现有 provider 注入（tests/common），不新增产品 seam；若需"第 N 次失败"脚本化 provider，仅在 tests/common 加。
- 验收：`cargo test --test manual_compact_e2e` 与 `--test compact_failure_calibers` 全绿，各跑 3 次无 flake。

## 2. R4 重连后行为（牵头，与 Session 协作）
- Runtime（回合停止侧）：`tests/compact_failure_calibers.rs` 新增 `reconnect_then_auto_compact_failure_stops_turn`：模拟断线重连（provider 首个流中断→reconnect）后 compact_if_needed→Err，断言 TurnOutcome::Error、call_model 计数不再增长、snapshot 还原；`reconnect_then_terminal_llm_error_stops_turn`：不可重试错误直接终止。
- Session（controller/client 重试 UX 侧）：重连后 manual compact 可再点（lease/按钮状态/协议事件）。
- 交接点：Runtime 提供"失败后 lease 已释放 + TurnOutcome::Error"断言 helper；Session 在 controller 层复用并断言客户端可再次发起。双方产物互附交叉审段。

## 3. 可疑实现排查范围
- 文件：src/runtime/{mod,exec,context,pending_auto_turn,*_auto_turn,stream_projection,phase,provider_registry,llm_resolve}.rs、src/agent/**、src/types/**、src/config/**（manager/settings_writer/global_db）、src/llm/**。
- 方法：rg `lock()`/`.await` 同作用域（std Mutex 跨 await）、lease/guard 未 Drop 的早退路径（`?` 前已取 lease）、take() 后失败未归还（如 restore_custom_mailbox）、多步写入无回滚、spawn 后 JoinHandle 遗失、错误吞没（`let _ =`/`.ok()`）。
- 输出：file:line + 现象 + 复现/证据 + 严重度（issue-grading.md）；小问题域内修+补测，大问题开 ISSUE。
- 已关闭不重复：ISSUE-005（rusqlite 越界）、017（commit_mixed 半写入）、018（CompactionFailed 文案）、027（compact 失败三口径）。仅核回归。

## 4. 本域未提交文件处置
`git diff --stat`：7 文件 +15/−34，全部为纯 rustfmt 格式化（换行/多余空行/mod 排序），无语义变更。WP-006/007/008/014 的实质改动已在 HEAD c6b24df。
| 文件 | 变化 | 来源 | 处置 |
|---|---|---|---|
| config/manager.rs | 删 1 空行 | 未知（cargo fmt），非 runtime 手写 | 保留，需 owner 确认 |
| config/schema.rs | fn 签名换行 | 未知（fmt） | 保留，需 owner 确认 |
| config/settings_writer.rs | 删空行 + WP-008 测试内 fmt | WP-008 文件，fmt 来源未知 | 保留 |
| runtime/exec.rs | 2 处换行 | 未知（fmt） | 保留，需 owner 确认 |
| runtime/mod.rs | mod 排序 + 空行 + 签名 | 未知（fmt） | 保留，需 owner 确认 |
| runtime/stream_projection.rs | 测试内 use/链式调用折行 | 未知（fmt） | 保留，需 owner 确认 |
| types/death_list_gate.rs | format! 折行 + 空行 | 未知（fmt） | 保留，需 owner 确认 |
- 验证：`cargo fmt --check -- <files>`、`cargo build`、`cargo test --lib config:: runtime:: types::death_list_gate`。
- 提交：交 GM 合批，作为单独 "style: rustfmt" 提交，不混入语义改动。

## 5. 测试裁剪清单（只列不删）
- src/runtime/{bash,subagent,custom}_auto_turn.rs：三份结构同构的 auto-turn 测试（排队/触发/去重），建议抽公共参数化 helper，保留各自差异断言。
- tests/session_data_sql_death_list.rs vs src/types/death_list_gate.rs：SQL 死亡清单两处均有子串扫描，重复；且 session_search 等预存红灯（WP-012 归档）——建议单点化。
- death_list_gate::session_seq_g2_*：子串门禁易假绿/误杀（ISSUE-013，context 所有），建议放宽为 AST/符号级或标注。
- ~~compact_failure_calibers::manual_compact_failure_releases_lease_for_retry 与 manual_compact_e2e 部分重叠：e2e 补全后可将前者降为单元级或合并。~~ **撤回（2026-10-12，采纳 Session 交叉审）**：保留，作为 runtime 层快速回归，不合并不降级。

## 6. 交叉审
- Session↔Runtime：互审 R1/R4 及对方清单（我审 WP-024 / WP-022-session.md）。
- 上下文↔Runtime：互审 compact/reminder 与回合交界（我审 WP-026 / WP-022-context.md）。
- 中层抽样核对 R1/R4。

## 7. 执行步骤 / done_when / 风险
1) 获批后 start；2) R1 补测；3) 与 Session 对齐 helper 后写 R4 runtime 侧；4) 排查出清单并修小项；5) fmt 文件验证并交 GM；6) 交叉审；7) review。
- done_when：R1/R4 测试 3 连绿；可疑清单含 file:line+严重度；处置表落地；交叉审段齐；approver close。
- 风险：重连模拟需 provider 流中断注入，若现有 fake 不支持需在 tests/common 加（报中层）；fmt 改动归属未知；死亡清单预存红灯干扰全量跑。


---

# WP-025 实施记录（Runtime，2026-10-12，未 commit，未 deliver）

工具链：`PATH=/home/box/.cargo/bin:$PATH`，rustc 1.99.0（满足 ≥1.88），共享 target 等过锁。

## 改动文件
- tests/manual_compact_e2e.rs
  - R1：新增 `manual_compact_retry_after_failure_completes`。第一次空摘要失败，第二次给出真实摘要并成功。断言：两次是不同 op；`Compacted` 恰好 1 条；working set 变小；lease 已释放，`is_compacting` 为 false。
  - 夹具修复：两处 `fixture.create` 都改为 `Some(common::TEST_PRIMARY_MODEL_REF)`。原来 session_binding.context_window==0，被 session_lifecycle.rs:252 拒绝，报 Config("no context window…")，旧测试在 1.99 下同样失败。
  - 全文件已 rustfmt。
- tests/common/fake_deps.rs：新增 `compact_fail_from_step: Option<u64>` 和 `model_errors: RefCell<Vec<(usize, LitecodeError)>>`，按 call_model 下标注入错误。默认值不变，现有测试不受影响。
- tests/compact_failure_calibers.rs：R4 新增 `reconnect_then_auto_compact_failure_stops_turn` 和 `reconnect_then_terminal_llm_error_stops_turn`。重连在 call_model 内部吸收，所以第 1 步给出重连后的工具调用，测第 2 步：
  - auto compact 失败：得到 Error(CompactionFailed)，model_calls=1。
  - 终端 Llm 错误：得到 Error(Llm)，model_calls=2，不再 compact。
- src/client_protocol/project.rs（追加范围，已获批）：`turn_error` 遇到 TurnEndReason::Error 且 final_text 等于 `LitecodeError::CompactionFailed.to_string()` 时，返回 `ErrorCode::CompactionFailed`，retryable=false；其他情况映射不变（retryable 时为 llm_http，否则为 internal）。新增单测 `wp025_turn_error_tests::auto_compact_failure_maps_to_compaction_failed_not_retryable`。
  - 取舍：用错误文本识别，没有给 TurnEndReason 加枚举。加枚举会波及持久化 schema（turn/end.reason）和 session 层。这样做依赖 runtime/mod.rs:893 的 `err.to_string()`，CompactionFailed 文案再改也会自动跟上，因为比较的是同一个 Display。
  - 客户端影响：`web/src/api/types.ts` 的 ErrorCode 已包含 `"compaction_failed"`，无需改类型。rg 查过 web/src，没有代码按回合错误 code=="internal" 分支，现有消费方只把 code 原样展示。auto compact 失败的回合从此显示 compaction_failed，不提示重试。

## 测试结果（rustc 1.99.0）
- compact_failure_calibers：7/7 通过，包括 R4 的 2 条和原有 3 条口径测试。
- manual_compact_e2e：4/4 通过（R1 新测试和修复后的旧 e2e）。
- agent_loop：12/12 通过。
- client_protocol_project：25/25 通过。
- lib `client_protocol::project`：1/1 通过（新单测）。
- 方案要求的连跑 3 次没跑完：第二、三轮被中断，目前各只有 1 轮全绿。

## 交叉审（Session 填写）
审阅人：Session，2026-10-12。只读审查，未改 Runtime 代码。复跑环境：rustc 1.99.0。

### 1. R1/R4 断言与测试
- 夹具修复正确：tests/manual_compact_e2e.rs:67、:133 已改为传 `Some(common::TEST_PRIMARY_MODEL_REF)`，对应 session_lifecycle.rs:252 的 context_window==0 拒绝。认可。
- 复跑结果：`cargo test --test manual_compact_e2e --test compact_failure_calibers` 显示 manual_compact_e2e 4/4 通过，compact_failure_calibers 7/7 通过（含 reconnect_then_auto_compact_failure_stops_turn、reconnect_then_terminal_llm_error_stops_turn）。只跑了 1 轮，方案要求的“3 连绿”仍未完成。
- R4 的测试只覆盖 runtime 层的 TurnOutcome，没有覆盖协议层：重连耗尽后客户端收到的 turn/end error.code 是什么，以及之后能否再发 agent/retry、start_turn。这部分由 Session 侧的 G3–G5 补齐（见 WP-022-session.md），这里不算 Runtime 的缺口。

### 2. auto compact 失败映射（src/client_protocol/project.rs:537-563）
- 字段：StructuredError {code,message,retryable} 三个字段都在（:550-554）。code=CompactionFailed，retryable=false，message 是 Display 文本。与 S2 约定一致：数字 -32000 不变，data.code 为 compaction_failed，retryable 为 false。
- 匹配顺序：compaction 判断在 reconnect 判断之前（:548-556 早于 :557），所以即使 reconnect 处于 Failed，也不会误判成 llm_http。正确。
- 其他错误：非 compaction 文本仍走原逻辑（:557-566），retryable 时为 llm_http，否则为 internal。单测 project.rs:1222-1225 只测了 internal 分支；llm_http 分支由 tests/client_protocol_project.rs:565 覆盖。
- 风险（非阻断）：映射靠文本相等（:548-549）识别，依赖两点：一是 src/types/error.rs:52-53 的文案（"compaction failed: empty summary"）是固定的、不带参数；二是 runtime/mod.rs:893-894 原样传 `err.to_string()`。以后如果 CompactionFailed 带上原因字段（文案里有动态内容），或者别的错误恰好产生相同文本，映射会悄悄失效或误判。error.rs:104-109 的文案断言能挡住一部分。建议后续在 emit_turn_completed 旁增加一个类型化的错误类别，比如 turn/end 内存事件里的 error_kind，不落盘，这样就不用改 TurnEndReason 的持久化 schema，可以单开 issue 跟进。本轮接受文本匹配。
- 协议测试：tests/client_protocol*.rs 里还没有 wire 级断言 `params.error.code=="compaction_failed"`，rg 只找到 :565 的 llm_http。建议补一条 wire 级测试，可由 Session 在 G3 里一并覆盖。

### 3. 可疑实现清单
- 本产物 §3 只写了排查范围和方法，没有给出 file:line 结论清单。“可疑实现”这一项未交付，无法逐条审，请补交（无发现也要写明“已排查、无发现”及范围）。
- §3 已关闭不重复的 ISSUE-005/017/018/027：同意只核回归。

### 4. 测试裁剪清单（§5）
- *_auto_turn 三份同构测试抽参数化 helper：同意，保留各自的差异断言。
- session_data_sql_death_list 与 death_list_gate 重复：同意单点化。注意 session_search 的假红已归到 WP-011/012，单点化前要先确认该红灯的归属，避免误删真正的门禁。
- session_seq_g2_* 子串门禁放宽：同意标注。改成 AST 级属于 context 的 ISSUE-013 范围，Runtime 不应自行改动。
- compact_failure_calibers:manual_compact_failure_releases_lease_for_retry 与 e2e 重叠：部分同意。e2e 依赖 controller 和 sqlite，前者是 runtime 层的快速口径，建议保留前者作为单元级回归，不要合并删除。

### 结论：有条件通过
阻断项：
1. 补交可疑实现清单（file:line+严重度，或写明无发现）。
2. R1/R4 补完 3 连绿记录。
非阻断项：文本匹配改为类型化（开 issue）；补 compaction_failed 的 wire 级断言（Session G3 承接）。


---

# WP-025 补充（响应 Session 交叉审，2026-10-12）

## §3 可疑实现结论（原阻断项 1）
排查范围：src/runtime/{mod,exec,pending_auto_turn,bash_auto_turn,custom_auto_turn,subagent_auto_turn,stream_projection,observer}.rs、src/agent/core.rs、src/llm/{reconnect,codec/*}.rs、src/config/{settings_writer,workspace,workspace_identity,log_filter}.rs。方法：rg `let _ =`、`.ok()`、`lock()`，逐个读上下文。

| # | 位置 | 现象 | 级别 | 处置 |
|---|---|---|---|---|
| R-1 | src/agent/core.rs:43-45 | auto compact 期间用户取消被映射成 Error，而不是 Cancelled（:56 是正确写法） | bug | ISSUE-033，**已修**（见下） |
| R-2 | src/runtime/exec.rs:294、386-388、462-467；src/reminder/file_tracker.rs:66-81 | 改动文件在 seam 开头就被 take_changes 消费，之后任一步失败都不会归还，永久丢失 | bug | ISSUE-032，已确认；跨域修复草案见下，等 GM 另开 WP |
| R-3 | src/runtime/mod.rs:829-832 | append turn/end 失败只打 warn，`let _ = finish_turn(..)` 的结果也被吞，随后照常 emit TurnCompleted，finish_turn 读到的 reason 为 None（与 Session S7 同根） | suspect | 记录；建议 append 失败时把 reason 直接带给 finish_turn |
| R-4 | src/runtime/bash_auto_turn.rs:181、pending_auto_turn.rs:190 | `let _ = thread::Builder::spawn(..)`，线程起不来时 idle auto-turn flush 静默失效 | suspect（低） | 建议至少 warn |
| R-5 | src/client_protocol/project.rs:537-563 | compaction_failed 靠文本匹配识别 | drift | ISSUE-036（新立，非阻断） |

复核后判定无问题：settings_writer.rs:376/402/461 的 broadcast send（没有订阅者时 Err 属正常）；llm/codec/* 中的 `let _ =` 都在测试的 mock server 里；stream_projection、provider_registry 的 std Mutex 都没有跨 await 持有。

## ISSUE-033 修复（后端中层已批准）
- src/agent/core.rs：`compact_if_needed` 返回 `Err(LitecodeError::Canceled)` 时改为 `TurnOutcome::Cancelled { final_text }`，写法与 call_model 分支一致。
- 测试：新增 tests/agent_loop.rs::`cancel_during_auto_compact_is_cancelled_without_model_call`；tests/common/fake_deps.rs 新增 `compact_cancel` 字段。
- ISSUE-033 已 closed，history 和 evidence 已追加。

## ISSUE-032 确认与跨域修复草案（不在本包修，等 GM 另开 WP）
- 确认：exec.rs:294 `take_changed_files` 会同时推进 file_tracker 快照（file_tracker.rs:66-81 `take_changes` 直接 insert 新快照）。之后如果 spine_reminder_view 失败（:386-388，只调 restore_seam_sources，不归还 changed），或者 FilesChanged 提醒落库失败（:462-467），这些改动永远不会再被报告。
- 草案（三域）：
  - context：file_tracker 增加 `mark_dirty(paths)`（或 `forget(paths)`），把快照回退或删除，让下次 take_changes 再次报出这些路径。
  - Session：增加 `restore_changed_files(sid, paths)`，转调 tracker 的 mark_dirty。
  - Runtime：在 exec.rs:386 的失败分支和 FilesChanged 落库失败分支（:462 附近，返回 Err 之前）调用 `restore_changed_files`；补 2 条失败注入测试。

## 其他
- sql death_list 假红的归属：Session 记在 WP-011，Runtime 方案 §5 写的是 WP-012。**合并前需中层确认归属。**
- 3 次连绿：Session 跑过 1 轮（e2e 4/4、calibers 7/7）。本侧结果见下一节。

## 最终测试记录（rustc 1.99.0，PATH=/home/box/.cargo/bin，共享 target 等锁；日志 .tmp/wp025-runs.log）
在最终代码上跑（含 ISSUE-033 修复和 project.rs 映射改动）：
- agent_loop：13/13 通过（含新增 cancel_during_auto_compact_is_cancelled_without_model_call）。
- client_protocol_project：26/26 通过。
- manual_compact_e2e + compact_failure_calibers：**3 轮连跑全绿**，每轮 e2e 4/4、calibers 7/7。
  - 另有修复前的 2 轮也全绿，Session 独立跑过 1 轮。
- lib 单测 `client_protocol::project::wp025_turn_error_tests` 在改映射时跑过，1/1 通过。
  - 最终代码上的 lib 复跑因共享 target 被其他域长时间编译占用而中断，未完成。core.rs 的改动已由 agent_loop 覆盖。
