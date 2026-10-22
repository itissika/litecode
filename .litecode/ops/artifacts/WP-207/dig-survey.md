# WP-207 dig-survey（只读，基线 df34734）

## 0. 先发现：集成测在本分支编译不过
`cargo test -j 2 --tests --no-run`（free avail 13GiB，91s 后失败，日志 cargo-tests-norun-pre.txt）：
`tests/code_surface_phase5.rs:138` `ToolCallOrigin` 缺 `deadline` 字段（fb5e986 WP-206 A 新增）。`tests/code_surface_phase{1,2b,3,4,5}.rs` 共 14 处 `ToolCallOrigin {` 字面量，均需补 `deadline`（测试侧修，cargo 在第一个失败处停，其余目标未知）。→ 全量集成测红清单要在 G4 修编译后才能拿到。

## 1. 9 条基线红处置（同意 brief）
| 测试 | 判定 | 处置 |
|---|---|---|
| ptc::tests::stack_limit / stack_overflow_stays_sealed_when_the_program_catches_it | **产品缺陷** | 修（见 §2） |
| contract_matrix::phase5_contract_matrix_asserts_live_output | 过时（WP-201 加 data） | 删整文件，独有断言并入模块 |
| phase1_gate ×3（run_code_calls_match_the_text_ledger…、uncaught_tool_error_keeps_the_outer_code、edit_lsp_error_reaches_outcome…） | 过时（ledger 去除/feedback.rs 删除）；uncaught 由 ptc::tests::uncaught_tool_error_keeps_code 覆盖 | 删 |
| deletion_gate::phase0_direct_legacy_deletion_gate | 过期迁移 gate（缺 edit/feedback.rs） | 删整文件 |
| phase3_gate::phase3_apply_edits_partial_reaches_run_code_ledger | 过时（ledger） | 删 |
| litecode_workspace::tests::refresh_reports_invalid_mcp_definitions_outside_the_accepted_diff（litecode_workspace.rs:1646） | 测试过时：fixture 用 `bash` 作冲突 id，Direct bash 已退役，不再是 builtin | 改：冲突 id 换现役 builtin `run`（或 `read`），断言文案同步 |

## 2. 栈溢出可被捕获（根因 + 修法）
- 根因：`src/ptc/engine.rs:273` `runtime.set_max_stack_size(request.limits.stack_bytes)` 与 `arm_stack_watch`（engine.rs:1217-1235，limit=base−stack_bytes，按 STACK_POLL_NEAR/FAR 轮询计数器）用**同一字节数**。QuickJS 原生检查每帧都做、自研 watch 只在轮询点做 → 原生先触发，抛普通 `InternalError: Maximum call stack size exceeded`，JS 可 try/catch 吞掉，且失败种类不是 ResourceLimit/Stack（Faults::STACK 只在 watch 触发时置位，engine.rs:1069）。
- 修法（推荐 A，最小）：原生上限 = `stack_bytes + STACK_NATIVE_HEADROOM`（建议 headroom ≥ 轮询间隔最大帧深 × 帧大小，取 max(64KiB, stack_bytes/2)），保证 watch 先触发并封口（不可捕获）；原生只作后备安全网。注意 `effective_stack_bytes` 的 1<<24 上限对加 headroom 后的值同样适用；工作线程栈须 ≥ 原生上限（核 ptc 线程 stack size）。
- 补强（B，可同包）：原生 InternalError 若仍发生（headroom 不足），在 interrupt handler/异常出口检测 "Maximum call stack size exceeded" 来自引擎（非用户 throw，tests.rs:1176-1177 已有伪造用例）时置 Faults::STACK → 归类 Stack 并封口。仅靠消息文本不安全，故以 A 为主。
- 测试：两条转绿；保留 tests.rs:1176 "用户 throw 同名消息不算 Stack" 用例。

## 3. 盘点（lib）
| 文件 | 测数 | 判定 |
|---|---|---|
| src/tool/deletion_gate.rs | 1 | 过期迁移 gate → 删整文件（+mod.rs 声明） |
| src/tool/contract_matrix.rs | 1（1063 行） | 过时 → 删整文件；先核独有断言（data 形状）并入各工具模块 |
| src/tool/phase1_gate.rs | 6 | 3 过时删；write_outside_workspace_reports_path_outside_workspace / unknown_parameter_is_invalid_input / read_image_is_unsupported_without_parts 有等价（write.rs outside 用例待核、executor.rs:1647 unknown_parameters_warn…、read.rs:826 test_read_image_is_unsupported）→ 无等价的并入后删整文件 |
| src/tools/phase2_gate.rs | 17 | 全绿但与 read/grep/glob 模块测大量重复（grep.rs:1010/1061/1111-1148、read.rs:730/784/852、glob.rs）→ 逐条比对，独有（grep message 五态、chain glob→read/grep→read/read→edit）并入 grep.rs/read.rs/edit tests，删文件 |
| src/tools/phase3_gate.rs | 9 | 1 过时删；write expected_hash 4 条、edit/apply_edits 3 条并入 write.rs / edit/tests.rs / apply_edits.rs（apply_edits.rs:297 serial_skip、planner.rs:102 已有部分等价）→ 删文件 |
| src/types/death_list_gate.rs | 13 | 源码扫描架构守卫，ms 级、仍有目标（provider catalog / seq 语汇）→ **保留**（见 Q2） |
| 描述/PRELUDE 子串断言 | ~66：ptc_contract_harness 23、code_manifest.rs 22、edit/tests 5、code.rs 5、registry.rs 5、apply_edits.rs 3、subagent/launch.rs 2、knowledge.rs 1 | 被 run_code_sdk.txt 快照覆盖的（code 面工具描述+PRELUDE：harness/code_manifest/edit/code/apply_edits ≈58）删；registry/subagent/knowledge 不在 SDK 快照 → 保留（Q3） |

## 4. 盘点（tests/，65 文件 ~451 条）
| 文件 | 测数 | 判定 |
|---|---|---|
| code_surface_phase1/2b/3/4/5 | 14/6/11/4/3=38 | 补 deadline 修编译；有效语义（审批、取消封口、锁在审批后取、并发写一致）保留并合并成 tests/code_surface.rs；删已过时：phase2b direct_code_tools_are_refused…、phase3 direct_edit_and_write_are_refused…、historical_file_path_is_rejected…、partial_program_keeps_one_ledger_line…（ledger）、phase1 run_code_description_is_a_pure_function…（快照覆盖）、phase4/5 重复的 cancel_before_approval_does_not_write_later 留一条 |
| ptc_contract_harness.rs | 3 | bundle_interface_is_generated_from_tool_contracts（快照重复）删；embedded_runtime_runs_read_glob_grep、literal_true_searches… 与 ptc/grep 单测重复 → 删整文件（Q4） |
| *_death_list.rs ×6 | 22 | 源码扫描守卫；逐个核目标：code_index_tick / session_data_sql / startup_order / terminal_bypass / workspace_path 仍有对应模块 → 保留；workspace_hotswitch（已删功能的符号禁令，防回归）→ Q2 |
| 其余 *_contract.rs（subagent_child_turn、terminal_pty、workspace_process） | 14 | 现役行为测 → 保留 |

## 5. 删除计数（预估，实施时以 deletions.md 为准）
- 整文件删：deletion_gate、contract_matrix、phase1_gate、phase2_gate、phase3_gate、ptc_contract_harness → 37 测（其中并入新测约 15 条）
- code_surface 过时 ~6、描述子串断言 ~58 处（断言级，不一定整测）
- 修：stack 2（产品）、MCP 1、集成测编译（14 处 deadline）

## 6. 风险
- 集成测从未在本分支跑过，修编译后可能暴露更多红 → 先跑、入 artifacts、再决定删/修（新增红按同规则判定，产品缺陷单独报）。
- stack headroom 需核线程栈大小，否则原生检查失效会真栈溢出崩进程。

## 7. Q（带推荐）
- Q1 集成测修编译（补 deadline）算"测试侧修"，放 G4 第一步先做？推荐是（否则无法拿删前基线）。
- Q2 death_list 源码扫描守卫（lib death_list_gate 13 + tests/ 6 文件 22）：推荐保留（ms 级、防回归，非迁移 gate）；仅 workspace_hotswitch 可删（功能早已删）。备选：全删。
- Q3 registry/subagent/knowledge 的描述子串断言不在 SDK 快照：推荐保留（否则无文本守卫）；备选扩快照覆盖后删。
- Q4 ptc_contract_harness 整删？推荐删（3 条均与快照/ptc/grep 单测重复）。
- Q5 修编译后集成测若出现新红：推荐过时→按同规则删并入 deletions.md；产品缺陷→停下报 gm，不在本包修。
- Q6 coverage-map 形式：brief 已定 artifacts/WP-207/coverage-map.md；推荐按"流程 → 模块::测试（数量）"表格，删前/删后两列。
