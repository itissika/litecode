# WP-021 子包 A · 后端汇总（WP-022，后端中层）

- 基线：分支 ops/team-framework，HEAD `c6b24df` + 未提交工作区（`git status` 99 行）。未 commit。中层未改 src/、tests/。
- 工具链：`PATH=/home/box/.cargo/bin:$PATH`，rustc 1.99.0；共享 target，排队等锁。
- 来源：WP-022-{session,runtime,context,tools,ide}.md、WP-021.md「子包 A」、issues ISSUE-007/032–040、board.json。
- 更新于 2026-10-12 16:2x HKT：五个子包 WP-024/025/026/027/028 均已 done。（16:11 前后 box 重启，文件已恢复。）

## 1. 总览

| 域 | 子包 | 状态 | 测试结果（本域自报） | 可疑项（数 / 最高级） | 裁剪清单条数 | 交叉审 |
|---|---|---|---|---|---|---|
| Session | WP-024 | done | lib 246/0；集成 118/1（基线 116/1，新增 2 条均过），唯一失败为 sql death_list 假红（src/engines/session_search/*，无归属 WP，见 §7） | 8 / bug（S1，已修）；另修 resume_session 自死锁 | 8 | Runtime 审 Session：完成，同意；Session 审 Runtime：有条件通过，条件已补 |
| Runtime | WP-025 | done | agent_loop 13/13；client_protocol_project 26/26；e2e 4/4 + calibers 7/7 连续 3 轮；lib 在最终代码上未复跑 | 5 / bug（R-1 已修=ISSUE-033；R-2=ISSUE-032） | 4（1 条撤回） | 完成（审 Context 6 条同意；被 Session 审） |
| 上下文 | WP-026 | done | reminder::sources 9、reminder:: 34、context_pipeline 47、death_list 13、stage_a 27，全过 | 12（含 A 级 2 条）/ bug（ISSUE-035、032、033） | 10（4 重复、2 可合并、其余保留/不动） | 完成（Runtime 审，6 条同意） |
| Tools | WP-027 | done | check 过；clippy 改动文件 0 条；新测 4 + 修复测 3 全过；tools:: 20 连跑 background_real_process 20/20；lib/集成本域全过；lsp windows 测试已 cfg；optional_lsp:1062 断言确认过时（b4f04e7 有意改为 format_file_line 编号行），已更新并通过 | 14 / arch 高（B8=ISSUE-034） | 7 + 补充 3 = 10 | 完成：IDE 审 custom.rs（X1/X2/X3+B7→ISSUE-039，X4→ISSUE-040，X5 部分反驳，X6/X7 确认）；Tools 审 IDE（terminal 退出码丢失，低；pty Drop 最多阻塞约 5s） |
| IDE 基座 | WP-028 | done | workspace_api 12/12、path_consumer 4/4、path_lap 5/5、terminal 3/3、path_death 4/4、process_contract 7/7；hotswitch death_list FAIL（ISSUE-007）；optional_lsp 23/1（已判测试过时，Tools 已改） | 7 / 中（S2=ISSUE-037 bug 已复现；S3 suspect） | 6 | 完成（双向） |

## 2. 四条线索结论

- **Broken pipe（Tools）**：根因是竞态——子进程不读 stdin 先退出，父进程随后写 stdin 得 EPIPE，被判工具失败。修法：stdin 改独立线程写（`spawn_stdin_writer`），BrokenPipe 视为 Ok，以退出码为准；同时修掉同步写 stdin 与读 stdout 互等的死锁。验证：`child_ignoring_stdin_is_not_a_broken_pipe_failure` 通过；`background_real_process_begin_job_then_mailbox_notice` 在 tools:: 全量 16 线程下 20/20 通过。中层半成品采用。
- **R1 手动压缩失败后重试**：新增 `manual_compact_retry_after_failure_completes`；原 e2e 失败是夹具未配上下文窗口（context_window==0），已修夹具。e2e 4/4 + calibers 7/7 连续 3 轮全绿。
- **R3 reminder 来源专属单测**：新增 `src/reminder/sources/tests.rs`，9 个 source 各有触发与抑制用例，`reminder::sources` 9/9。
- **R4 断线重连后停回合、可再点**：Runtime 侧 `reconnect_then_auto_compact_failure_stops_turn`、`reconnect_then_terminal_llm_error_stops_turn`（agent-loop 层）；Session 侧 G3–G5（manager 层与 wire 级 compact 错误码）。协议入口全链路 e2e（G5）未做，转给 Runtime 后续补。

## 3. 已修复项（均附测试，未 commit）

| 项 | 位置 | 测试 |
|---|---|---|
| Broken pipe / stdin 死锁 / stderr 顺序读 / 输出含 cancelled 误判 / 孙进程组杀（A1–A7） | src/tools/custom.rs | child_ignoring_stdin_…、large_output_before_reading_stdin_does_not_deadlock、background_output_mentioning_cancelled_is_ok、timeout_kills_grandchild_process |
| B5 wait 失败 panic → WaitFailed 返回 Error | src/tools/custom.rs | wait_failure_returns_error_instead_of_panicking |
| B6 后台线程创建失败被吞 → abandon_job + Error | src/tools/custom.rs、custom_hub.rs | background_spawn_failure_is_reported_and_job_released |
| B12 RwLock 中毒连环 panic | src/optional/mod.rs | poisoned_state_lock_does_not_cascade_panics |
| ISSUE-033 压缩中取消 → Cancelled（已 closed） | src/agent/core.rs:43-48 | tests/agent_loop.rs::cancel_during_auto_compact_is_cancelled_without_model_call |
| auto compact 失败回合错误码 internal → compaction_failed（retryable=false） | src/client_protocol/project.rs:545-555 | project::wp025_turn_error_tests；client_protocol_project::turn_finished_auto_compact_failure_is_compaction_failed |
| session/compact 同步错误 error.data：agent_already_running（retryable=true）/ compaction_failed（false），数字码仍 -32000 | src/client_protocol/connection.rs（compact_error_response）、protocol.rs | connection.rs::compact_error_tests |
| Session S1 open 吞 SaveTaskState、revision 落后 DB；resume_session 持锁重入自死锁 | src/session/manager.rs | s1_reopen_prune_keeps_revision_in_sync |
| lsp Windows 路径测试在 Linux 必挂 → `#[cfg(windows)]` | src/tools/lsp.rs:1086 | tools::lsp 26/0 |
| optional_lsp 断言过时（行号格式已统一） | tests/optional_lsp.rs | agent_lsp_tool_line_text_scenarios 通过 |

## 4. bug 级未修项

| issue | 级别 | 域 | file:line | 现象 |
|---|---|---|---|---|
| ISSUE-032 | bug | runtime（跨 context/session） | src/runtime/exec.rs:294、386-388、462-467；src/reminder/file_tracker.rs:66-81 | 改动文件在 seam 开头被消费，失败不归还，永久丢失 |
| ISSUE-034 | arch 高 | tools（归属待定） | src/mcp/client.rs:57-94；src/mcp/pool.rs:285-289 | MCP 请求无超时、不按 id 匹配，持锁全程 |
| ISSUE-035 | bug | context | src/context_pipeline/compact.rs:283-323（:286 revision 现取 unwrap_or(0)） | 已落库后 reload 失败仍回滚内存并发 Failed |
| ISSUE-037 | bug 中 | ide | src/workspace/service.rs:270-287、404-433 | 并发 create/write(overwrite=false) 静默覆盖，300/300 复现 |
| ISSUE-038 | bug 低 | ide | src/workspace/service.rs:347-357 | 跨盘覆盖移动直接报 CrossesDevices |
| ISSUE-039 | 中 | tools | src/tools/custom.rs:449-457、334-366、339-346 | 孙进程持管道时超时/取消失效且不清理（X1+X2+X3，含 B7） |
| ISSUE-040 | drift | tools | src/tools/custom.rs:306-314 | 未接入 bind_child_to_parent，Windows 宿主崩溃不级联 |
| ISSUE-036 | drift（非阻断） | runtime | src/client_protocol/project.rs:545-555 | compaction_failed 靠文本匹配识别 |
| ISSUE-007 | 门禁假红 | ide | tests/workspace_hotswitch_death_list.rs | 子串误报 8 处 |

另（低，只记录，来自 Tools 审 IDE）：src/terminal/mod.rs:195-196、:451-453 kill 后只做一次 try_reap，exit 事件退出码大概率为 None；src/terminal/pty.rs:278 Drop→wait_join 最多阻塞约 5s，最后一个 Arc 不应在 tokio worker 上释放。

suspect 级（只记录，不开 issue）：Session S3–S8；Runtime R-3、R-4；Context 1b/1c/1e/2/2b/2c/4/6/7；Tools B9、B11、B13、B14；IDE S3–S7。

## 5. 测试裁剪 / 放宽 / 重复清单（只列不删，待 Andy 拍板）

**Session（8）**
- 重复：client_protocol_telemetry 的 subscribe/unsubscribe_logs_is_jsonrpc_method 合一；session_list_empty_filter 6 条收成表驱动；manual_compact_e2e 与 compact_failure_calibers 的 lease 释放（部分重复，Runtime 主张保留 calibers 作快速回归，见待裁定）。
- 放宽：client_protocol_permission 三条序列化快照留一条完整快照；session_data_lifecycle.rs:77 sleep(50ms)；session_turn_entry 两条源码 grep 守门。
- 可删（实现侧）：connection.rs deferred 死代码分支。
- 登记：session_data_sql_death_list 假红。

**Runtime（3 有效，1 撤回）**
- 重复：bash/subagent/custom_auto_turn 同构测试抽参数化 helper。
- 重复：session_data_sql_death_list 与 death_list_gate 的 SQL 子串扫描单点化。
- 放宽：death_list_gate::session_seq_g2_* 子串门禁（属 ISSUE-013 / context）。
- 撤回：calibers 与 e2e 合并。

**上下文（6 条有动作）**
- 重复：reminder/engine.rs bash_exit_names_a_user_kill、custom_tool_settled_names_outcomes、plan_changed_dedupes_on_revision、step_budget_once_per_turn_inside_the_window。
- 收窄：engine.rs restore_fires_once_after_compaction 去掉与 sources 重复的文案断言。
- 可合并：budget.rs token_count_* 两条；keep_recent.rs entire_transcript_within_keep_returns_none / empty_returns_none。
- 不动：death_list_gate（13）、stage_a 等跨域门。

**Tools（10）**
- 放宽：background_real_process_begin_job_then_mailbox_notice 等待放宽到 10s；ask_user 多问题文案断言；plan.rs "No plan file was created"；tools::bash 两条 live_* 与 bash_jobs_contract 一条（20 连跑 2–4 次失败）。
- 重复：permission_pipeline 纯 sink 往返 vs client_protocol_permission；knowledge/spec_tests 错误文案 vs index_tests reason。
- 可删候选：webfetch whatwg 解析测试（只测第三方 crate）。

**IDE 基座（6）**
- 放宽：workspace_hotswitch_death_list 改为按已删除符号禁（ISSUE-007），不删。
- 重复：workspace_path_death_list 白名单自检可并入 no_bare_canonicalize；hotswitch 清单去掉 api/workspace/open，交 workspace_open_http_endpoint_is_gone 守；rel_path_under_* 三条合并到 workspace_path_lap.rs。
- 保留：terminal_bypass_death_list、workspace_api CRUD。

## 6. 抽样核对（中层读当前工作区代码）

| 域 | 抽查项 | 结果 |
|---|---|---|
| Session | S8 serve/state.rs:99 `list_session_ids_blocking().unwrap_or_default()` | 一致 |
| Session | S6 manager.rs:1715 `let _ = self.remove_session` | **行号漂移**：实际在 :1737（S1/S2 改动后行号下移），内容一致 |
| Session | S5 connection.rs:88-93 resolve_sid | **行号漂移**：实际 fn 在 :120-124，:624/:815 有 `first_session_id().unwrap_or_default()`；内容一致 |
| Session | 裁剪 T4 session_data_lifecycle.rs:77 sleep(50ms) | 一致 |
| Session | 裁剪 T1 telemetry :39/:49 两条测试存在 | 一致 |
| Runtime | R-1 core.rs:43-48 已有 Canceled→Cancelled 分支 | 一致（修复已落地） |
| Runtime | R-3 runtime/mod.rs:829-832 append 失败仅 warn、`let _ = finish_turn` | 一致 |
| Runtime | R-4 bash_auto_turn.rs:181、pending_auto_turn.rs:190 `let _ = thread::Builder` | 一致 |
| Runtime | 裁剪：三份 *_auto_turn 测试存在（4/3/3 条） | 一致 |
| Runtime | 裁剪：death_list_gate::session_seq_g2_* 存在（:522） | 一致 |
| 上下文 | ISSUE-035 compact.rs:286 `revision_blocking(..).unwrap_or(0)`、:294 `*rows = snapshot` | 一致 |
| 上下文 | ISSUE-032 file_tracker.rs:66-81 take_changes 读即覆盖快照；exec.rs:294 先消费、:386-388 只回灌 seam sources | 一致 |
| 上下文 | #6 mentions.rs:72-88 去重键含原始 path 字符串 | 一致 |
| 上下文 | 裁剪：engine.rs 4 条重复测试存在（:169/:191/:253/:282） | 一致 |
| 上下文 | 裁剪：keep_recent.rs :257/:263、budget.rs :167/:174 | 一致 |
| Tools | ISSUE-034 client.rs:75 read_line 无超时、:82 不比对 id；pool.rs:285 持锁跨 initialize/call_tool | 一致 |
| Tools | lsp.rs:1086 `#[cfg(windows)]` | 一致 |
| Tools | 裁剪：plan.rs:213/:425 文案断言 | 一致 |
| Tools | 裁剪：custom.rs:918 background_real_process 测试存在 | 一致 |
| IDE | S2 service.rs:270-287 先 `exists()` 后 atomic_write | 一致 |
| IDE | S1 service.rs:349-357 file-replace 分支排在 cross-device 分支前 | 一致 |
| IDE | S3 lsp/server.rs:771-776 try_lock 失败静默跳过 kill | 一致 |
| IDE | S5 proc_lifetime.rs:244-265 注释提 subreaper，代码只认 ppid==1 | 一致 |
| IDE | 裁剪：workspace_path_death_list :67/:145、workspace_process_contract :166 | 一致 |

结论：抽查 24 项，证据全部成立；Session 两处行号因本包后续改动漂移，不影响结论。

### 抽样实跑（中层，2026-10-12 16:38–16:43 HKT）
环境：工具集管家修复后（rustc 1.99.0，g++ 已装），本 shell 需加 `RUSTUP_HOME=/workspace/litecode/.tmp/home/.rustup RUSTUP_TOOLCHAIN=stable` 才能选中工具链。每条 `timeout 900`，共享 target 未遇长时间锁等待。日志 .tmp/wp021_runs.log、.tmp/wp021_{1..7}.log。之前「缺 c++ 编译失败」的记录作废。

| # | 命令 | 结果 | 覆盖的修复项（已逐条确认在输出中为 ok） |
|---|---|---|---|
| 1 | `cargo test --lib tools::` | 392 passed / 0 failed（16:41 完成，含首次编译） | Broken pipe：child_ignoring_stdin_is_not_a_broken_pipe_failure、background_real_process_begin_job_then_mailbox_notice；A1/A5–A7：large_output_before_reading_stdin_does_not_deadlock、background_output_mentioning_cancelled_is_ok、timeout_kills_grandchild_process；B5 wait_failure_returns_error_instead_of_panicking；B6 background_spawn_failure_is_reported_and_job_released；tools::lsp 21 条 ok（Windows 用例已 cfg 跳过） |
| 2 | `cargo test --lib optional::` | 7 / 0 | B12 poisoned_state_lock_does_not_cascade_panics |
| 3 | `cargo test --lib reminder::sources` | 9 / 0 | R3 |
| 4 | `cargo test --lib session::` | 207 / 0 | S1 s1_reopen_prune_keeps_revision_in_sync（同时覆盖 resume_session 自死锁，修前该测试卡死）；G3 g3_turn_finished_reason_non_empty_on_error_termination；G4/G5 g4_g5_compact_rejected_while_running_and_turn_restarts_after_error |
| 5 | `cargo test --lib client_protocol::` | 29 / 0 | S2 compact_error_tests 3 条（agent_already_running/true、compaction_failed/false、nothing/cancelled 不变）；wp025_turn_error_tests |
| 6 | `cargo test --test agent_loop --test client_protocol_project --test manual_compact_e2e --test compact_failure_calibers` | 13/0、26/0、4/0、7/0 | ISSUE-033 cancel_during_auto_compact_is_cancelled_without_model_call；compaction_failed wire 级 turn_finished_auto_compact_failure_is_compaction_failed；R1 manual_compact_retry_after_failure_completes；R4 reconnect_then_* 两条 |
| 7 | `cargo test --test optional_lsp agent_lsp_tool_line_text_scenarios` | 1 / 0 | optional_lsp 过时断言修正 |

结论：7 组全部通过，无失败、无未跑完。说明：resume_session 死锁没有单独命名的测试，靠 s1 测试间接覆盖；Session 的 lib 自报 246 是 session::+client_protocol::+serve:: 合计，本次只跑了前两者（207+29）。

## 7. 待 GM 裁定

1. **仅格式化（rustfmt）补丁集**，建议单独一个 "style: rustfmt" 提交，不混语义改动：
   - Session：src/client_protocol/connection.rs 中的 fmt hunk（该文件另含 S2 语义改动，需按 hunk 拆）、src/session/data/sqlite/ops.rs、src/session/manager.rs 中的 fmt hunk（同含 S1 语义改动）。
   - Runtime：src/config/{manager,schema,settings_writer}.rs、src/runtime/{exec,mod,stream_projection}.rs、src/types/death_list_gate.rs（fmt 来源未知，需 owner 确认）。
   - 上下文：src/context_pipeline/compact.rs、src/reminder/{engine,kinds,mentions,mod}.rs（另：reminder/sources/mod.rs 挂载 + 新 sources/tests.rs 属 R3 功能，非 fmt）。
   - Tools：src/cli/cli.rs（仅 AskAnswer 换行，归 Tools）、src/knowledge/{corpus,fsref,index,index_tests,mentions,spec_tests,validate}.rs、src/permission/{mod,sinks}.rs、src/tool/{availability,registry}.rs、src/tools/{ask_user,bash_jobs_contract,custom_hub,knowledge,litecode_workspace,mod,plan,webfetch,websearch}.rs、tests/{client_protocol_permission,common/permission,permission_pipeline}.rs。
   - IDE：src/workspace/service.rs（refuse_top_level_symlink_copy 签名 fmt，位于 symlink 禁区 WP-004；随批提交，或由批次负责人 `git checkout -- src/workspace/service.rs`）。
   - 冻结只记录：src/engines/code_search/embed.rs（测试内一个 assert 拆行，引擎产品禁区 WP-016）。
   - 另：全仓 `cargo fmt --check` 曾在 src/reminder/sources/tests.rs、tests/manual_compact_e2e.rs 有差异，提交前需再跑一次。
2. **ISSUE-034** MCP send_request 归属与修复包（协议层改造，非小修）。
3. **ISSUE-032** 需跨域 WP：context 提供 file_tracker `mark_dirty/forget`，Session 提供 `restore_changed_files`，Runtime 在 exec.rs 失败分支调用并补失败注入测试。
4. **ISSUE-035** compact reload 失败回滚：Runtime 建议非阻断，修复包与时机待定。
5. **ISSUE-037 / 038** workspace 修复包（探针已在 tests/wp028_probe_*，默认 ignore）。可与 ISSUE-039/040（custom 进程生命周期）一并考虑。
6. **issue 编号并发分配冲突**：Tools 误写 ISSUE-031.json 覆盖了上下文域未入 git 的同名文件，内容已以 ISSUE-035 重建；ISSUE-031 号空缺。建议 ops 工具分配编号时做存在性检查或加锁。
7. **sql death_list 假红无归属**：session_data_sql_death_list::sessions_db_sql_and_rusqlite_stay_in_sqlite_module（命中 src/engines/session_search/*；Session 集成 118/1 中唯一失败），Session 记 WP-011、Runtime 记 WP-012，两者已关闭，需指定 owner。
8. **ISSUE-007** 热切换死亡清单子串误报：建议改为按已删除的符号禁；config/gate 命中的是字段名 ApplyPlan.reload_workspace，词边界去不掉。
9. **协议变更已通知前端中层**：session/compact 失败与回合级 auto compact 失败，客户端应读 `error.data.code` / `retryable`（agent_already_running / compaction_failed）。
10. **engines/code_search/embed.rs** 冻结，只记录。
11. 其他：Runtime 测试裁剪 T5（calibers 保留与否）Session 与 Runtime 已一致为保留；G5 协议入口 e2e 转 Runtime 后续补。

## 8. 未提交改动处置表（`git status -- src tests`，16:17 HKT，共 59 个）

类型：纯格式化 / 行为改动 / 混合需按 hunk 拆 / 新增测试 / 冻结只记录。处置均为「随 GM 批次提交」，不由中层 commit。

| 文件 | 域 | 类型 | 处置 |
|---|---|---|---|
| src/client_protocol/connection.rs | Session | 混合需按 hunk 拆（rustfmt + S2 compact_error_response） | fmt hunk 进 style 补丁；S2 进行为提交 |
| src/session/manager.rs | Session | 混合需按 hunk 拆（rustfmt + S1 修复、resume_session 死锁修复、新测试） | 同上 |
| src/session/data/sqlite/ops.rs | Session | 纯格式化 | style 补丁 |
| src/client_protocol/protocol.rs | Session | 行为改动（JsonRpcErrorBody 增 data） | 行为提交 |
| src/client_protocol/workspace.rs | Session | 行为改动（err_response 补 `data: None`，配套 S2） | 行为提交 |
| tests/client_protocol_project.rs | Session / Runtime | 新增测试（G3 wire 级 compaction_failed 等） | 随行为提交 |
| src/agent/core.rs | Runtime | 行为改动（ISSUE-033） | 行为提交 |
| src/client_protocol/project.rs | Runtime | 行为改动（compaction_failed 映射 + 单测） | 行为提交 |
| src/runtime/exec.rs、src/runtime/mod.rs、src/runtime/stream_projection.rs | Runtime | 纯格式化 | style 补丁 |
| src/config/manager.rs、src/config/schema.rs、src/config/settings_writer.rs | Runtime（fmt 来源未知） | 纯格式化 | style 补丁 |
| src/types/death_list_gate.rs | Runtime（门禁属 context） | 纯格式化 | style 补丁 |
| tests/agent_loop.rs | Runtime | 新增测试（ISSUE-033） | 随行为提交 |
| tests/common/fake_deps.rs | Runtime | 新增测试（夹具：compact_fail_from_step、model_errors、compact_cancel） | 随行为提交 |
| tests/compact_failure_calibers.rs | Runtime | 新增测试（R4 两条） | 随行为提交 |
| tests/manual_compact_e2e.rs（未跟踪） | Runtime | 新增测试（R1 + 夹具修复） | 随行为提交；见 §9 注 |
| src/context_pipeline/compact.rs | 上下文 | 纯格式化 | style 补丁 |
| src/reminder/engine.rs、kinds.rs、mentions.rs、mod.rs | 上下文 | 纯格式化 | style 补丁 |
| src/reminder/sources/mod.rs | 上下文 | 行为改动（挂载 `#[cfg(test)] mod tests;`） | 随 R3 提交 |
| src/reminder/sources/tests.rs（未跟踪） | 上下文 | 新增测试（R3） | 随 R3 提交 |
| tests/stage_a_ctx_consistency.rs | 上下文（产物未单列，待确认） | 纯格式化（-1 行） | style 补丁 |
| src/tools/custom.rs | Tools | 混合需按 hunk 拆（rustfmt + A1–A7、B5、B6、新测试） | 行为提交为主，fmt hunk 可拆 |
| src/tools/custom_hub.rs | Tools | 混合需按 hunk 拆（rustfmt + B6 新增 abandon_job） | 同上（Tools 产物 D 节列为纯格式化，与 B6 不符，见 §9） |
| src/optional/mod.rs | Tools | 行为改动（B12 + 测试） | 行为提交 |
| src/tools/lsp.rs | Tools | 行为改动（测试加 `#[cfg(windows)]`） | 行为提交 |
| tests/optional_lsp.rs | Tools | 新增测试（过时断言改为编号行） | 随行为提交 |
| src/cli/cli.rs | Tools | 纯格式化 | style 补丁 |
| src/knowledge/{corpus,fsref,index,index_tests,mentions,spec_tests,validate}.rs | Tools | 纯格式化 | style 补丁 |
| src/permission/{mod,sinks}.rs | Tools | 纯格式化 | style 补丁 |
| src/tool/{availability,registry}.rs | Tools | 纯格式化 | style 补丁 |
| src/tools/{ask_user,bash_jobs_contract,knowledge,litecode_workspace,mod,plan,webfetch,websearch}.rs | Tools | 纯格式化 | style 补丁 |
| tests/client_protocol_permission.rs、tests/common/permission.rs、tests/permission_pipeline.rs | Tools | 纯格式化 | style 补丁 |
| src/workspace/service.rs | IDE 基座 | 纯格式化（symlink 禁区 WP-004） | GM 定：随 style 补丁，或 `git checkout --` 还原 |
| src/engines/code_search/embed.rs | IDE 基座 | 冻结只记录（引擎禁区 WP-016） | 不提交，等引擎解冻 |
| tests/wp028_probe_s1_xdev_move.rs、tests/wp028_probe_s2_create_race.rs（未跟踪） | IDE 基座 | 新增测试（`#[ignore]` 探针，ISSUE-038/037） | 随修复包或单独提交 |

## 9. 中层发现的不一致

- tests/manual_compact_e2e.rs 在 git 中是**未跟踪**文件；Runtime 方案写「已在 HEAD(c6b24df)」，不符。提交时要作为新增文件处理。
- src/tools/custom_hub.rs：Tools 产物 D 节列为纯格式化，但 B6 修复新增了 `abandon_job`，应按混合处理。
- tests/stage_a_ctx_consistency.rs 有 1 行改动，各域产物都没列为本域文件，暂归上下文，需确认。
- Session S5、S6 行号因本包后续改动漂移（见 §6）。
- sql death_list 假红：Session 记 WP-011，Runtime 记 WP-012，两包均已关闭（见 §7-7）。
- 本 shell 直接 `cargo test` 报 rustup 无默认工具链，需带 RUSTUP_HOME 才能运行（管家说明为「直接 cargo test」，不完全一致）。
