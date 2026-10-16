# tools.diff 说明（WP-167，Tools 审后第 2 版，待签字）
形状（Tools 定）：
- (a) webfetch `fetch` / websearch `search` 入口各调一次 `warm_hook` → `EngineManager::ensure_warm`；首次同步建 client，之后复用；启动（`defer_reconcile` 只写 endpoint）、注册、回合开始均不预建。
- (b) 设置变更走 `reconcile` → `start_warmup`：configure / stop 照旧；**变更前 has_client 为真才立即重建**（丢旧 client 后同步/blocking pool 重建），为假只更新配置不建 client。
- (c) `ensure_warm` 幂等、并发安全：快路径见 client 即返回；构建在**按 tool_id 的锁**（`warm_locks`，选做已做）下，拿锁后再查 client。失败记 Failed+error 并返回 Err，**不缓存**，下次调用重试。`Stopped` 返回 Ok 不建 → 工具见 client 为空，仍是原 "… engine not warmed" 文案。
- 预热失败把真实错误返回调用方：`if let Err(e) = warm() { return Err(format!("{tool} engine failed to start: {e}")); }`（webfetch / websearch）。
- websearch 不再每次开线程：覆写 `call_async`，`spawn_blocking` 包住同步调用，`search` 里直接调 warm()。说明两点供审：① webfetch 现有代码覆写的是 `execute`（不是 call_async），websearch 按指示覆写 `call_async`（默认 `execute` 会调它），效果等同；② 包的是 `tool.call(input)` 而非 `call_inner`，以保留默认路径的 `finalize_signals`（webfetch 用 call_inner，原本就无此步）。
- webfetch `execute()` 原先重建工具会丢 hook，已改为带上。
- tools/lsp.rs `ensure_ready` 先 `ensure_lsp_probed().await`。
测试：
- optional/mod.rs：`defer_reconcile_builds_no_client`、`ensure_warm_builds_once_and_reuses`、`concurrent_first_calls_build_once`（8 线程 barrier，builds==1）、`failed_build_is_not_cached_and_retries`（无 endpoint 失败→配置后重试成功，builds==2）、`stopped_engine_is_not_rebuilt`（Ok 且不建）、`settings_reconcile_rebuilds_immediately`（builds==2，websearch client 仍 None、endpoint 已配置）；既有 `reconcile_starts_warmup_for_network_core` 不改。
- tools/websearch.rs：`failed_warmup_returns_the_real_error`（内容含 `websearch engine failed to start: ` 与 `not configured`）、`call_async_runs_on_the_blocking_pool`。
- `builds` 计数字段仅 `#[cfg(test)]`。

## 第 3 版（Tools 复审：Stopped 后重新启用的回归）
- 修法：`start_warmup` 在 `if !self.has_client(tool_id) { return; }` 之前先 remove 该工具的 state 与 last_error。
- 核实前提成立：`start_warmup` 是私有函数，唯一调用点是 `reconcile` 中 `should_run_engine(..) == true` 分支；停用走 `stop_engine`（`reconcile` 的 else 分支与 `stop_all`）。另注：当前 `should_run_engine` 对 webfetch/websearch 恒为真，所以实际进入 Stopped 的只有 `stop_all`（关停）路径。
- 新测试 `reenable_after_stop_builds_on_next_call`：stop_engine(webfetch) → reconcile → ensure_warm，断言 client 为 Some、状态为 Warm；同时断言 websearch 先前的 Failed 及其 last_error 在 reconcile 后被清掉。
