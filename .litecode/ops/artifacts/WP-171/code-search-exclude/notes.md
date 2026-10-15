# WP-171 run3：tests/code_search_ipc.rs `warmup_and_refresh_follow_disk_excludes` 失败

## 结论：测试过时，不是排除逻辑缺陷，产品代码未改
- 失败现象（`evidence/run3/run.out:5272-5273`）：`secret.rs` 已写进 `search_exclude`，warmup 加首建之后仍能在 code_search 结果里搜到它（断言位于 `tests/code_search_ipc.rs:354`）。
- **两边都读了磁盘上的排除配置，没有漏读**：`src/engines/code_search_ipc/server.rs` 中 warmup 在第 93 行和第 108 行、refresh 在第 175 行、reconcile_disk 在第 209 行都调用了 `reload_worker_excludes`（第 25-27 行，即 `reload_workspace_excludes_from_disk`）。
- **根因：代码语义索引按设计不读 `search_exclude`**。
  - `src/workspace/filter/preset.rs:68-76`：`FilterPreset::CodeIndex` 设为 `search_exclude: false`，只认 `files_exclude` 和 `git_ignore`。文档注释在 `preset.rs:28-30`，模块说明在 `src/workspace/filter/mod.rs:6-7`。
  - 首建和调和的扫描都走这个 preset：`src/engines/code_search/build.rs:24-26`（`scan_stat_candidates`）和 `:61`（`scannable_files`）。收录与否另由 `code_index_extensions` 白名单决定（`src/workspace/filter/index_policy.rs:46-58`）。
  - 这个行为是提交 **3ddc643**（2026-10-14，「语义索引改后缀白名单」）有意引入的，提交说明写明「代码语义检索只收 code_index_extensions，不再跟随 search_exclude」。用户文档 `src/config/guides/excludes.md` 也写着「改 search_exclude 不会改变代码语义索引」。3ddc643 没有同步修改这个 IPC 用例，用例仍按旧语义断言。
- **与 WP-167、defer_reconcile、今天其他改动都无关**：code_search worker 不经过 `EngineManager`（webfetch/websearch），也不经过 sweep 门闩或 `serve::run`。用例直接起 worker，走 IPC 调用。相关的过滤代码在今天的提交里都没有改动；工作区里别人未提交的改动也没有涉及 `src/workspace/filter`、`src/engines/code_search*`。

## 修法（只改 tests/code_search_ipc.rs 这一个用例，用例名保留）
按现行约定重写，原意「warmup 和 refresh 都要跟随磁盘上的配置」保持不变：
1. `other.rs` 写进 `search_exclude` 后仍应能搜到，锁住 3ddc643 的约定。
2. 从 `code_index_extensions` 里去掉 `py`：warmup 加首建之后搜不到 `secret.py`。
3. 把 `py` 加回去并持久化，refresh（断言为 Incremental）之后能搜到 `secret.py`。
4. 再去掉 `py`，refresh 之后搜不到 `secret.py`。扩展名不在白名单里时，`scan_stat_candidates` 不再列出该文件，`diff_against_stamps` 会把它标为删除（`reconcile.rs:64-68`）。

diff 见 `fix.diff`。**cargo 未跑**（编译时段在前端手上），需排期验证：`CARGO_BUILD_JOBS=1 cargo test --test code_search_ipc warmup_and_refresh_follow_disk_excludes`。
未验证的风险：第 3 步对 `RefreshMode::Incremental` 的断言沿用了原用例的写法（规模同样是 3 个文件里有 1 个变化）；如果 `decide_index_work` 对这种比例改走 Rebuild，就去掉这条断言，修的是测试而不是产品代码。
