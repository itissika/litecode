# WP-165 / WP-167 / WP-171 IDE — 待 rebase 清单

日期：2026-10-15 13:18 HKT（Asia/Hong_Kong）  
整理人：IDE 基座（执行子代理）  
工作区分支现状：`ops/wp-172-actions-ci`（相对 `origin/ops/wp-172-actions-ci` 有本地未提交改动）。  
**禁止 cargo / 重型**（本清单只做盘点）。**不提交不推送。**

## 板面依据（「等合入」）

摘自 `.litecode/ops/board.json`（2026-10-15 本地）：

| WP | status | owner | 要点 |
|----|--------|-------|------|
| **WP-165** | ready | backend_mid | 启动排位整改总包；本清单主体 |
| **WP-166** | in_progress | session | Session 域；代码/diff 已有；曾卡编译环境；**正重跑编译**。合入前 WP-167 的 `session.diff` 要 rebase |
| **WP-167** | in_progress | ide | IDE 域门闩等已进树；**`run.rs` 跨域接线仍等 WP-166 合入后 rebase**（板面 progress 原文） |
| **WP-171** | done | tools_steward | run7 lcfresh 全绿（GM 关单）；临时 tip 未 push，合入另议 |

交叉引用：
- WP-167 跨域原稿：`.litecode/ops/artifacts/WP-167/cross-domain/session.diff`（与本工作区 `run.rs` 意图一致）
- WP-171 证据：`.litecode/ops/artifacts/WP-171/evidence/run7/`、`init-flow.md`
- WP-166 E0618：`.litecode/ops/artifacts/WP-166/fix_e0618.diff`（工作区 `ws_wp166_tests.rs` 已有同类改动）

---

## 1) 未提交改动总览（与本清单相关的 `src/` / `tests/`）

`git diff --stat`（仅产品/测试；不含 `.litecode/ops` 板面噪音）：

```
src/client_protocol/ws_wp166_tests.rs   |  30 +-   ← WP-166（别人/并行，非本域主动）
src/config/path.rs                      |   1 +    ← WP-171 IDE（cfg windows）
src/lsp/mod.rs                          |  32 +-   ← WP-171 IDE（cfg / 跨平台断言）
src/lsp/sweep_gate.rs                   |  12 +    ← WP-165 followup（注释）
src/optional/mod.rs                     |   8 +-   ← WP-171 IDE（懒建单测）
src/proc_lifetime.rs                    |  16 +-   ← WP-165 followup（sweep 耗时日志）
src/serve/run.rs                        |  11 +-   ← WP-165 / WP-167 漏接
src/session/data/sqlite/mod.rs          |   3 +    ← 他域（Session 死单；勿抢）
src/terminal/shell.rs                   |   7 +    ← WP-171 IDE（cfg windows）
src/tool/registry.rs                    |  10 +-   ← WP-171 IDE（session_search 可见性）
src/workspace/git.rs                    |  34 +-   ← WP-171 IDE（git_cmd 清 GIT_*）
tests/code_search_ipc.rs                |  35 +-   ← WP-171 IDE（search_exclude 过时）
tests/optional_p0.rs                    |  21 +-   ← WP-171 IDE（懒建）
tests/startup_order_death_list.rs       |  35 +-   ← WP-165
?? src/session/data/sqlite/test_support.rs         ← 他域未跟踪文件；勿动
```

未跟踪产物目录：`.litecode/ops/artifacts/WP-165/`、`WP-171/`（及其中子目录）。

---

## 2) 按 WP 分组

### A. WP-165（启动不堵 READY + 接 WP-167 跨域两行 + followup）

| 文件 | 意图 |
|------|------|
| `src/serve/run.rs` | ① 同步 sweep → `sweep_gate::global().start_background(...)`（在 ServeState/LspHub 之前）；② `engine_manager.reconcile` → `defer_reconcile`（WP-167 `session.diff` 原样） |
| `tests/startup_order_death_list.rs` | 启动顺序死单：start_background 已接、不挡 READY、顺序断言 |
| `src/proc_lifetime.rs` | followup：sweep 后台线程开始/结束/轮数/杀掉数/耗时日志（无超时；签名仍 `fn()`） |
| `src/lsp/sweep_gate.rs` | followup：模块注释——CLI/非 serve 不 arm、wait 直通是有意为之 |

**产物路径（WP-165）**

| 产物 | 内容 |
|------|------|
| `.litecode/ops/artifacts/WP-165/fix.diff` | run.rs + death_list（含 start_background + defer_reconcile） |
| `.litecode/ops/artifacts/WP-165/wp167-defer_reconcile.diff` | 单独抽出的 defer_reconcile 一行 |
| `.litecode/ops/artifacts/WP-165/followup.diff` | proc_lifetime 日志 + sweep_gate 注释 |
| `.litecode/ops/artifacts/WP-165/analysis.md` | 静态分析、Linux/Windows 过滤、验收对应、followup §6 |
| `.litecode/ops/artifacts/WP-165/test-run.log` | death_list / sweep_gate 等验证日志（当时已跑过） |
| `.litecode/ops/artifacts/WP-165/measure_ready_vs_lsp.sh` + `timings.jsonl` | 耗时对比**可选**；jsonl 仍是占位 |
| `.litecode/ops/artifacts/WP-165/rebase-checklist.md` | **本文件** |

Session 评审已通过 WP-165；followup 三项（日志 / CLI 注释 / defer→Stopped 调查）已落盘，未再要求 cargo。

### B. WP-167 漏接（跨域 session.diff，落在 WP-165 里补）

- 原稿：`.litecode/ops/artifacts/WP-167/cross-domain/session.diff`
- 工作区已用 WP-165 的 `run.rs` 接上两行；板面仍写「等 WP-166 合入后 rebase」——合入时以 **WP-166 tip + 本工作区 run.rs** 为准重放，勿重复打两遍。
- IDE 本域（`sweep_gate` / `hub.wait` / tools warm）早前已在 05ab3c4 树内；本清单不再复述。

### C. WP-171 IDE 修（测例对齐；产品行为基本不动）

| 文件 | 意图 | 产物 |
|------|------|------|
| `src/lsp/mod.rs` | Windows verbatim/盘符测例：`cfg(windows)` 或跨平台断言 | `WP-171/wp171-fix/fix.diff` + `notes.md` |
| `src/optional/mod.rs` | `reconcile_starts_warmup_for_network_core` → 懒建（ensure_warm） | 同上 |
| `src/config/path.rs` | `strip_makes_verbatim_prefix_compatible` → `cfg(windows)` | `WP-171/runtime-strip-verbatim-cfg-windows.diff` |
| `src/terminal/shell.rs` | Windows 路径 stem 测例拆出 `cfg(windows)` | （同批 IDE；亦见 tools 侧 diff 痕迹） |
| `src/tool/registry.rs` | `session_search` 在 code_search 未 Warm 时应隐藏（WP-057） | `WP-171/tools.diff` |
| `tests/code_search_ipc.rs` | `warmup_and_refresh_follow_disk_excludes` 改跟 `code_index_extensions`（3ddc643 后 search_exclude 不再挡语义索引） | `WP-171/code-search-exclude/fix.diff` + `notes.md` |
| `src/workspace/git.rs` | `git_cmd` + sanitize，修并行 `GIT_DIR` 竞态 | `WP-171/git-and-warmup/fix.diff` + `notes.md` |
| `tests/optional_p0.rs` | `network_core_engines_always_warmup` / `settings_reload_…` 对齐懒建 | 同上 |

WP-171 run7 全绿（GM done）；上述改动是 run2–run6 失败归 IDE 的补丁，合入顺序见 §4。

### D. 工作区里「别人的 / 他域」——rebase 时识别，勿当成 WP-165

| 文件 | 归属提示 |
|------|----------|
| `src/client_protocol/ws_wp166_tests.rs` | **WP-166**：`env` 遮蔽改名（E0618）；见 `WP-166/fix_e0618.diff` |
| `src/session/data/sqlite/mod.rs` + `?? test_support.rs` | **Session 死单并行**；中层已叮嘱别动别人的文件 |
| `.litecode/ops/*`、WP-158/168/172 产物 | ops / 他包；与本 rebase 无关 |

---

## 3) 与 WP-166 可能冲突的热点

| 热点 | 风险 | 我们侧 |
|------|------|--------|
| `src/client_protocol/ws*.rs` / `ws_wp166_tests.rs` | **高**——Session 正改启动 settled / list-then-replay | **未主动改**；工作区已有 WP-166 的 E0618 改名，rebase 时以 Session 为准 |
| `src/session/**` | **高** | **勿碰**（含 sqlite test_support） |
| `src/serve/run.rs` | **中**——WP-166 也可能动 serve 启动路径 | 我们只改 sweep 行 + `defer_reconcile`；合入时手工对齐 WP-167 `session.diff` |
| `src/serve/router.rs`（`LITECODE_READY`） | **中**——WP-166 有 snapshot upkeep / READY 邻近改动 | **刻意未碰** READY 打印时机 |
| `/health`、`session/new` 路径 | **中** | **刻意未碰** |

---

## 4) 建议 rebase / 合入顺序

1. **等 WP-166 合入**（Session 编译重跑通过并入目标分支）。  
2. **先落地 WP-165**  
   - `fix.diff`（`run.rs` start_background + defer_reconcile + death_list）  
   - 再 `followup.diff`（sweep 耗时日志 + sweep_gate 注释）  
   - 若 `run.rs` 已与 `wp167-defer_reconcile.diff` 重叠，以工作区/WP-165 fix 为准，勿双重应用。  
3. **再落地 WP-171 IDE 测例修**（可按产物子目录逐个 apply，冲突面小）：  
   - `wp171-fix/`（lsp mod + optional 内测）  
   - `runtime-strip-verbatim-cfg-windows.diff` + terminal/shell cfg  
   - `tools.diff`（registry session_search）  
   - `code-search-exclude/`  
   - `git-and-warmup/`（git.rs + optional_p0）  
4. **再验证**（排期槽位；本清单不跑）：  
   - `cargo test --test startup_order_death_list`  
   - `cargo test --lib` 过滤：`sweep_gate`、`spawn_ls_waits_for_sweep_gate`、相关 lsp/optional  
   - `cargo test --test optional_p0` 过滤两个懒建测例  
   - `cargo test --lib workspace::git::tests::log_includes_merge_parents -- --exact`  
   - `cargo test --test code_search_ipc warmup_and_refresh_follow_disk_excludes`  
   - 全量留给 WP-171/CI 口径，勿在 IDE 槽抢重型。

---

## 5) 明确「不要碰」

合入 / rebase / 补冲突时 **禁止** 改动：

1. **`LITECODE_READY` 打印时机**（`src/serve/router.rs` 现约 257 行一带）  
2. **`/health` 路由与行为**  
3. **`session/new`（及 WP-166 的 list→replay / startup settled 语义）**

WP-165 只允许动：orphan sweep 后台化、`defer_reconcile`、相关死单/测例，以及 followup 日志与注释。

---

## 6) 一句话摘要（给后端中层）

工作区已攒齐：**WP-165**（`run.rs` 两行 + death_list + sweep 日志/注释）、**WP-167 跨域漏接已在 WP-165 内补上**、**WP-171 IDE 测例修一整串**；**WP-166** 的 `ws_wp166_tests` / session sqlite 是并行他域。等 WP-166 合入后按 **165 → 171 IDE 测例 → 验证**  rebase；全程远离 READY / health / session/new。

---

最新执行状态见同目录 [`rebase-status.md`](./rebase-status.md)（2026-10-15 13:22）。
