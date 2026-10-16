# WP-165 — orphan sweep 不阻塞 READY（静态分析与接线审计）

日期：2026-10-15（Asia/Hong_Kong）  
仓库：`/workspace/litecode`，分支 `ops/team-framework`  
状态：代码已改、测试与测量脚本已写；**尚未** `cargo build/test`（等工具集管家/后端中层放行）。未推远端。

## 1. 改动摘要（后端中层批准范围）

| 文件 | 变更 |
|------|------|
| `src/serve/run.rs` | ① sweep 行：同步 `sweep_orphan_lsp_processes()` → `sweep_gate::global().start_background(...)`，位置在 `WorkspaceEngines::new`（建 LspHub）/ `ServeState::new` 之前，没加超时。② **WP-167 漏接，在 WP-165 里补上**：`engine_manager.reconcile(&resolved)` → `engine_manager.defer_reconcile(&resolved)`，按 `artifacts/WP-167/cross-domain/session.diff` 原样接入，没有扩大范围。单独的 diff 见 `wp167-defer_reconcile.diff`。 |
| `tests/startup_order_death_list.rs` | 加强 `serve_run_does_not_sweep_inline`（断言在 ServeState/LspHub 之前启动）；新增 `serve_run_arms_sweep_before_ready_path`。文件头里“该测试会红”的说明已删除，改为写明 WP-165 接入了 session.diff。`serve_run_does_not_warm_optional_engines`（原有测试）现在应能通过，未跑 cargo，尚未验证。 |

git diff 核对：`src/serve/router.rs`（READY 打印）、`/health`、session/new 相关路径都没碰。`src/client_protocol/ws_wp166_tests.rs` 显示修改，那是开工前就有的 WP-166 改动，不在本次 diff 里。

### 验收覆盖
1. **start_background 已接上**：`serve_run_does_not_sweep_inline`。
2. **清理不挡 LITECODE_READY**：`serve_run_arms_sweep_before_ready_path`、`sweep_gate_has_no_timeout`；`start_background` 起线程后立即返回。
3. **残留仍被清，第一个 spawn_ls 等清理结束**：sweep 函数没改；`every_spawn_waits_for_the_sweep_gate`、`lsp::tests::spawn_ls_waits_for_sweep_gate`、`sweep_gate::tests::*`。

## 2. Linux vs Windows `sweep_once` 过滤条件

对照点：Windows `proc_lifetime.rs:192`，Linux `:215` 起。

| 条件 | Windows | Linux |
|------|---------|-------|
| 排除自身 | `pid != self_pid` | `pid == self_pid` → continue |
| 可执行文件在 managed `lsp` 目录下 | `exe_under`（`QueryFullProcessImageNameW` + 去 verbatim + **大小写不敏感** `starts_with`） | `/proc/{pid}/exe` + **大小写敏感** `starts_with(&dir)` |
| 父进程已死 | `!is_process_alive(ppid)`（Toolhelp 记录的 ppid，死父仍保留） | `ppid == 1`（读 `/proc/{pid}/stat`；注释写明「死父 → 被 init/subreaper 收养」） |

**结论：语义 intentionally 平台化，不是简单漏写，但也不是逐条件等价。**

1. **父进程判定不可直接对齐。** Windows 不收养孤儿，死父 PID 仍停在 Toolhelp 里，故可用 `!is_process_alive(ppid)`。Linux 会立刻把孤儿挂到 init（或 user session subreaper），死父 PID 不再可见；`ppid == 1` 是常见可靠信号。若把 Linux 改成 `!is_process_alive(ppid)`，挂在**仍存活的 subreaper** 下的孤儿会漏杀（ppid 活着）。两边都杀不干净 subreaper 场景时，属内核行为差异，不是本次接线 bug。
2. **路径匹配差异合理。** Windows 需要 case-insensitive + strip_verbatim；Linux `/proc/.../exe` 与 `lsp_dir()` 的绝对路径大小写敏感匹配即可。
3. **本次未改 `proc_lifetime.rs`。** WP-167 口径也是「判定与杀进程逻辑一字不改，只挪到后台线程」。若以后要收紧 Linux subreaper 漏网，需另开 WP（例如记录 litecode 自己 spawn 的 LS 集合，而不是扫 ppid）。

## 3. 对照 05ab3c4 / WP-167 接线：漏接点

WP-167 在 05ab3c4 已合入本域：

| 接线 | 位置 | 05ab3c4 状态 | 当前 |
|------|------|--------------|------|
| `SweepGate` + `start_background` / `wait` / 无超时 | `src/lsp/sweep_gate.rs` | 已有 | OK |
| `spawn_ls` 开头 `sweep_gate.wait().await` | `src/lsp/hub.rs:204` | 已有 | OK |
| hub 默认 `global()` + 测试注入 | `hub.rs` | 已有 | OK |
| death list / unit tests | `tests/startup_order_death_list.rs`、`sweep_gate`/`hub` 测例 | 已有 | OK（WP-165 再补顺序断言） |
| **生产调用 `start_background`** | `src/serve/run.rs` | **未合**（跨域 `session.diff`） | **本次已接** |
| **`defer_reconcile` 替代启动期 `reconcile`** | `src/serve/run.rs` | **未合**（同 `session.diff`） | **WP-167 漏接，本次在 WP-165 里补上**（`wp167-defer_reconcile.diff`） |

`artifacts/WP-167/cross-domain/session.diff` 正是这两行；death list 原注释曾写明 `serve_run_*` 在该 diff 落地前会红；两行现已接入，该注释已更新。  
`tools.diff`（optional/webfetch/websearch warm hook）属 Tools 域，不在本次 WP-165 范围；静态看 `defer_reconcile` / `ensure_warm` / `warm_hook` 已在树内，是否与 tools 入口完全对齐留给 Tools 验收。

**漏接结论：** 阻塞 READY 的唯一生产漏接是 `run.rs` 仍同步 sweep；另有同 diff 的 `defer_reconcile` 漏接。两者都已在 WP-165 里补上。`spawn_ls` 的 `wait()` 与门闩实现无漏。

## 4. 验收对照（代码层，未跑 cargo）

- 清理不阻塞 READY：`start_background` 不 await；READY 在 `router.rs:257` 的 listen 路径打印。
- 残留仍会被清：sweep 函数未改，仅换线程。
- 第一个 `spawn_ls` 等清理结束：`hub.rs:204` `wait().await`（已有行为测 `spawn_ls_waits_for_sweep_gate`）。
- 启动顺序测：death list 源码扫描 + 新增顺序断言。
- 耗时对比：**已改为可选**。`measure_ready_vs_lsp.sh` 已写好（有残留/无残留 × 修复前/后）；`timings.jsonl` 现在只是占位（只有 schema 注释，没有数据），放行 cargo 并决定要测时才往里追加。

## 5. 产物路径

```
.litecode/ops/artifacts/WP-165/
  analysis.md              # 本文件
  fix.diff                 # 全部代码 diff（run.rs + 测试）
  wp167-defer_reconcile.diff  # 单独：WP-167 漏接的 defer_reconcile 一行
  measure_ready_vs_lsp.sh  # 有/无残留 × 修复前/后 测量脚本
  timings.jsonl            # 测量结果占位（跑完后追加）
```

## 6. 评审后跟进（Session 已通过 WP-165；只改代码，cargo 未跑，验证另排时段）

diff 见 `followup.diff`（只改 `src/proc_lifetime.rs` 和 `src/lsp/sweep_gate.rs`；READY、/health、session/new 都没碰）。

### 6.1 sweep 耗时日志
- 日志加在 `proc_lifetime::sweep_orphan_lsp_processes` 里。这个函数在 serve 下由 `lsp-orphan-sweep` 线程执行，所以日志就是后台线程打的。用的是仓库现有的 `tracing::info!`，打四类信息：
  - `sweep started`；
  - 每轮杀了进程时打 `pass`（原来就有）；
  - `sweep finished`，带上 `passes`、`killed`（累计）、`elapsed_ms`。
- 函数签名仍是 `fn()`，所以 `start_background(sweep: fn())` 以及 sweep_gate、hub 的测试都不受影响。
- 行为不变：原来的提前 `return` 改成 `break`，以便统一打结束日志，最多仍然 3 轮。没有加超时。`Instant` 只出现在 proc_lifetime.rs 里，`sweep_gate.rs` 中门闩的代码仍不含 timeout/Duration/Instant（death list 用例 `sweep_gate_has_no_timeout` 检查这一点）。
- `passes` 计入最后那一轮「0 个可杀」的确认轮：没有残留时 passes=1、killed=0。

### 6.2 CLI 等非 serve 入口不启用门闩：确认是有意为之
- 依据：WP-167 `plan.md` 第 48 行写明「CLI / 单测从不 arm → wait() 直通，等同今日（今日 CLI 也不 sweep）」。WP-167 之前 sweep 只在 `serve::run` 里调用（`git show 05ab3c4:src/serve/run.rs` 第 75 行）。
- 现状：生产代码里调用 `start_background` 的只有 `src/serve/run.rs:78-79`。`src/cli/cli.rs:763` 只在 `Serve` 子命令下进入 `serve::run`。其余 `WorkspaceEngines::new()` 的调用点（runtime/*_auto_turn.rs、tools/*.rs 等）都在测试里。
- 风险（不是 WP-167 引入的回归，没有改行为，只在注释里写明）：
  1. 非 serve 进程从不清理残留进程，和 WP-167 之前一样；
  2. 以后如果有新的非 serve 入口需要「先清理再 spawn」，必须在它第一次 spawn 前自己启用全局门闩，否则 `wait()` 会直接放行。
- 以上已写进 `sweep_gate.rs` 的模块注释。

### 6.3 `defer_reconcile` 之后启动时 optional 引擎初始状态（原为 Stopped，现为 None）：结论是没有依赖，不需要改
- **实际上不会出现启动时为 Stopped 的情况**：`should_run_engine`（`src/optional/mod.rs:343-345`，05ab3c4 之前在 289-291 行）对 webfetch/websearch 恒为真。所以无论 WP-167 之前还是之后，`reconcile` 都不会走 `stop_engine`（`:313`，写入 Stopped 在 `:321`）。真正会写入 Stopped 的只有关停路径 `stop_all`（`src/serve/router.rs:266`）。启动时为 None 并不是 defer_reconcile 引入的新情况，旧逻辑下也不会在启动时得到 Stopped。
- **后端里读 `EngineWarmupState::Stopped` 的只有一处**：`src/optional/mod.rs:257`，`ensure_warm` 遇到 Stopped 不构建 client。启动时为 None 会正常走构建，这是预期行为。
- **`EngineManager` 的状态没有对外暴露**：`state()`（`:144`）、`engine_status()`（`:160`）、`is_warmed()`（`:132`）在 `src/optional/mod.rs` 之外没有生产调用方（rg 结果只有本模块和它的测试）。`/api/workspace/engines` 返回的是 `WorkspaceEngines::workspace_engine_statuses`（`src/workspace/routes.rs:312` → `src/engines/mod.rs:771`），覆盖 code_search/lsp 等工作区引擎，不包含 webfetch/websearch。`src/engines/status_view.rs` 里的 `state: None` 默认值（`:29`、`:34`、`:58`、`:206`）同样针对工作区引擎，和 optional 无关。
- **工作区引擎里那些依赖 Stopped 的分支和这次改动无关**：`src/engines/mod.rs:291`、`:1093`、`:1110`、`:1173`、`:1229`，`code_search/index_status.rs:521`，`session_search/semantic_index.rs:1557`，读的都是 `EngineState`（工作区引擎），和 `EngineWarmupState` 是两个不同的枚举。
- **前端**：`web/src/api/settings.ts:9-19` 定义的 `EngineWarmupState` 包含 `"stopped"`，`state?: ... | null` 本来就允许 null；`engineStore.ts:111` 消费的是 `/api/workspace/engines` 的 `engines` 字段（工作区引擎）。
- **测试**：`src/optional/mod.rs:426` 断言 `stop_engine` 之后为 Stopped，`:432` 断言重新启用后为 None，这两条走的都是显式停用路径，和启动时的状态无关，不受影响。
