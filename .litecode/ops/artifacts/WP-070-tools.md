# WP-070-tools（WP-072 · Tools）方案

状态：方案（drafting→awaiting_approval），批前不动代码。不 commit，不碰 windows-*.yml。

## ① ISSUE-042：litecode_workspace 生产路径自开 sessions.db
现状：`src/tools/litecode_workspace.rs:821` `call_inner` 构造 ToolExecutionContext 时 `session: Some(SessionDataReader::open(&runtime.workspace.paths.sessions_db))`。
`call_inner` 由 `src/tool/trait_.rs:40` 的默认调用路径触发，所以算生产路径。正式路径 `execute()` 已经使用注入的 `execution.session`（`execution.session_reader()`，在 :487）。
方案：
- R1：`call_inner` 不再自己 open。改为 `self.run(&input, &ToolExecutionContext{ session: None, .. })`。这样 session 段会走现有的 `section(.., Err)` 分支，显示 "unavailable"，不会 panic。生产调用一律走 `execute`，使用注入的 reader。
- 备选 R1'：如果要求 call_inner 也能给出完整会话段，就让工具在构造时持有注入的 `SessionDataReader`（由 registry 从 `SessionManager::reader()` 传入，见 `src/session/manager.rs:367`）。这一步只调用现成的 Session API，Session 层不用新增接口。
- 结论：**Session 层不需要新增接口**。现有的 `ToolExecutionContext.session` 注入加上 `SessionManager::reader()` 已经够用。
- 验证：`cargo test --test session_data_sql_death_list` 转绿；`cargo test --test subagent_session_isolation`（:791-824 用到这个工具的 call_inner）保持绿，必要时在测试 fixture 里注入 reader，不改断言语义；litecode_workspace 单测也要绿。

## ② optional_lsp.rs:1409 agent_lsp_tool_views_with_rust_analyzer
现状：formatter `src/tools/lsp.rs:455` 输出 `matched {n} times on {loc} → {unique} unique definition`，箭头后带空格。同一文件的单测 `lsp.rs:841` 断言的也是 `→ 1 unique`。只有 RA 集成测 :1409 写的是 `→1 unique`。git 历史显示格式来自 b4f04e7（line formatting 重构），104fd43 没改这一句。
裁定：**改测试，不改 formatter**。理由：formatter、单测、mock 集成测三处口径一致，RA 测试是唯一的漂移点，且仅差空格；改 formatter 会把已经对齐的单测打红。
等级断言：多命中路径是 `ToolCallResult::ok(body).with_warning(..)`（:683），`with_warning` 会把 level 设成 Warning，和 :1408 的期望一致。所以等级不符很可能来自前面的单命中 hover（:1387 期望 Ok），或者 RA 没就绪时走了错误/提示分支。落地时按真实 RA 输出逐条核对：formatter 行为符合 mock 测（:1110-1192）口径的，就改 RA 测试断言；如果发现 formatter 真有回退，再单独报中层。
- 观测限制：box 上没有 rust-analyzer（rustup proxy 未安装组件）。只读复跑（`</dev/null`）停在 :1366 "goToDefinition should succeed after RA index"，没走到 :1409。**工具需求**：请工具集管家在 box 上安装 rust-analyzer（`rustup component add rust-analyzer`），否则这条只能做静态修正，无法实跑验绿。

## 回归 / 验收
- `cargo test --test session_data_sql_death_list`
- `cargo test --test subagent_session_isolation`
- `cargo test --lib tools::lsp tools::litecode_workspace`
- `cargo test --test optional_lsp -- --include-ignored </dev/null`（需 RA）

---
## 落地记录（start 后 · 2026-10-13）

### ① ISSUE-042：选用 R1'（复用工具已持有的 SessionManager::reader()）
可达性证据（rg `\.call\(|\.call_inner\(|call_async\(` 全量 src）：
- trait 默认链：`src/tool/trait_.rs:68` 默认 execute → call_async(:58) → call(:40) → call_inner。`LitecodeWorkspaceTool` 自己覆盖了 `execute`（`src/tools/litecode_workspace.rs` 约 :803，直接调 `self.run(&input,&execution)`），所以生产路径走不到这条默认链。
- 生产调度只有一处：`src/tool/executor.rs:387` `tool_clone.execute(input, execution)`。
- `src/` 中其余 `.call(`/`.call_inner(` 命中都在 `mod tests` 里（ask_user:353+、grep:1188+、read:626+、todo:191+、plan/write/websearch/code_search/edit/tests.rs）。webfetch.rs:231 调的是它自己的 call_inner。registry.rs:149 只负责构造，不调用。tests/subagent_session_isolation.rs:791-824 调的是 SubagentListTool，跟本工具无关。
- 结论：call_inner 没有可达的生产调用方，R1 的前提成立。但这个工具在构造时已经持有 `sessions: Arc<SessionManager>`（:279），直接改成 `session: Some(self.sessions.reader())` 不用改任何接线，还能保持 call_inner 的会话段可用，所以选 R1'，比改成 None 更安全。Session 层不需要新增接口。
- Diff：litecode_workspace.rs call_inner 中 `SessionDataReader::open(&runtime.workspace.paths.sessions_db)` 改为 `self.sessions.reader()`。
- 验证：`cargo test --test session_data_sql_death_list` 2 passed（转绿）；`cargo test --test subagent_session_isolation` 15 passed（保持绿，测试没改）；`cargo test --lib tools::litecode_workspace` 14 passed；`cargo test --lib --no-run` 编译通过。

### ② optional_lsp.rs:1409：改测试（`→1` 改为 `→ 1`），formatter 不变
- RA 修好后先撤掉我之前对测试文本的修改，用原测试复跑两次：`cargo test --test optional_lsp agent_lsp_tool_views_with_rust_analyzer -- --include-ignored --nocapture </dev/null`，日志 /tmp/wp072_ra_run1.log、/tmp/wp072_ra_run2.log。两次都 FAILED，panic 在 tests/optional_lsp.rs:1409:5。真实输出是 "text `target` matched 2 times on L4 → 1 unique definition"，确认失败可以复现。
- 等级断言：实跑中 :1408 `level == Warning` 通过（输出末尾有 "Warning: narrow text and retry…"），等级不需要改。
- 改测试文本以后复跑两次，日志 /tmp/wp072_ra_run3.log、/tmp/wp072_ra_run4.log：都是 `1 passed`，各约 4s。
- 说明：中层转述"工具集管家全量跑通过"，和这里两次复现不一致。推测对方跑的时候工作区里还留着我当时那份临时文本修改（/tmp/lsp_ra_fix.log 只证明已经过了 :1366）。

---
## WP-075（ISSUE-047 A）落地记录 · 2026-10-13
改动（只改了 src/tools/lsp.rs）：
- 非位置类动作（单次查询）和按位置查询的命中循环，都从 `tool_action_with_query` 换成 `hub.tool_action_outcome`，仍然取 `.text`。
- 新增 `CONTENT_MODIFIED_WARNING` 和 `with_content_modified()`：content_modified=true 时调 `with_warning(...)`。循环里只要有一次命中为 true 就标记。多命中时和原有的 "narrow text and retry…" 合成一条 Warning。处理逻辑对所有语言通用，不做 Rust 特判，-32801 不重试。
- 新增单测 `tools::lsp::tests::content_modified_surfaces_updating_warning`。
验证：
- `cargo test --lib tools::lsp`：27 passed（含新测）
- `cargo test --lib --no-run`：编译通过
- `cargo test --test optional_lsp -- --include-ignored </dev/null`（/tmp/wp075_optlsp.log）：23 passed，2 failed
  - agent_lsp_tool_views_with_rust_analyzer 在 :1387 失败（hover 单命中期望 level=Ok）。单独跑两次都稳定复现（/tmp/wp075_agent_lsp_tool_views_with_rust_analyzer_{1,2}.log）。真实 RA 对这次 hover 回了 -32801，输出是 "No hover information available" 加上新的 ContentModified 警告。以前这条测试靠 `len()>5` 这个很弱的断言，把空 hover 当成通过；现在工具如实透出了这个信号，所以属于预期中的行为变化，暴露出测试原本的假设不对。
  - rust_analyzer_smoke：在整组里跑失败（报 mock_lsp_server not configured，像是并行测试之间的环境变量串扰），单独跑 2/2 通过。
- 待中层裁定：:1387 的处理不在已批方案里，暂不改。建议改测试：单命中 hover 在 content_modified 时按警告提示轮询重试，直到拿到非 ContentModified 结果，或者把这种情况接受为 Warning。裁定前不 deliver。

### WP-075 追加：按中层裁定 A 修改 :1387（只改测试，产品代码不加重试）
- 修改位置：tests/optional_lsp.rs，agent_lsp_tool_views_with_rust_analyzer 的单命中 hover。
  - 改成有界轮询：每 500ms 一次，最多 20s；返回 Warning 时继续轮询。
  - 轮询中每次收到的 Warning 都必须包含 "language server is updating"。
  - 最终结果必须 level=Ok，且包含 "target"；原来只检查长度大于 5，已经收紧。
- 真实 RA 单独跑 2/2 通过（/tmp/wp075_ra_fix{1,2}.log）。每次都是 2 次尝试：第 1 次是 ContentModified 加提示，第 2 次拿到正常结果。
- 全组 `cargo test --test optional_lsp -- --include-ignored </dev/null`：跑了两次，各 24/25 通过，都不是真实 RA 那条在失败。
  - 第 1 次（/tmp/wp075_optlsp2.log）：edit_fix_must_not_echo_previous_lsp_error 在 :836 失败（并行时没及时收到诊断）。单独跑 3/3 通过，它走的是 edit 诊断路径，不经过 lsp 工具的 hover/定义查询，判断和本包无关，是已有的并行不稳。
  - 第 2 次（/tmp/wp075_optlsp3.log）：rust_analyzer_smoke 失败，就是中层已知、会另外登记 issue 的那个并行串扰问题。
- `cargo test --lib tools::lsp`：27 passed；`cargo test --lib --no-run`：编译通过。
