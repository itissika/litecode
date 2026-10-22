# WP-206 A组（Runtime #1+#2）实现草稿 — 未编译、未测（调度令：IDE 占槽，禁 cargo）

## 结论（待 cargo 验证）
- #1 `run` 前台窗：`BACKGROUND_EXIT_WINDOW`(500ms) 删除 → `pub const RUN_FOREGROUND_WINDOW = 30s`（agent_jobs.rs）；`PipeDetach::Background { window }` 携带窗口；run 取 `min(30s, run_code 剩余 − 250ms 余量)`。
  - 剩余预算来源：`ptc::CallInfo.remaining`（engine 在 call begin 取 `clock.remaining()`）→ code.rs `invoke_binding` 算 deadline → `ToolCallOrigin.deadline` → executor 放入 `RunScope`。
- #2 锁只持到 spawn：新模块 `src/tool/run_scope.rs`（tokio task_local `RunScope{deadline, spawn_locks}`；`SpawnLocks` = Arc<Mutex<Vec<Box<dyn Any+Send>>>>，`release()` 幂等，最后一个 clone Drop 即释放）。
  - `Tool::releases_lock_after_spawn()`（默认 false，run=true）；executor 对这类工具把 `EffectGuard` + `WriteLockGuard` 移入 spawn_locks，自身持一个 clone 至 run_tool 返回 → 取消/超时/panic unwind 兜底释放。
  - `PipeRequest.on_spawned: Option<oneshot::Sender<()>>`：`cmd.spawn()` 成功即 send；cancel-before-spawn/spawn 失败时 Sender 被 drop → run 侧 `spawned_rx` 以 Err 解析，同样 `release()`。
  - 选 task_local 而非给 `ToolExecutionContext` 加字段：避免改 45 处构造点（plan "或等价"）。
- run.md：补"约 30s 内结束直接返回 exit_code，超出转后台用 wait；并行 run 真并发、可能触及同一文件，需顺序用 await 串"。**会影响 SDK fixture sha，按令未刷**。

## 文件
src/tool/run_scope.rs(新)、src/tool/mod.rs、src/tool/trait_.rs、src/tool/executor.rs、src/terminal/agent_jobs.rs、src/terminal/mod.rs、src/tools/run.rs、src/tools/descriptions/run.md、src/ptc/mod.rs、src/ptc/engine.rs、src/tools/code.rs、ToolCallOrigin 构造点补 `deadline: None`（tool/pipeline.rs、tool/contract_matrix.rs、tool/phase1_gate.rs、tools/baseline/mod.rs）、测试：src/tools/run_lock_gate.rs(新)+tools/mod.rs 注册；wait.rs/kill.rs/run.rs 测试改 `RunTool::with_window(...,500ms)`（cfg(test) 钩子）保持快速 detach。

## 新测（run_lock_gate.rs，7 条）
short_command_returns_exit_code_in_foreground（sleep 2 → exit_code 0）；window_is_capped_by_run_code_deadline；locks_release_after_spawn_while_command_still_runs；spawn_failure_releases_locks（cwd 不存在）；cancel_before_spawn_releases_when_the_call_ends；dropped_in_flight_future_releases_and_panic_unwind_releases；four_parallel_write_runs_finish_in_about_one_second（run_code Promise.all 4×`sleep 1 && touch f$i`，断言 0.95s ≤ t < 3.5s、f1..f4 存在、exit_code 全 0）。

## 待跑（放行后，free≥2Gi，CARGO_BUILD_JOBS=2）
1. `cargo test --lib -j 2 --no-run 2>&1 | tee .litecode/ops/artifacts/WP-206/cargo-A-build.txt`
2. `cargo test --lib -j 2 tools::run_lock_gate 2>&1 | tee .../cargo-A-runlock.txt`
3. `cargo test --lib -j 2 tools::run tools::wait tools::kill terminal::agent_jobs 2>&1 | tee .../cargo-A-jobs.txt`（libtest 只收一个 filter：分四次跑或用 `-- run wait kill agent_jobs` 不行，需逐个）
4. `cargo test --lib -j 2 tool::executor ptc:: tools::code tool::contract_matrix tool::phase1_gate 2>&1 | tee .../cargo-A-exec.txt`（同上逐个）
5. `cargo clippy --lib --tests -j 2 2>&1 | tee .../cargo-A-clippy.txt`
预期：sdk_snapshot 因 run.md 变动会红，等统一刷 fixture。

## 风险
- 未编译：可能有小的类型/借用错（如 `EffectGuard` 需 `Send+'static`，应满足）。
- 并行测共享进程级 resource gate，其他测持 Workspace 时可能拉长耗时。
- WriteLockGuard 的 `release_all(session)` 会释放同 session 全部 key（既有行为，同 session 本就不互斥）。

## 作用域证据（task_local 不跨 task 丢失）
调用链（nested，run_code 内）：
1. `src/ptc/engine.rs:808` `handle.spawn(async move { host.call(...) })` —— 新 task T1 在此创建，但 **RunScope 尚未安装**（在其下游才装），无丢失问题。`remaining` 在 spawn 前于 JS 线程读出（engine.rs `let remaining = clock.remaining()`），以值进 `CallInfo`。
2. T1 内：`src/tools/code.rs` HostFn 闭包 → `invoke_binding`(:655) → `run_nested`(:114/:713) → `execute_tool_call`(executor.rs:464) → `run_tool`(:429)：全 `.await` 链，无 tokio::spawn/spawn_blocking（executor.rs 里 spawn_blocking 仅出现在 :1224 测试）。
3. `src/tool/executor.rs:702` `run_scope::scope(run_scope, catch_unwind(tool.execute(..)))` —— 安装点；该 future 在 T1 内被 select/await 轮询（:714/:717/:749）。
4. `src/tools/run.rs:117` `run_scope::current()` 在 scope 内同步读取并 **clone 出 Arc**（之后不再依赖 task_local）。
5. `src/tools/run.rs:119` `supervise_pipe` → `agent_jobs.rs:853` `tokio::spawn` 出 supervisor task T2；T2 只持 `on_spawned` Sender，`agent_jobs.rs:900` 在 `cmd.spawn()` 成功后 `send(())`。T2 不读 task_local。
6. `src/tools/run.rs:138-139` 在 T1（run future 内）`spawned_rx.await` 后 `scope.spawn_locks.release()` —— 释放用的是第 4 步 clone 的 Arc，与 task 无关。
7. 兜底：executor 持 `_spawn_locks` clone 到 run_tool 返回；取消/超时/panic 时 guards 随之 Drop。
结论：无跨 task 丢失路径，无需修改。端到端测 `four_parallel_write_runs_finish_in_about_one_second` 即覆盖该链（若 scope 丢失，`current()` 得 Default，真锁不早释 → 串行 ~4s → 测红）。

## cargo 结果（CARGO_BUILD_JOBS=2，-j 2）
- cargo-A-build.txt：编译通过（仅既有 warning）
- cargo-A-runlock.txt：run_lock_gate 7/7 ✅（4×1s Promise.all 并行在 2.01s 的整组内完成）
- cargo-A-jobs.txt：tools::run 7、tools::wait 4、tools::kill 4、terminal::agent_jobs 15，全绿
- cargo-A-exec.txt：tool::executor 19 ✅、tools::code 47 ✅；红：ptc::tests::stack_limit / stack_overflow_stays_sealed…（2）、contract_matrix phase5（1）、phase1_gate 3 条、sdk_snapshot（run.md 变动，预期，待统一刷 fixture）
- cargo-A-baseline-noA.txt：stash 掉 A 改动后同样 6 条红（stack×2、contract_matrix×1、phase1_gate×3）→ **基线既有失败，非 A 引入**
