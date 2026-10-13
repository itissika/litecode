# WP-073 / WP-070-ide：ISSUE-043 stdin 继承 + ISSUE-044 ContentModified 有限重试
Owner ide；approver backend_mid；父包 WP-071。分支 ops/team-framework，**未 commit**；未碰 windows-*.yml；真机验证不在本包。

## 1. 改动清单（未提交工作区）
### ISSUE-043：12 处 Command 加 `.stdin(Stdio::null())`
| 文件:行 | 命令 | 类别 |
|---|---|---|
| src/lsp/deps.rs:133 | npm config get prefix | 探测 |
| src/lsp/deps.rs:358 | run_npm | 安装/探测 |
| src/lsp/deps.rs:416 | rustup which rust-analyzer | 探测（ISSUE 原址 :412） |
| src/lsp/deps.rs:452 | run_version_probe `<server> --version` | 探测（ISSUE 原址 :446-452，warmup timeout 主因） |
| src/lsp/deps.rs:649 | dotnet --list-sdks | 探测 |
| src/lsp/deps.rs:723 | installer（rustup component add / go install / dotnet tool install -g） | 安装 |
| src/lsp/install/adapters/rust_analyzer.rs:95 | ldd --version | 探测 |
| src/lsp/install/mod.rs:245 | go version | 探测 |
| src/lsp/install/mod.rs:265 | go install gopls@latest | 安装 |
| src/lsp/install/mod.rs:307 | dotnet tool install csharp-ls --tool-path | 安装 |
| src/lsp/install/npm.rs:108 | `npm_command_tokio`（覆盖 :152 npm --version、:193 npm install） | 探测/安装 |
| src/lsp/install/npm.rs:327 | npm --version（#[test] 内，测试代码） | 测试 |
不改：src/lsp/server.rs:144 LSP 主进程，已确认 `.stdin(Stdio::piped())`（:149），走协议。

### ISSUE-044：-32801 有限重试
- src/lsp/conn.rs:24 `LSP_CONTENT_MODIFIED = -32801`
- conn.rs:27 `CONTENT_MODIFIED_BACKOFF = [200ms, 500ms, 1000ms]`（最多 3 次重试，共 4 次尝试）
- conn.rs:36 `RETRYABLE_READ_METHODS` 只读白名单：hover、definition、typeDefinition、declaration、implementation、references、documentSymbol、documentHighlight、signatureHelp、foldingRange、selectionRange、semanticTokens/full|range、inlayHint、prepareCallHierarchy、callHierarchy/incoming|outgoingCalls、prepareTypeHierarchy、typeHierarchy/supertypes|subtypes、completion、codeLens、workspace/symbol；清单外一律不重试（rename/prepareRename/codeAction/executeCommand/formatting 等）
- conn.rs:62 `is_retryable_read_method`；conn.rs:67 `is_content_modified`（匹配 complete_response 生成的 `LSP error (code -32801)` 前缀，不改错误类型）
- conn.rs:508 `LspIo::request_retry_modified`：rpc_id=Some（编辑器透传）或非白名单 → 单发；否则遇 -32801 退避重试；下一次退避会超出 `request_timeout()` 总预算即停止；耗尽返回最后一次原错误；每次重试 tracing::debug
- src/lsp/server.rs:357 `send_request_id` 改走 `request_retry_modified`（覆盖 request_nav_with_retry / send_request_at_rev / hub 所有内部请求；want_rev 校验仍在其后）
- 单测（conn.rs tests，:694 起 mock：outbound 通道 + dispatch_message 回包）：content_modified_retries_then_succeeds（前 2 次 -32801，共 3 次请求后成功）、content_modified_exhausts_after_four_attempts（4 次后原错误）、content_modified_rename_is_not_retried（1 次）、content_modified_passthrough_rpc_id_is_not_retried（1 次）、is_content_modified_matches_only_32801

## 2. 交互输入检查结论
逐一核对 deps.rs/install/* 的安装命令：`rustup component add rust-analyzer`、`go install golang.org/x/tools/gopls@latest`、`dotnet tool install -g csharp-ls` / `--tool-path`、`npm install`（npm.rs:193）均为非交互命令，无 y/n 提示或交互确认（rustup component add 无确认；go install 无；dotnet tool install 无确认，首次运行横幅只输出不读入；npm install 不提示，npx 才会，本处未用）。且这些 stdout 原本已 null/piped，不存在用户可见的交互通道。**单列项：无**，12 处全部改。

## 3. 测试结果（来源状态：本包改动后工作区，HEAD 843a167 之后 + 未提交改动；共享 target）
| 命令 | 结果 | 日志 |
|---|---|---|
| `cargo test --lib lsp::` | 111 passed / 3 failed；我的 5 条新测全 ok。3 failed = lsp::tests::{normalize_makes_verbatim_path_prefix_compatible, publish_diagnostics_uri_matches_drive_case, uri_to_path_recovers_mangled_verbatim_file_uri}，在 src/lsp/mod.rs（未改动，HEAD 原样），为 Windows 路径语义测试在 Linux 上失败，与本包无关、预先存在 | .tmp/wp073_lib.log |
| `cargo test --test optional_lsp -- --nocapture </dev/null` | 23 passed / 1 failed / 1 ignored；失败为 agent_lsp_tool_views_with_rust_analyzer :1409（文案 `→ 1 unique`，属 WP-072 Tools 范围） | .tmp/wp073_opt_devnull.log |
| `sleep 2000 \| timeout 1200 cargo test --test optional_lsp -- --nocapture`（stdin 保持打开） | **24 passed / 0 failed / 1 ignored，无 warmup timeout**（:1409 此轮亦过，推测 WP-072 同期改动落地，未核实） | .tmp/wp073_opt_pipe.log |
| 修前对照（stdin 不关） | 25 个挂 19 个 warmup timeout（来源：后端中层定责记录 /tmp/lsp_run2.log 时期，未本包复跑） | — |
| `cargo test --test optional_lsp rust_analyzer_smoke -- --ignored --nocapture </dev/null` ×3 | 3/3 passed（4.5s/4.4s/4.1s）；rust-analyzer 可用（.litecode/ops/.toolchain） | .tmp/wp073_smoke{1,2,3}.log |

## 4. 残留问题
- lib lsp:: 3 个 Windows 路径测试在 Linux 失败（src/lsp/mod.rs，预先存在，非本包）——建议中层另判是否加 cfg(windows) 或修。
- optional_lsp :1409 归 WP-072。
- 修前对照数据来自既有记录，未在本包内重跑（避免占共享 target 时长）。
- -32800 RequestCancelled 未纳入（按方案保持最小）。
