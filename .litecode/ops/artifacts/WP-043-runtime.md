# WP-048 / WP-043-runtime（Runtime：覆盖核对 + 裁剪提案）

- 父包：WP-043；本包：WP-048；owner：runtime；approver：backend_mid
- 状态：已执行并 `deliver`（方案曾批 → start → Step1 绿表+缺口 → Step2 提案-only → deliver）
- 分支：`ops/team-framework`；基线 HEAD `0c75d0a`；WT 仍含 WP-025 等未提交行为改动（本包**未再改**测试/代码）
- 工具链：`rustc 1.99.0` / `cargo 1.99.0`；`/workspace/litecode` **直接** `cargo test`（**无 PATH 前缀**）
- 禁区遵守：未碰引擎产品 / symlink / take_changes·ISSUE-032 / ISSUE-034；**未删测、未补测、未 commit**
- ISSUE-041：已登记；owner=**session**（中层裁定）；本包只登记不修
- 实跑：2026-10-12 20:17–20:19 HKT；日志 `.tmp/wp048/`

---

## 0. 硬串行

1. **Step 1 覆盖** → 绿表 + 缺口（只列；补测须中层点头）✅
2. **Step 2 裁剪** → 删/留/放宽/抽 helper **提案**交中层；**未过审不落地** ✅（本包无落地删）

---

## 1. Step 1 — 覆盖绿表（C1–C8）

### 1.1 流程 → 锚点 → 实测

| # | 核心流程 | 承诺口径 | 锚点测试 | 实测 |
|---|---|---|---|---|
| C1 | Agent loop 主链 | 文本停 / 工具步进 / max_steps / 取消三态 / 不完整 FC 不执行 / persist 失败回滚 | `tests/agent_loop.rs`：`single_text_response_stops_loop`、`function_call_then_text_grows_transcript`、`tool_step_persists_function_call_before_output`、`max_steps_with_perpetual_tools`、`cancelled_breaks_loop`、`cancelled_after_model_persists_output`、`cancelled_after_stream_skips_tool_execution`、`incomplete_function_call_is_not_executed`、`persist_failure_rolls_back_working_set`（+ common permission×2） | **绿** 13/0（`.tmp/wp048/int1.log`） |
| C2 | Auto compact 失败 | `TurnOutcome::Error(CompactionFailed)`，不再 `call_model`；transcript/pipeline 快照还原 | `agent_loop::auto_compact_fail_stops_turn_without_model_call`；`compact_failure_calibers::{compact_failure_restores_transcript_snapshot,auto_compact_failure_restores_pipeline_working_set}` | **绿**（含于 int1：agent_loop + calibers 7/0） |
| C3 | Manual compact 失败→可重试 | lease 释放；可再 start；失败后重试可完成 | `calibers::manual_compact_failure_releases_lease_for_retry`；`manual_compact_e2e::{manual_compact_failure_e2e_releases_lease_and_retry_starts,manual_compact_retry_after_failure_completes}` | **绿** calibers+e2e 4/0（e2e 计 common×2） |
| C4 | 取消→Cancelled（ISSUE-033） | compact 中 `Canceled` → `TurnOutcome::Cancelled`，不 `call_model` | `agent_loop::cancel_during_auto_compact_is_cancelled_without_model_call`；实现 `src/agent/core.rs:45-48` | **绿** |
| C5 | 断线重连后停回合 | reconnect 后 auto compact 失败停回合；终端 LLM 错误停回合 | `calibers::{reconnect_then_auto_compact_failure_stops_turn,reconnect_then_terminal_llm_error_stops_turn}` | **绿** |
| C6 | 回合错误码 compaction_failed | auto compact 失败 → `ErrorCode::CompactionFailed`，`retryable=false`（非 internal） | `project::wp025_turn_error_tests::auto_compact_failure_maps_to_compaction_failed_not_retryable`；`client_protocol_project::turn_finished_auto_compact_failure_is_compaction_failed`；映射 `project.rs:543-554`（仍靠 Display 文案，ISSUE-036） | **绿** 1+1（`.tmp/wp048/c6_{int,lib}.log`） |
| C7 | Pending / idle auto-turn | 空队不备；busy 不抢；idle flush 落 user 行/图片序/mentions；revert 后空；bash/custom/subagent mailbox | `pending_auto_turn` 9；`bash_auto_turn` 4；`custom_auto_turn` 3；`subagent_auto_turn` 3 | **绿** `cargo test --lib auto_turn` → **19/0**（`.tmp/wp048/c7.log`） |
| C8 | WP-036 / ISSUE-035 交界 | Compact **落库成功**后 reload 失败：DB 为真相；Runtime 不得把该路径当「压缩失败→Error」 | 产品测在 context：`compact_persist_ok_reload_err_keeps_db_truth`（`compact.rs:806`，返回 `Ok(true)`+emergency）。Runtime：`core.rs:43-49` 仅 `Err`→`Error`，`Ok(true)` 继续；`fake_deps` 失败注入只 `Err(CompactionFailed)`（真失败）。读码确认 **不混淆** | **绿（读码+他域测）**；本域无「Ok(true)≠Error」专测 → 见缺口 G2 |

### 1.2 实跑命令汇总

| # | 命令 | passed | failed | exit | 日志 |
|---|---|---|---|---|---|
| 1 | `cargo test --test agent_loop --test manual_compact_e2e --test compact_failure_calibers -- --test-threads=1` | 13+7+4 | 0 | 0 | `.tmp/wp048/int1.log` |
| 2 | `cargo test --test client_protocol_project turn_finished_auto_compact_failure_is_compaction_failed -- --test-threads=1` | 1 | 0 | 0 | `.tmp/wp048/c6_int.log` |
| 3 | `cargo test --lib client_protocol::project::wp025_turn_error_tests -- --test-threads=1` | 1 | 0 | 0 | `.tmp/wp048/c6_lib.log` |
| 4 | `cargo test --lib auto_turn -- --test-threads=1` | 19 | 0 | 0 | `.tmp/wp048/c7.log` |

说明：`agent_loop`/`calibers`/`manual_compact_e2e` 套件内各含 `common::permission`×2，计入 passed 总数。**本域核心具名测均过；无 flake。**

### 1.3 WP-033 / WP-036 刚落核对

| 落地项 | 归属 | 本包结论 |
|---|---|---|
| ISSUE-033 取消→Cancelled | Runtime（WP-025 closed） | C4 绿 |
| compaction_failed 文案映射 | Runtime（WP-025）；ISSUE-036 open | C6 绿；类型化**不在本包修** |
| ISSUE-035 Compact DB 真相 | 上下文 | C8 读码：Runtime `Ok(true)`≠Error；失败测仍对准真 `Err` |
| ISSUE-032 take_changes | 禁区 | 不测不修 |
| ISSUE-041 sql death_list 假红 | session（中层定） | 只登记；不挡本域绿表 |

---

## 2. 缺口清单（只列，本包不补测）

| # | 缺口 | 现状 | 建议 |
|---|---|---|---|
| G1 / G5-wire | 重连/错误终止后经 **JSON-RPC 协议入口** 再 `start_turn` / compact 全链路 e2e | Session F5 仅 manager 层 `g4_g5_…turn_restarts_after_error`；WP-021/024 已转 Runtime「后续」；本域 calibers 停在 agent-loop 层 | **另立 WP / 中层点头后再补**；本包不擅自开测 |
| G2 | ISSUE-035 × Runtime：无断言「persist ok + reload err → `Ok(true)` ≠ `TurnOutcome::Error`」 | 契约由 context 测 + `core.rs` `Result<bool>` 分支保证；calibers 只注入 `Err` | **软缺口**；若补测需中层点头（跨域 fixture） |
| G3 | ISSUE-036：compaction_failed 靠 `CompactionFailed.to_string()` 全等 | C6 覆盖仍绿；脆 | 类型化另包；本包不修 |
| G4 | ISSUE-041：`session_data_sql_death_list` 假红 | 已建 ISSUE；owner=session；全量集成观感红 | 本包不修；不挡 C1–C8 |

**无阻断缺口。** 显式句：除上表 G1–G4 外，C1–C8 承诺口径均有有效锚点且本轮实测全绿。

---

## 3. Step 2 — 裁剪提案（只提案，待中层裁决；本包未删）

### 3.1 重核 WP-021-backend §5 Runtime 旧清单

| 旧项 | 现状重核（2026-10-12） | 提案 | 中层裁决 |
|---|---|---|---|
| bash/subagent/custom_auto_turn 同构抽 helper | 仍三文件并列；`idle_without_subscribers_*` / `busy_session_leaves_*` / `idle_with_subscribers_*` 结构同构；pending 9 条更丰富未算入旧「三份」；本轮 19/19 绿 | **留行为断言；可选抽公共 helper**（重构测，非删） | _待裁_ |
| session_data_sql_death_list × death_list_gate SQL 子串单点化 | 两套仍在；假红 → ISSUE-041（session） | **单点化推迟到 ISSUE-041 处置后**；本包不删门禁 | _待裁_ |
| death_list_gate::session_seq_g2_* 放宽 | 属 ISSUE-013 / context | **本域不动**；移交 context | n/a（非本域） |
| calibers×e2e lease 重叠 | Session WP-047 T5 与 Runtime 一致：**留两者** | **保持保留**；不再提案删 | 建议维持既裁 |

### 3.2 本域提案表

| ID | 测试/位置 | 类型 | 提案 | 理由 | 中层裁决 |
|---|---|---|---|---|---|
| T1 | `*_auto_turn` 三文件同构 | 重复结构 | **抽 helper，留**各自差异 | 同 3.1 | _待裁_ |
| T2 | `manual_compact_failure_releases_lease_for_retry` vs e2e | 部分重叠 | **留** calibers | 快回归；与 Session T5 / WP-022 一致 | _待裁（建议留）_ |
| T3 | `session_data_sql_death_list` 假红 | 门禁 drift | **不删**；跟 ISSUE-041 | 假红≠冗余测 | _待裁_ |
| T4 | agent_loop 取消三态 × compact 取消 | 边界 | 默认 **留**；表驱动合并可选 | 取消点不同（model/stream/compact） | _待裁_ |
| T5 | `client_protocol_project` 非 Runtime 投影条 | 越域 | 列表交 Session，Runtime **不裁** | 所有权 | n/a |

输出列约定：`ID | 动作 | 中层裁决 | 落地证据` — **本包落地证据全空**（未过审不删）。

---

## 4. 交叉审

### 4.1 Runtime → Session（WP-047 / WP-043-session）

- 同意 F3：`compaction_failed` / `retryable=false` 与本域 C6 一致；`turn_finished_auto_compact_failure_is_compaction_failed` 本轮复绿。
- 同意 F5 分层：Runtime calibers reconnect_* 绿；**G5-wire 仍开**（本包缺口 G1，与 Session G-wire 同条）。
- 同意 T5：**留** calibers lease 快回归 + e2e 主路径。
- 无新增阻断；不要求 Session 改产物。

### 4.2 Runtime → 上下文（WP-049 / WP-043-context）

- 同意 C1 / ISSUE-035：`compact_persist_ok_reload_err_keeps_db_truth` 为产品真相；Runtime `Ok(true)` 不进 `TurnOutcome::Error`（C8 读码）。
- 本域缺口 G2 =「无 Runtime 专测 Ok(true)≠Error」— 与 Context 产品测互补；**非阻断**，补测须中层点头。
- 同意 G1 pending-cache 软缺口归 Context/Session，不扩 Runtime。
- 无分歧需改 ISSUE history。

### 4.3 请 Session / 上下文审我方（可选回填）

1. C1–C8 锚点与你侧 F*/C* 是否仍对齐。  
2. 缺口 G1（G5-wire）归属「Runtime 另立 WP」是否仍可接受。  
3. 裁剪 T1/T2 提案是否与你侧冲突。

---

## 5. done_when / 风险

| 项 | 状态 |
|---|---|
| Step 1 绿表每行有实测 | ✅ |
| 缺口清单完整（含无阻断显式句） | ✅ |
| Step 2 提案交中层；未擅自删 | ✅ |
| 交叉审节（我方对 Peer） | ✅；Peer 回填可选 |
| 禁区未碰；无 commit | ✅ |
| `deliver` → backend_mid | 本步 |

**风险：** 共享 target 锁（本轮未遇长锁）；WT 上 WP-025 未 commit 与 HEAD 混跑（行为测依赖 WT）；ISSUE-041 干扰全量集成观感。

---

## 6. 执行记录

1. ~~fill+submit~~ → backend_mid approve（含 ISSUE-041 owner=session；直接 cargo；缺口只列）  
2. `start`（已 in_progress）  
3. Step 1 实跑 C1–C8 → 写绿表+缺口  
4. Step 2 定稿提案（不落地）  
5. 交叉审自写 Peer 节 → **`deliver`**
