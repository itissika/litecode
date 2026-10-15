# WP-165 / WP-167 rebase 状态（2026-10-15 13:22 HKT）

执行方：IDE 基座。**本回合未跑 cargo**（禁止擅自占槽）。未提交、未推送。  
对照清单：`.litecode/ops/artifacts/WP-165/rebase-checklist.md`。

## 0. 基线

| 项 | 值 |
|----|-----|
| 工作区分支 | `ops/wp-172-actions-ci`（ahead 1 vs origin） |
| tip | `0554d72` — `fix(WP-172): fold WP-171 product test fixes for green ci-init-env` |
| WP-166 | board **done**（后端中层验收）；tip 已含 `ws_wp166_tests` E0618 等 |
| 对比基 | `origin/ops/team-framework`（仍为同步 sweep + reconcile 的旧 `run.rs`） |

说明：tools_steward 已把本域先前未提交的 WP-165/167 接线 + followup + WP-171 IDE 测例修 **fold 进 `0554d72`**。本回合工作区 `src/`/`tests/` **相对 tip 无额外未提交产品 diff**（仅有 `.litecode/ops` 板面/产物与他包噪音）。

## 1. 做了什么（本回合）

### WP-167 整理
1. **F6 / routes.rs**：核查 `.litecode/ops/artifacts/WP-167/ide.diff`——**已不含** `src/workspace/routes.rs` hunk（全文无 `routes.rs` / `mark_startup_settled`）。按 `review_wp166.md` F6，**无需再裁剪**；写明于 `WP-167/ide.F6-scrubbed.md`。
2. **刷新 `cross-domain/session.diff`**：用 `git diff origin/ops/team-framework...HEAD -- src/serve/run.rs` 重生成（旧稿备份为 `session.diff.pre-rebase.bak`）。内容 = `start_background` + `defer_reconcile`，行号对齐当前 tip。
3. **接线落地核对**（均已在 tip，**本回合未再改代码**）：
   - `sweep_gate` / `hub.rs` `wait()` / `activate_deferred` / path_index 延迟 legacy 删除 → 早在 05ab3c4 系
   - `serve::run` `start_background` + `defer_reconcile` → tip `0554d72`
   - Tools `tools.diff` 侧 warm_hook / ensure_warm → 树内已有（非本回合）

### WP-165
1. tip 已含清单要求全部项：
   - `src/serve/run.rs`：`start_background` + `defer_reconcile`
   - `tests/startup_order_death_list.rs`：顺序死单加强
   - followup：`proc_lifetime` 耗时日志；`sweep_gate` CLI/非 serve 不 arm 注释
2. **刷新产物 diff**（相对 `origin/ops/team-framework`）：
   - `WP-165/fix.diff`（run + proc_lifetime + sweep_gate + death_list）
   - `WP-165/followup.diff`
   - `WP-165/wp167-defer_reconcile.diff`（从 run.rs diff 抽出含 defer 的 hunk）
3. **确认未碰**：`src/serve/router.rs`、`/health`、`session/new` 相关路径在 tip 相对 team-framework **无本域主动改动**（`0554d72` 未改 router/routes）。

### WP-171 IDE 测例
- **本回合未动**（按中层「另排」）。tip 里已 fold 的测例修保持原样，未再叠补丁。

### 他域
- **未动** `ws_wp166_tests.rs`、`session/data/sqlite/*`（已在 tip；无新改）。

## 2. 剔除了什么

| 项 | 结果 |
|----|------|
| ide.diff 中 F6（`routes.rs`） | 产物侧**本就没有**；记录于 `ide.F6-scrubbed.md`，未改 ide.diff 字节 |
| WP-171 测例大包 | 本回合**不处理**（另排） |
| READY / health / session/new | **未改** |

## 3. 当前 diff 摘要

工作区相对 tip：产品代码 **干净**。  
相对 `origin/ops/team-framework`（待合入到该线时用）关键 WP-165/167 文件：

```
src/serve/run.rs                 | start_background + defer_reconcile
src/proc_lifetime.rs             | sweep started/finished 日志（passes/killed/elapsed_ms）
src/lsp/sweep_gate.rs            | 非 serve 不 arm 注释
tests/startup_order_death_list.rs| 顺序断言 + arms_before_ready
```

详见已刷新的：
- `.litecode/ops/artifacts/WP-165/fix.diff`（138 行）
- `.litecode/ops/artifacts/WP-165/followup.diff`（56 行）
- `.litecode/ops/artifacts/WP-167/cross-domain/session.diff`（29 行）

## 4. 申请占槽（等后端中层放行后再跑）

**不要全量。** `CARGO_BUILD_JOBS=1`，建议 exact：

```bash
source .litecode/ops/tools/cargo_env.sh
export CARGO_BUILD_JOBS=1

# WP-165 / WP-167 接线死单
cargo test --test startup_order_death_list

# 门闩与 spawn 等待
cargo test --lib -- sweep_gate spawn_ls_waits_for_sweep_gate

# 懒建契约（与 defer_reconcile / WP-167 一致；optional 内测）
cargo test --lib optional::tests::reconcile_starts_warmup_for_network_core -- --exact
```

可选（若槽宽裕，仍非全量）：
```bash
cargo test --test optional_p0 network_core_engines_always_warmup -- --exact
cargo test --test optional_p0 settings_reload_reconciles_engine_manager -- --exact
```

通过标准：上述全部 ok；且 diff 中无 router READY / health / session/new 变更。

## 5. 结论 / 下一步

| 问题 | 答案 |
|------|------|
| WP-167 接线是否已在树上？ | **是**（tip `0554d72`） |
| WP-165 是否还需再改代码？ | **否**（本回合只刷新产物与状态） |
| F6 routes 是否已剔除？ | **产物已干净**；有书面确认 |
| 还缺什么？ | **编译验证占槽**（上节列表）；向 `ops/team-framework` 合入时用刷新后的 `session.diff` / `fix.diff` |
| WP-171 测例？ | 另排；本回合不动 |

**请后端中层**：确认 tip 合入策略（已在 `ops/wp-172-actions-ci` vs 需 cherry-pick/merge 回 `ops/team-framework`），并放行上节 exact 测例占槽。

---

## 7. Exact 验证（2026-10-15 13:24，后端中层放行占槽）

**全绿。** 日志：`exact-verify.log`（WP-167 与 WP-165 各一份）。  
详见 `.litecode/ops/artifacts/WP-167/deliver-notes.md`。

| 测例组 | 结果 |
|--------|------|
| startup_order_death_list | 9/9 ok |
| sweep_gate + spawn_ls_waits_for_sweep_gate | 5/5 ok |
| reconcile_starts_warmup_for_network_core | 1/1 ok |
| optional_p0 两例（可选） | 2/2 ok |

**可 deliver WP-167 → review**（基线 tip `0554d72`，不 push team-framework）。
