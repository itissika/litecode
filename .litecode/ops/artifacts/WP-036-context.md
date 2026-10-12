# WP-036 / WP-039 — 上下文：ISSUE-035 压缩以 Session DB 为唯一真相

- owner: context　approver: backend_mid　parent: WP-036　child: WP-039
- 源码：HEAD `104fd43` + 未提交；**只改** `src/context_pipeline/compact.rs`（+169/−8，含回归单测）
- 工具链：`rustc 1.99.0 (b940084d7 2026-09-28)`；证据命令**无 PATH 前缀**；共享 target 排队，真实 cargo 运行
- 不 commit；未触碰 session / runtime / file_tracker / 前端 / 引擎产品 / symlink / take_changes

## 1. 改动摘要

| 项 | Before | After |
|---|---|---|
| `expected_revision` | persist 时现取 `revision_blocking(..).unwrap_or(0)`（约 :286） | snapshot 时一次捕获 `:191`，`?` fail-closed；mutation 用捕获值 |
| reload Err（persist 已成功） | `*rows = snapshot` + `CompactionStage::Failed` + `return Err` | **不**回滚；warn；`transcript → WorkingRow::pending` + `tail` 应急缓存；`Succeeded`；`Ok(true)` |
| persist Err / LLM / cancel / budget | 回滚 snapshot + Failed | **不变** |
| `expected_prefix` | `Some(prefix_len)` | **保留**（Runtime 所述 sibling safety net） |
| death_list g2 | 要求 `compact_transcript` 在 Compact 后含 `working_set_blocking` | 仍内联调用该 API（cfg(test) 注入包在同 fn 内） |

### file:line（修复后）

- revision 捕获：`compact.rs:188-191`
- Compact mutation：`compact.rs:289-298`（`expected_revision` 变量 + `expected_prefix`）
- reload / 应急缓存：`compact.rs:313-357`
- 测试注入：`compact.rs` `inject_reload_err_after_compact_once`（`#[cfg(test)]`）
- 回归：`compact::tests::compact_persist_ok_reload_err_keeps_db_truth`

## 2. 测试证据（2026-10-12 19:50 HKT）

| 命令 | 结果 |
|---|---|
| `cargo test --lib context_pipeline::compact` | **7 passed**（含新测 `compact_persist_ok_reload_err_keeps_db_truth`） |
| `cargo test --lib context_pipeline` | **48 passed**（原 47 +1） |
| `cargo test --lib death_list` | **13 passed**（g2 绿；未放宽门禁） |
| `cargo test --test stage_a_ctx_consistency pipeline_compact_reloads` | **1 passed**（成功热路径仍 fold reload） |
| `cargo test --test compact_failure_calibers` | **7 passed**（未落库失败仍回滚口径不变） |

新测断言：`Ok(true)`、生命周期 Succeeded 且无 Failed、DB working_set 含 summary、内存 ≠ 压缩前 preview、pending tail 保留、应急行 `log_seq == None`。

## 3. ISSUE-035

- history 已记 WP-039 修复说明；**status 仍为 open**，等 Session 交叉审 + 中层 close 后再关。
- basis 追加 WP-039 / WP-036。

## 4. Session 交叉审请核对

1. **DB authority**：Compact mutation 成功后，`working_set_blocking` / fold 是否仍是唯一权威。
2. **Succeeded-not-Failed**：reload 失败路径只发 `CompactionStage::Succeeded`，不再 Failed。
3. **pending-cache**：应急 `WorkingRow::pending(summary+kept)+tail` 是否可接受（seq 空、短窗口缓存；后续 begin_turn / sync 自愈）。
4. **revision capture**：snapshot 时捕获 + fail-closed 是否与 Session 乐观锁语义一致；`expected_prefix` 仍在。
5. **零 session diff**：本包未改 `src/session/**`、`runtime`、`file_tracker`。

（Session 请在本文件追加交叉审段。）

## 5. 非目标 / 禁区确认

- 未修 ISSUE-032（take_changes）、ISSUE-033（cancel→Error）。
- 未改死亡清单；未删/放宽 failure-calibers。

## 交叉审（Session · WP-039 / ISSUE-035）

- 审阅人：Session
- 时间：2026-10-12 19:55 HKT
- 范围：只读核对 `src/context_pipeline/compact.rs` 与 Session Compact 持久化侧（`src/session/data/{writer,command,mod}.rs`、`sqlite/{ops,session}.rs`、`working.rs`、`context_pipeline/mod.rs` sync）；未改代码。
- 复跑：`compact_persist_ok_reload_err_keeps_db_truth` 1 pass；`death_list` 13 pass（含 g2）；`compact_failure_calibers` 7 pass。HEAD `104fd43`；`git status` 仅 `M src/context_pipeline/compact.rs`。

### 清单核对

1. **DB authority** — **同意**  
   Compact 落库走 `SessionMutation::Compact`（`compact.rs:289-298` → `writer.rs:810-834` → `apply_compact_checkpoint_checked` `session.rs:2874`）。成功后权威仍是 `working_set_blocking` fold（`data/mod.rs:129-140`）。热路径 fold 重载：`compact.rs:334-360`。与契约「落库后从 fold 重载、禁止以内存拼装为权威」一致；应急缓存仅在 reload Err 时启用，不替代 DB。

2. **Succeeded-not-Failed** — **同意**  
   persist 已成功且 reload Err：`compact.rs:336-356` **不** `*rows = snapshot`，发 `CompactionStage::Succeeded`，`return Ok(true)`；无 Failed。persist/LLM/cancel/budget 失败仍回滚+Failed（`:252-310`）不变。回归断言：`compact.rs:892-911`。

3. **pending-cache** — **有条件同意**  
   应急：`transcript → WorkingRow::pending` + `tail`（`compact.rs:342-345`；`WorkingRow::pending` = `log_seq: None`，`working.rs:23-28`）。**可接受**为短窗口降级，且不否定 DB 已提交真相。  
   **条件：** 应急行全部 `log_seq == None`。同回合内若 `log` 游标前进后走 `sync_turn_working`（`context_pipeline/mod.rs:217-232`），会把全部 pending（含 summary+kept）再 `extend` 到 fold 后，可能同回合重复。`end_turn` 清空后下一 `begin_turn` 自愈。非 ISSUE-035 阻断项；建议后续只把原 pending tail 标 pending，或 sync 时排除应急合成行。manual `compact_now` 用局部 `rows`，不写入 pipeline state，无此风险。

4. **revision capture** — **同意（fail-closed）**  
   snapshot 时捕获：`compact.rs:188-191`，`revision_blocking()?`，禁止 `unwrap_or(0)`。写入 mutation 的 `expected_revision`（`:292`）。Session CAS：`writer.rs:463-472`（`actual != expected` → `SessionConflict`）；成功后 `bump_receipt` + `persist_receipt` 更新 `sessions.revision`（`writer.rs:535-555`；`ops.rs:81-84`）。`expected_prefix: Some(prefix_len)` 仍在（`:297`；`apply_compact_checkpoint_checked` `:2884-2895`）。与 Session 乐观锁语义一致。

5. **零 session diff** — **同意**  
   `git status --short`：仅 `M src/context_pipeline/compact.rs`；`src/session/**`、`runtime`、`file_tracker` 无未提交改动。

### 总裁定

**有条件接受**

- 核心主张成立：persist 成功后 reload 失败 → 不回滚压缩前内存、不发 Failed、发 Succeeded + `Ok(true)`；DB 为唯一真相；`expected_revision` fail-closed；未改 Session 源码。
- 条件（不阻断 close ISSUE-035）：§3 应急 pending 与同回合 `sync_turn_working` 的潜在重复；跟进即可。可选：知识节点 `context-compact` summary 区分「提交前失败还原」与「提交后 reload 失败不还原」。
