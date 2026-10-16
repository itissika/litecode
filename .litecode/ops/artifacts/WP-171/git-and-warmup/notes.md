# WP-171 run6：git merge parents + optional_p0 webfetch warmup（IDE 域）

日期：2026-10-15 12:42–12:45 HKT。轻量改；只跑了这 3 个测例，没跑全量。未提交未推送。

## 结论一览

| 测例 | 判定 | 改动 |
|------|------|------|
| `workspace::git::tests::log_includes_merge_parents` | 测例缺陷（并行竞态） | 测例辅助：raw git 也清 `GIT_*` |
| `network_core_engines_always_warmup` | 测例过时（契约已是懒建） | 测例改断言 |
| `settings_reload_reconciles_engine_manager` | 测例过时（同上） | 测例改断言 |

产品代码（`EngineManager::reconcile` / `start_warmup` / `defer_reconcile`）**未改**。

## 1) `log_includes_merge_parents`（`src/workspace/git.rs`）

- **现象**（run6）：`assert!(ok, {args:?})` 在 `["add", "a.txt"]` 失败。run2/run3 曾绿 → 环境相关/竞态。
- **根因**：该测例与 `init_repo` 用裸 `StdCommand`，**没有**走生产路径的 `sanitize_git_env`。同文件测例 `status_ignores_poisoned_git_dir`（约 989–1002 行）会在进程环境里短暂 `set_var("GIT_DIR", bare_snap)`。`--test-threads` 默认并行时，`git add` 可能继承别人的 `GIT_DIR`，指向 bare snapshot，于是失败。不是「找不到 git」「默认分支不是 main」（`init_repo` 已 `-c init.defaultBranch=main` + `symbolic-ref HEAD refs/heads/main`）。
- **修法**：新增测试辅助 `git_cmd`，对 fixture 命令同样 `sanitize_git_env`；`init_repo` 与 merge 测例的 `run` 都改用它；失败时打印 stderr。

## 2–3) optional_p0 webfetch「always warm」

- **契约（当前产品）**：WP-167 Tools 签字（`artifacts/WP-167/cross-domain/tools.md` (b)）+ 今早 `wp171-fix` 已把 `optional::tests::reconcile_starts_warmup_for_network_core` 改成懒建断言：
  - `reconcile` / 设置重载触发的 `reconcile`：**不**预建 HTTP client；
  - 首次工具调用 `ensure_warm` 才建；
  - `stop_all` → `Stopped`。
  - 实现：`src/optional/mod.rs` `start_warmup` 约 190–205 行（无 client 则 return）；`defer_reconcile` 约 309 行（serve 启动路径）。
- **与 WP-167/defer_reconcile 的关系**：这两个测例直接打 `EngineManager::reconcile` / `RuntimeHandle::apply`→`reconcile`（`src/runtime/mod.rs:184-185`，`DocId::ALL` 含 `Engines` 故会 reconcile，见 `src/config/gate/mod.rs:80-88`）。失败是因为仍断言旧的 always-warm，**不是**产品回退；**不要**用回退 `defer_reconcile` 来「修」。
- **修法（测例对齐契约，未 skip）**：
  - `network_core_engines_always_warmup`：reconcile 后 **未** warm → `ensure_warm` → warm → `stop_all` → Stopped。用例名保留并加注释说明 WP-167。
  - `settings_reload_reconciles_engine_manager`：`apply(ALL)` 后 **未** warm → `ensure_warm` → warm（证明 reload 仍走 reconcile，且懒建可用）。

## 验证

```
source .litecode/ops/tools/cargo_env.sh
CARGO_BUILD_JOBS=1 cargo test --lib workspace::git::tests::log_includes_merge_parents -- --exact   # ok
CARGO_BUILD_JOBS=1 cargo test --test optional_p0 network_core_engines_always_warmup -- --exact   # ok
CARGO_BUILD_JOBS=1 cargo test --test optional_p0 settings_reload_reconciles_engine_manager -- --exact  # ok
```

首次因 rustup 自动装 toolchain 抖动失败（见 `test-run.log`）；稳定后重跑见 `test-run2.log`，三例全绿。MemAvailable ≈ 6.2–6.5GiB。

## 产物

- `fix.diff` — 仅 `src/workspace/git.rs` + `tests/optional_p0.rs`
- `notes.md` — 本说明
- `test-run.log` / `test-run2.log` — 验证日志
