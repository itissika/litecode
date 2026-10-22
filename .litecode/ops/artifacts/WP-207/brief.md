# WP-207 brief — 测试瘦身 + ptc 栈溢出分类修复

- 类型：implement；owner：backend_mid；approver：gm
- 基线：`feat/tool-contract-ptc` @ `c5cd966`；**不合 main**
- 来源：GM 对 9 条基线红的根因分析 + 全套抽样（Andy 判定：测试膨胀属实）

## 背景
- 单元测 ~2242（tools 407 / engines 370 / session 228 / llm 192 / workspace 170 / tool 128 / config 128 / lsp 118 / runtime 78 / client_protocol 66 / context_pipeline 61 / ptc 57 / terminal 35）；集成测 `tests/` 65 文件 ~453 条——**WP-206 只跑了 `--lib`，集成测在本分支未跑过**。
- 9 条基线红中 7 条为过时测试（断言已被 WP-197/200/201 改掉的契约：run_code 无 data / ledger 行 / edit feedback.rs / Direct bash），全部位于迁移期 gate 文件；2 条 ptc stack 为真 bug。
- 迁移 gate 文件：`src/tool/deletion_gate.rs`(462 行，自述"Phase 0 records red")、`src/tool/contract_matrix.rs`(1063 行)、`src/tool/phase1_gate.rs`、`src/tools/phase2_gate.rs`、`src/tools/phase3_gate.rs`、`src/types/death_list_gate.rs`；`tests/` 下 `code_surface_phase1..5`、6 个 `*_death_list.rs`、`ptc_contract_harness` 及若干 `*_contract.rs`。
- 另有 ~69 处对 description/PRELUDE 文本的子串断言，与 SDK fixture 快照重复。
- `resource_gate.rs`、`run_lock_gate.rs` 是真实锁语义测，**不属于**迁移 gate。

## 9 条基线红处置
| 测试 | 结论 | 处置 |
|---|---|---|
| ptc::tests::stack_limit | 真 bug：QuickJS 原生 max stack（engine.rs:273）与自研 stack watch（engine.rs:1217）同为 64KB，原生先触发→可捕获 Exception | 修产品 |
| ptc::tests::stack_overflow_stays_sealed_when_the_program_catches_it | 同上 | 修产品 |
| tool::contract_matrix::phase5_contract_matrix_asserts_live_output | 过时（WP-201 b3e82f7 加 data） | 删（独有断言并入对应模块） |
| phase1_gate::run_code_calls_match_the_text_ledger_and_logs_keep_level | 过时（WP-200/201 去 ledger、加 data） | 删 |
| phase1_gate::uncaught_tool_error_keeps_the_outer_code | 过时+冗余（ptc::tests::uncaught_tool_error_keeps_code + render 测覆盖） | 删 |
| phase1_gate::edit_lsp_error_reaches_outcome_and_run_code_data | 过时（25a15e1 删 edit/feedback.rs） | 删 |
| deletion_gate::phase0_direct_legacy_deletion_gate | 过期迁移 gate | 删整文件 |
| phase3_gate::phase3_apply_edits_partial_reaches_run_code_ledger | 过时（WP-200 去 ledger） | 删 |
| litecode_workspace::…refresh_reports_invalid_mcp_definitions_outside_the_accepted_diff | 过时（Direct bash 已退役） | 改：用现役 builtin 名（如 `read`/`run`）测冲突 |

## Scope
1. **先全量跑**：`cargo test`（lib + `tests/`）在本分支列出全部红，入 artifacts。
2. **删除（彻底）**：`deletion_gate.rs`、`phase1_gate.rs`、`contract_matrix.rs` 与 `phase3_gate.rs` 的过时部分、目标已消失的 death_list 套件、`code_surface_phase*` 中与单元测重复/断言已变更契约的部分。Andy 要求：**无意义/低价值测试直接删，不"以防万一"保留**；不 `#[ignore]` 代替删除。
3. **并入后再删**：phase gate 中若有独有且仍有效的断言，先并入对应工具模块测试（read/grep/glob/edit/write/run/wait/kill/code/render），再删 gate。
4. **改**：MCP 冲突测改用现役 builtin 名。
5. **收拢描述文本断言**：对 description/PRELUDE 的子串断言，凡已被 SDK fixture 快照覆盖的删除；快照是唯一文本守卫。
6. **stack overflow 产品修复（in scope）**：保持契约"栈溢出不可捕获、`ResourceLimit`/`Stack`"。做法二选一或等价：QuickJS 原生 max stack 设为大于 watch 阈值（确保 watch 先触发），或将原生 "Maximum call stack size exceeded" 归类为 Stack 资源限额并封口不可捕获。两条 stack 测试转绿；加/保留测试覆盖"try/catch 不能吞栈溢出"。

## 验收标准
1. **核心路径覆盖地图（必交）**：`artifacts/WP-207/coverage-map.md`，列核心流程 → 覆盖测试（模块+测试名/数量）：run_code 管线（ptc/code/render/run_lock）、工具契约 edit/apply_edits/read/grep/glob/write/run/wait/kill、LSP、session 持久化、provider 流式（llm）、SDK fixture 快照。删前删后对照，核心流程覆盖不得下降。
2. **删除清单**：`artifacts/WP-207/deletions.md`，逐条（文件/测试名）写理由（过时/冗余/过期 gate/文本重复）及替代覆盖测试（如有）。
3. stack 两测绿；栈溢出在 try/catch 下仍不可捕获（测试）。
4. **全量 `cargo test`（lib + tests/ 集成测）全绿**；集成测必须实际运行，日志入 artifacts；删前/删后测试数对比。
5. 不改产品代码，除 stack 修复外。
6. push 实验分支；**不合 main**。

## 约束
- 分支 `feat/tool-contract-ptc`；不合 main。
- 跑 cargo 前确认空闲内存 ≥ 2GiB；建议 `-j 2`。
- 如有描述文本变动须同步 SDK fixture sha + 快照测（本包预期不改描述）。
- 不碰 `desktop/build/icon.ico`（工作区既有删除，非本包）；不改 web/。
