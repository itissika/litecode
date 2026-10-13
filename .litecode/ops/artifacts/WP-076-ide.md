# WP-079 产物（IDE）：代码语料 30s 静默增量刷新 + 代码/会话计划拆分

父包 WP-077 → WP-076。未 commit；未动 src/serve、windows-*.yml、前端、client_protocol、cli.rs。

## 签名（定稿，见 .tmp/wp079_signature.md）
```rust
// src/engines/mod.rs
impl WorkspaceEngines {
    /// One silent incremental pass for the code corpus. Sync and non-blocking:
    /// it only decides and, when due, spawns the refresh (tokio task if a runtime
    /// is current, else a thread) and returns at once. Never rebuilds, never
    /// enters Warming. Safe to call every tick, during turns.
    pub fn code_index_tick(&self) -> CodeIndexTick;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeIndexTick {
    /// Incremental refresh spawned.
    Started,
    /// Nothing done; reason below.
    Skipped(CodeIndexTickSkip),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeIndexTickSkip {
    NoWorkspace,
    NotDesired,
    NotWarm,
    Busy,          // refresh already running
    WorkerDown,
    Clean,         // no pending code work
    RebuildPending // only first-build / incompatible / manual may rebuild
}
```
```rust
const CODE_INDEX_TICK: std::time::Duration = std::time::Duration::from_secs(30);
{
    let engines = state.workspace_engines.clone(); // WorkspaceEngines: Clone
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(CODE_INDEX_TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let _ = engines.code_index_tick();
        }

调用：非 async，`state.workspace_engines.code_index_tick()`（WorkspaceEngines 为 Clone 句柄），返回值可忽略。router.rs 的 30s 循环（CODE_INDEX_TICK）由 Session 加（中层已批跨域方案 A）。

## 改动
- src/engines/mod.rs:164-195 新增 `CodeIndexTick` / `CodeIndexTickSkip`。
- src/engines/mod.rs:930 `code_index_tick()`：依次判 workspace、desired、Warm、!refresh_busy、`index_work_from_disk`（None→Clean，Rebuild→RebuildPending 永不自动重建，仅 Update 往下）、worker_alive；满足则 `spawn_scoped_refresh(.., RefreshScope::Code, &refresh_busy, true)`，Update 分支只 begin_refreshing、不 begin_building、不置 Warming。不去抖、不看回合。
- src/engines/mod.rs:826 `consume_index_work`（手动 Refresh）：不再用 `engine_index_work` 合并计划；先 `consume_code_index_work`（代码自身计划），再 `consume_session_index_work`（会话计划，从不 begin_building/置 Warming）；返回代码侧 mode，代码无事可做时返回会话侧 mode（前端 mode 枚举不变）。`engine_index_work`/`merge_index_work` 仅留作状态展示。
  - 说明：代码自身 Rebuild 期间（code 置 Warming），会话 consume 的 preflight 返回 InProgress，本次跳过、下个会话 tick 再做——会话不会被拖入重建。
- src/engines/code_search_engine.rs:585-595 flake 修复（测试代码，见下）。

## 测试 ↔ GM 四条验收
| 验收 | 测试 | 结果 |
|---|---|---|
| 1 写文件不调 refresh，限时 pending→0 且可搜 | tests/optional_code_search.rs:535 `code_index_tick_syncs_without_refresh`：hash 嵌入（LITECODE_CODE_SEARCH_USE_HASH=1），reconcile→Warm；写 b.rs + `notify_fs_changes`（同 serve watcher）；每 100ms 直接调 `code_index_tick()`（缩短间隔），20s 内断言 pending_hint=0、搜得到唯一 token、tick 自己返回过 Started、状态仍 Warm；全程不调 refresh/consume | ok |
| 2 会话需重建、代码仅增量，手动 refresh 不置 Warming | mod.rs:1815 `manual_refresh_split_keeps_code_out_of_warming`（并断言合并视图确为 Rebuild，证明旧逻辑会误入）；反向 :1838 `manual_refresh_code_rebuild_does_not_mark_session_building` | ok |
| 3 待办为重建时自动刷新不触发 | mod.rs:1756 `code_index_tick_skips_rebuild_work`（RebuildPending、状态仍 Warm、非 Building、不 busy）；:1775 `code_index_tick_skip_conditions`（NoWorkspace/NotDesired/Clean/NotWarm/Busy/WorkerDown） | ok |
| 4 death_list | tests/code_index_tick_death_list.rs：按标识符边界扫 `code_index_tick` 函数体，禁 begin_building / Warming / request_refresh / consume_index_work / reconcile / engine_index_work，且须含 RebuildPending；扫 `consume_index_work` 体禁 engine_index_work / merge_index_work / begin_building | 2 passed |

验证（HOME=.tmp/home，timeout，flock 共享 target，</dev/null）：
- `cargo test --lib engines::`：276 passed / 0 failed / 1 ignored。/tmp/wp079_lib.log
- `--test code_index_tick_death_list`：2 passed。
- `--test optional_code_search`：18 passed / 1 failed —— `catalog_on_warmup_enables_tool_in_list`（:160 断言 Warming 时工具仍在列表），与 10-12 锁定口径「Warming 时工具隐藏」相反，属 WP-060 后的过时测试，不经过本包改动的函数。新测 ok。
- `--test code_search_ipc`：10 passed / 1 failed —— `refresh_rebuilds_when_dirty_too_large`（:253 期望 DirtyTooLarge→Rebuild），WP-060 已按口径删除 DirtyTooLarge，过时测试；直连 worker client，不经过本包代码。
- 以上 2 条按规则不擅自删改（需 issue），交中层裁定。

## 新 issue
- ISSUE-051（owner tools，domain tools-permission，[常见 bug]）：src/tools/code_search.rs:34-48 `maybe_consume_index` 对 IndexWork::Rebuild 也触发 consume → begin_building + Warming。只记录未改。

## flake：engines::code_search_engine::tests::stop_kills_os_process_when_client_not_installed
- 根因：测试在 `engine.stop()`（SIGKILL）后只调一次 `child.try_wait()` 收尸。SIGKILL 投递后退出是异步的，try_wait 可能在子进程真正退出前执行 → 返回 None、未收尸 → 僵尸进程对 kill(pid,0) 仍为「存活」→ 2s 后判失败。产品代码（kill_client/kill_process）无问题。
- 修法（仅测试，engines 域）：在同样 2s 预算内轮询 `try_wait()` 直到 Some，再做存活检查。
- 验证：单测单独连跑 10/10 绿；`engines::` 整组连跑 10/10 该测绿。

## 补充（12:41 中层裁定：两条旧测试）
- 立 ISSUE-052（owner ide，[常见 bug]，grade drift）：测试未跟口径更新，列 catalog_on_warmup_enables_tool_in_list（tests/optional_code_search.rs:160）与 refresh_rebuilds_when_dirty_too_large（tests/code_search_ipc.rs:253）。
- 基线实测：`git worktree add --detach .tmp/wt_head HEAD`（HEAD=466acad），在共享 target 锁上排队跑（HOME=.tmp/home、timeout、</dev/null）：
  - catalog_on_warmup_enables_tool_in_list：FAILED（panicked :160）。/tmp/wt_base_cat.log
  - refresh_rebuilds_when_dirty_too_large：FAILED（panicked :253，left Incremental / right Rebuild）。/tmp/wt_base_ipc.log
  - 结论：HEAD 上即失败，与 WP-079 改动无关。worktree 已 `git worktree remove --force` + prune。
  - 注意：worktree 与主树同包名共用 target，基线构建覆盖了主树的 lib 产物，之后主树编译测试一度链到旧 lib（找不到 code_index_tick）；`touch src/lib.rs` 后重编恢复。以后做基线建议用独立 CARGO_TARGET_DIR。
- 改法：
  - catalog → 改名 `catalog_hides_tool_while_warming_shows_after_warm`：reconcile 后（start 同步置 Warming）断言 code_search 不在 LLM 工具列表；wait_until_warmed 后断言在列表、vectors.usearch 存在。删除旧的「Warming 前/中也在列表」断言。
  - ipc → 改名 `refresh_stays_incremental_when_all_files_dirty`：两文件全部改动后 refresh 断言 mode=Incremental，且 a_symbol_v2/b_symbol_v2 都能搜到（未删测试）。
- 重跑：optional_code_search 19 passed / 0 failed；code_search_ipc 11 passed / 0 failed；code_index_tick_death_list 2 passed。

## 补充（12:47 打回：manual_refresh_split_keeps_code_out_of_warming 并行偶发失败）
- 现象复现：`engines::` 整组连跑 8 次失败 5 次，均为 mod.rs 断言 `state == Some(Warm)` 实得 `Some(Idle)`。
- 根因（测试时序，非产品竞态）：单测里没有 worker 进程。`consume_index_work` → `consume_code_index_work` 对代码 Update 照常 `spawn_scoped_refresh`；#[test] 无 tokio runtime，走 `std::thread::spawn` 执行 `refresh_scope`，worker 不在 → `notify_worker_failed` → WorkspaceEngines::new 注册的处理器（mod.rs:274-281）把 code_search 置 Idle。测试同步断言 Warm，与该后台线程赛跑；整组并行时 CPU 争用让线程更常抢先，故「单跑过、并行败」。与全局 env / 共享目录 / 端口无关（每测独立 TempDir、独立 WorkspaceEngines）。
- 产品侧判断：真实运行中 Warm 必有 worker；worker 掉线时置 Idle 是既有的恢复设计，不是本包引入的竞态，不改产品代码。
- 修法（仅测试，src/engines/mod.rs）：
  - :1817 `manual_refresh_split_keeps_code_out_of_warming`：以 `accepted.mode == Incremental` 作确定性证据（Rebuild 模式 ⇔ begin_building+Warming）；新增 :1848 `wait_refresh_settled`（≤5s 等 refresh_busy 清零）后再断言 state ∈ {Warm, Idle}（不得 Warming/Failed）且 index view 非 Building。
  - :1858 反向用例：consume 后同样 `wait_refresh_settled`，避免后台线程泄漏到后续断言；并加断言会话 consume 不置代码 busy。
- 格式：`rustfmt --edition 2024 src/engines/mod.rs tests/code_index_tick_death_list.rs`（仅指定文件；--check 确认子模块本已干净，git diff --stat 仅 mod.rs 行数变化），之后 --check 无 diff。
- 验证：`cargo test --lib engines::` 连跑 10 次 10/10 绿（276 passed / 0 failed / 1 ignored，日志 /tmp/w10_{1..10}.log）；code_index_tick_death_list 2 passed。
