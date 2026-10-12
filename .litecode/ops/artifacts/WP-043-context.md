# WP-043 / WP-049 — 上下文：核心覆盖核对 + 测试裁剪提案

- owner: context　approver: backend_mid　parent: WP-043　WP: WP-049
- 基线：`git rev-parse --short HEAD` = **`0c75d0a`**（stabilize batch WP-033… compact DB truth 已入）
- 本域未提交源码：**无**（`src/context_pipeline`、`src/reminder`、相关 tests / death_list_gate 相对 HEAD 干净；仅 `.litecode/ops/*` 面板/产物脏）
- 工具链：rustc **1.99.0**；`cargo test` 直跑（无 PATH 前缀）；共享 target 未遇长锁
- 禁区遵守：未碰引擎产品 / symlink / take_changes；**未删除、未放宽、未 `#[ignore]` 任何测试**；不 commit
- 实跑时区：2026-10-12 20:15–20:16 HKT；日志 `.tmp/wp049/*.log`

---

## 1. 覆盖（步骤 A）

### 1.1 流程 → 测试映射（C1–C8）

| # | 流程 | 主要测试（file:line / 名） | 实跑 |
|---|---|---|---|
| C1 | compact DB 真相 / ISSUE-035（WP-039） | `compact.rs:806` `compact_persist_ok_reload_err_keeps_db_truth`；实现 `:313-367` 不回滚+Succeeded；`inject_reload_err_after_compact_once` `:519`；persist 前失败仍回滚路径由 timeout/cancel + `compact_failure_calibers`（他域）覆盖 | **1/1 具名绿**；含于 lib 48 |
| C2 | fold 重载（禁 summary+kept 拼装权威） | `death_list_gate.rs:522` `session_seq_g2_*`；`stage_a_ctx_consistency.rs:206` `pipeline_compact_reloads_working_set_from_fold`；`working_set_identity.rs:90` `keep_recent_compact_reader_matches_fold_checkpoint_then_kept` | death_list 13 绿；stage_a 含该测；working_set 5 绿 |
| C3 | compact LLM 统一入口 | `death_list_gate.rs:589` `compact_llm_path_*`；`compact.rs` CountingProvider + `compact_request_returns_summary_on_provider_success`（断言一次 `complete_with_stream_events`） | 含于 death_list 13 + context_pipeline 48 |
| C4 | reminder sources（WP-026 R3） | `sources/tests.rs` 9：tasks / background / files_changed / bash_exit / subagent / custom / plan_changed / step_budget / env（各含正向+抑制） | **9/9**；含于 reminder:: 34 |
| C5 | engine / mentions / kinds 链路 | engine `:157-313`（7，含 restore 组合）；mentions 8；migrate 6；file_tracker 2；knowledge_status 1 | reminder:: **34** 绿 |
| C6 | budget / keep_recent / media | budget 6（含 baseline_adds / baseline_falls_back）；keep_recent 14；media_budget 5 | 含于 context_pipeline 48 |
| C7 | 装配 env/system/summary/mod | env 2；system 9；summary 3；mod pad/commit 2 | 含于 48 |
| C8 | 跨域行为门 | `stage_a_ctx_consistency` **27**；`working_set_identity` **5**；`turn_prefix_stability` **12**（含 `restore_row_lands_after_compacted_and_prefix_stays`、`state_reminders_only_append_across_steps`） | 全绿 |

WP-017 阶段 A 必过名（10）现况：均仍存在（`keep_recent_skip_*` 现为 `keep_recent_skip_still_enforces_hard_limit_when_over_budget` / `keep_recent_skip_under_hard_limit_returns_ok_without_compact`）。全文件现 27 测（WP-017 时只跑 10 exact）——本次跑全文件，均绿。

### 1.2 绿表（实跑）

| # | 命令 | passed | failed | exit | 日志 |
|---|---|---|---|---|---|
| 1 | `cargo test --lib context_pipeline` | **48** | 0 | 0 | `.tmp/wp049/context_pipeline.log` |
| 2 | `cargo test --lib reminder::` | **34** | 0 | 0 | `.tmp/wp049/reminder_all.log` |
| 3 | `cargo test --lib reminder::sources::` | **9** | 0 | 0 | `.tmp/wp049/reminder_sources.log` |
| 4 | `cargo test --lib death_list` | **13** | 0 | 0 | `.tmp/wp049/death_list.log` |
| 5 | `cargo test --lib …compact_persist_ok_reload_err_keeps_db_truth -- --exact` | **1** | 0 | 0 | `.tmp/wp049/issue035.log` |
| 6 | `cargo test --test stage_a_ctx_consistency` | **27** | 0 | 0 | `.tmp/wp049/stage_a.log` |
| 7 | `cargo test --test working_set_identity` | **5** | 0 | 0 | `.tmp/wp049/working_set.log` |
| 8 | `cargo test --test turn_prefix_stability` | **12** | 0 | 0 | `.tmp/wp049/turn_prefix.log` |

相对 WP-017：`context_pipeline` 47→**48**（+ISSUE-035 回归）；WP-017 的 `reminder` filter 36 vs 本次 `reminder::` **34**（过滤前缀差异，sources 9 已计入 34；非失败）。WP-017「sources 0 专属」缺口 **已关闭**（WP-026）。

### 1.3 缺口

| # | 缺口 | 严重度 | 建议 |
|---|---|---|---|
| G1 | 应急 pending 同回合 `sync_turn_working` 可能重复（Session 交叉审条件接受，WP-036-context）——**无专用回归测** | 软 / 非阻断 | 另包或跟进；本包不扩 scope |
| G2 | mentions 去重键含原始 path（WP-026 #6 suspect）——无「路径规范化后去重」测 | 已知不修 | 记录；不开新测除非中层立包 |
| G3 | budget「同长度回退后误用旧前缀」suspect（WP-026 #7）——已有 `baseline_falls_back_when_recorded_prefix_no_longer_exists` / `baseline_adds_*`，**等长误用**边界仍无精确用例 | 软 | 可选补测；非阻断 |
| G4 | ISSUE-032 take_changes 丢通知 | 跨域 open（runtime） | **禁区**；不测不改 |
| G5 | expected_revision fail-closed 无独立单测（逻辑在 ISSUE-035 路径+实现注释） | 软 | 可并入 C1 跟进；非必须 |

**无阻断缺口。** 核心 C1–C8 均有有效覆盖且全绿。

---

## 2. 裁剪提案（步骤 B · 只列不删）

重核依据：WP-021-backend §5 上下文（6）+ WP-022-context (d)；对照 HEAD `0c75d0a` 现行代码（含 `sources/tests.rs`、`compact_persist_ok_reload_err_keeps_db_truth`）。

| # | 测试 | 类别 | 理由（现状） | 建议 | 替代覆盖 |
|---|---|---|---|---|---|
| P1 | `reminder/engine.rs:191` `bash_exit_names_a_user_kill` | 重复 | 仅断言 Kill 文案；`sources/tests.rs:109` `bash_exit_distinguishes_kill_and_lists_running` 覆盖 kill/exit/running 更全 | **删** | sources bash_exit |
| P2 | `engine.rs:169` `custom_tool_settled_names_outcomes` | 重复 | 与 `sources/tests.rs:175` `custom_tool_settled_lists_entries_and_skips_empty_detail` 同路径 | **删** | sources custom |
| P3 | `engine.rs:253` `plan_changed_dedupes_on_revision` | 重复 | 与 `sources/tests.rs:195` `plan_changed_diff_against_seen_and_spine` 重叠 | **删** | sources plan_changed |
| P4 | `engine.rs:282` `step_budget_once_per_turn_inside_the_window` | 重复 | 与 `sources/tests.rs:229` `step_budget_warns_within_three_once_per_turn` 重叠 | **删** | sources step_budget |
| P5 | `engine.rs:216` `restore_fires_once_after_compaction` | 留（可收窄） | 经 `sync`+`SpineReminderView.kinds_after_compacted` 覆盖组合链路；sources `tasks_restore_*` 只测 source 函数 | **留**；可选去掉与 sources 重复的文案细节（若有） | 自身 + sources tasks |
| P6 | `budget.rs:167` / `:174` `token_count_prefers_provider_*` + `…_falls_back_*` | 可合并 | 二分支同一 API | **放宽/合并**为表驱动一条 | 合并后仍覆盖两分支 |
| P7 | `keep_recent.rs:257` / `:263` `entire_transcript_within_keep_returns_none` + `empty_returns_none` | 可合并 | 同一 `None` 出口 | **放宽/合并** | 合并后仍覆盖 |
| P8 | `death_list_gate.rs` 全 13（含 g2 / compact_llm） | **不动** | 死名单；任何改动须先开 issue | **留** | — |
| P9 | stage_a / working_set_identity / turn_prefix_stability | 留 | 跨域行为门；无与 lib 的有害重复 | **留** | — |
| P10 | `compact.rs:806` `compact_persist_ok_reload_err_keeps_db_truth` | 留 | ISSUE-035 回归；WP-039 新测 | **留** | — |
| P11 | `reminder/sources/tests.rs` 全 9 | 留 | R3 专属；替代 P1–P4 的 source 层 | **留** | — |
| P12 | `engine.rs:157` `first_seam_writes_env_only` / `:313` `serde_roundtrip_*` | 留 | 无 sources 等价组合/serde | **留** | — |

**提案计数：** 删 **4**（P1–P4）· 合并/放宽 **2**（P6–P7）· 留 **6** 类（P5/P8–P12 等）· death-list 改动提案 **0**（明确不动）。

**未过 backend_mid 裁决前：零删除、零放宽落地。**

---

## 3. 交叉审请 Session 核对（父包约定）

请 Session 审本产物：
1. C1 绿表与 ISSUE-035 口径（DB 真相、Succeeded、不回滚）是否与 Session 侧 Compact mutation / 协议投影一致。
2. G1 软缺口是否仍接受为非阻断。
3. 裁剪 P1–P4 删除是否影响 Session 可见的 reminder 契约（预期否：仅测重复）。

（Session 段由对方追加；本交付不代写。）

---

## 4. 结论

- 覆盖：本域核心流程 **全绿**；ISSUE-035 回归 **绿**；WP-017 sources 缺口已关闭。
- 裁剪：中层已裁决；P1–P4/P6/P7 已落地（见 §5）；P5/P8–P12 保留。
- 阻断：无。

---

## 交叉审（Session · WP-049 覆盖）

- 审阅人：Session
- 时间：2026-10-12 20:19 HKT
- 范围：只读审本产物 **§1 覆盖绿表 + §1.3 ISSUE-035 相关缺口（G1/G5）**；**不审** §2 裁剪提案（中层明示不必审裁剪）。未改代码；未动 `WP-043-session.md`。
- 对照：WP-039 Session 交叉审（`WP-036-context.md`）；实现 `src/context_pipeline/compact.rs`；Session Compact CAS `session/data/writer.rs`；协议投影 `client_protocol/project.rs`；应急 pending 与 `sync_turn_working`（`context_pipeline/mod.rs`）。
- 日志抽查：`.tmp/wp049/issue035.log` — `compact_persist_ok_reload_err_keeps_db_truth` **1 passed**（EXIT=0）。

### 核对要点

1. **DB 真相口径（C1 / ISSUE-035）** — **同意**  
   落库 `SessionMutation::Compact`（`compact.rs:289-298` → `writer.rs:810-832`）。persist 成功后 reload Err：不 `*rows = snapshot`，装应急缓存，发 `CompactionStage::Succeeded`，`Ok(true)`（`compact.rs:313-356`）。权威仍是 DB fold；应急不否定已提交真相。与 WP-039 Session 侧裁定一致。绿表 C1 行号与现行代码吻合（测 `:806`；实现 `:313-367`；inject `:519`）。

2. **Succeeded-not-Failed ↔ Session 协议投影** — **同意 · 无矛盾**  
   热路径只在 `stage == Failed` 时附 `StructuredError`（`project.rs:366-374`）；`Succeeded` 时 `error: null`，`session/compact_lifecycle` 不会把「已落库的 compact」投影成失败码。persist/LLM/cancel/budget 失败路径仍 Failed + 回滚（`compact.rs:252-310`）不变。客户端不会因 reload 失败误走 `compaction_failed` 生命周期失败分支。

3. **expected_revision fail-closed（G5）** — **同意（软缺口可接受）**  
   snapshot 时 `revision_blocking()?`（`compact.rs:188-191`），禁止 `unwrap_or(0)`；写入 mutation `expected_revision`（`:292`）。Session CAS：`writer.rs:464-471`（`actual != expected` → `SessionConflict`）。无独立单测属软缺口，逻辑已在 C1 路径+注释覆盖；**非阻断**，可跟进。

4. **应急 pending（G1）** — **有条件同意 · 仍非阻断**  
   应急行全 `WorkingRow::pending`（`log_seq: None`，`compact.rs:342-345`）。同回合若 log 游标前进后走 `sync_turn_working`（`mod.rs:217-232`），会把全部 pending（含 summary+kept）再 `extend` 到 fold，可能同回合重复；`end_turn` 清空后自愈（`:241-246`）。与 WP-039 条件相同；**无专用回归测**已如实列为 G1。建议另包跟进，本包不扩 scope。**仍接受为非阻断。**

5. **零 Session 协议矛盾** — **同意**  
   Compact 持久化契约（乐观锁 + fold 重载权威）与 lifecycle 投影口径一致；覆盖节未引入与 Session `session/compact` / `session/compact_lifecycle` 冲突的新语义。

### 总裁定

**有条件同意**

- C1 绿表与 ISSUE-035（DB 真相、Succeeded、不回滚）与 Session Compact mutation / 协议投影 **一致**。
- G5 fail-closed 软缺口 **接受**。
- **条件（不阻断）**：G1 应急 pending × 同回合 `sync_turn_working` 潜在重复 — 维持 WP-039 条件接受，跟进即可。
- 裁剪提案本审 **未覆盖**（按中层指示跳过）。

---

## 5. 裁剪落地（中层裁决 → 已执行 · 2026-10-12 20:21 HKT）

### 5.1 裁决（backend_mid return）

| 项 | 裁决 | 落地 |
|---|---|---|
| P1–P4 | **DELETE** engine 四条重复测；sources/tests.rs 为权威 | 已删 |
| P5 | **KEEP**（文案收窄非必须） | 未改 |
| P6 | **MERGE** budget 两条 token_count_* → 表驱动一条 | 已合并 |
| P7 | **MERGE** keep_recent 两条 None → 一条 | 已合并 |
| P8–P12 | **KEEP** | 未碰 |

### 5.2 变更明细

**删除（`src/reminder/engine.rs`）**
- `custom_tool_settled_names_outcomes`
- `bash_exit_names_a_user_kill`
- `plan_changed_dedupes_on_revision`
- `step_budget_once_per_turn_inside_the_window`

**保留（engine）**：`first_seam_writes_env_only`、`restore_fires_once_after_compaction`、`serde_roundtrip_keeps_kind_and_text`

**合并**
- `budget.rs`：`token_count_prefers_provider_when_set` + `token_count_falls_back_to_estimate_without_provider` → `token_count_prefers_provider_or_falls_back_to_estimate`
- `keep_recent.rs`：`entire_transcript_within_keep_returns_none` + `empty_returns_none` → `within_keep_or_empty_returns_none`

未改：death_list、stage_a / working_set / turn_prefix、ISSUE-035 回归、sources×9。

### 5.3 复跑证据（直跑 cargo，无 PATH 前缀）

| 命令 | 落地前（§1.2） | 落地后 | exit | 日志 |
|---|---|---|---|---|
| `cargo test --lib reminder::` | 34 | **30**（−4） | 0 | `.tmp/wp049/reminder_prune.log` |
| `cargo test --lib context_pipeline::budget` | 6（含于 48） | **5**（−1） | 0 | `.tmp/wp049/budget_prune.log` |
| `cargo test --lib context_pipeline::keep_recent` | 14（含于 48） | **13**（−1） | 0 | `.tmp/wp049/keep_prune.log` |
| `cargo test --lib context_pipeline` | 48 | **46**（−2） | 0 | `.tmp/wp049/ctx_prune.log` |

基线 HEAD 仍为 `0c75d0a`；本域仅上述 3 文件未提交测试编辑（`git diff --stat`：engine −107 / budget +− / keep_recent +−）。不 commit。

### 5.4 结论（更新）

- 覆盖：仍成立（前次全绿；裁剪只动重复/合并单测）。
- 裁剪：P1–P4/P6/P7 **已落地**；P5/P8–P12 保留。
- 阻断：无。待 Session 交叉审段（若尚未写入）与中层 acceptance。
