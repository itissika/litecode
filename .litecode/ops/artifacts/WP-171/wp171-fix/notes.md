# WP-171 run2 失败用例中属我域的 4 个（2026-10-15 11:15–11:18 HKT）

改动只在测试代码里，产品代码没动；diff 见 `fix.diff`，日志见 `test-run.log`。只跑了按名字筛选的测试，`CARGO_BUILD_JOBS=1`，MemAvailable 约 6.3GiB。

## lsp（src/lsp/mod.rs）
1. **normalize_makes_verbatim_path_prefix_compatible** → 只在 Windows 上跑（`#[cfg(windows)]`）。理由：用例断言的是 `Path::starts_with` 对 `E:\litecode` 按路径组件做前缀匹配，这是 Windows 的路径语义。Unix 上整个 `E:\...` 字符串只算一个组件，前缀匹配本来就不成立。`strip_verbatim` 的字符串处理在 `strip_verbatim_drive_and_unc` 里已经有跨平台测试。
2. **publish_diagnostics_uri_matches_drive_case** → 拆成两个。
   - 盘符大小写不敏感那半只在 Windows 上跑：Unix 文件系统区分大小写，`/E:` 和 `/e:` 是两个不同目录，在 Linux 上判为相等反而是错的。
   - verbatim URI 恢复（`file:////?/E:/...` 和 `file:///E:/...` 判为同一文件）是纯字符串逻辑，单独成新用例 `publish_diagnostics_uri_matches_mangled_verbatim`，所有平台都跑。
3. **uri_to_path_recovers_mangled_verbatim_file_uri** → 主体是跨平台逻辑，改成不依赖平台的断言：
   - `normalize_windows_file_uri` 能把畸形 URI 改写成 `file:///E:/litecode/src/agent/core.rs`；
   - 畸形 URI 和正常 URI 经 `uri_to_path` 得到同一路径。
   - 只有「结果等于 `E:\...`」这一条依赖 Windows 路径形式，保留在 `#[cfg(windows)]` 下。

## optional（src/optional/mod.rs）
4. **reconcile_starts_warmup_for_network_core** → 是**测试过时，不是产品缺陷**。
   - WP-167 经 Tools 签字的设计（`WP-167/cross-domain/tools.md` 第 (b) 条）规定：`reconcile` 只重建已经存在的 client，没有 client 时不预建，等首次工具调用时由 `ensure_warm` 构建。用例还在断言旧的「reconcile 立即预热」。
   - 已改成新约定：reconcile 之后未预热、构建次数为 0；第一次 `ensure_warm` 之后变为 Warm。用例名没改。
   - **与 WP-165 接入的 `defer_reconcile` 无关**：这个用例直接调用 `EngineManager::reconcile`，不经过 `serve::run`；而且它在 run2（`run.rs` 改动之前）就已经失败了。`defer_reconcile` 没有回退。

## 结果
筛选出的这 4 个用例（加上新拆出的 1 个）全部通过。另外两个用 `#[cfg(windows)]` 隔离的用例在 Linux 上不编译、不运行，需要以后在 Windows 上确认。
run2 失败名单里还有 `config::path`、`terminal::shell`、`tool::registry` 三个用例，不在本次范围内。
