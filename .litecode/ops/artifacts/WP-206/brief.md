# WP-206 brief — Code 模式测评修复（Andy 已拍板 8 项）

- 类型：implement；owner：backend_mid；approver：gm
- 基线：`feat/tool-contract-ptc` @ `80c7a91`；**不合 main**
- 来源：Andy 机器上 agent 对 Code 模式的测评 + GM 只读核对（结论：以下 8 项为真问题/已决策项）

## Goal
修掉测评中确认的正确性/一致性问题，并把 `run` 改回"短命令前台拿结果、真并发"的手感。

## Scope（全部已由 Andy 决定，不再讨论方向）
1. **run 前台窗**：`src/terminal/agent_jobs.rs:789` `BACKGROUND_EXIT_WINDOW` 500ms → **30s**（默认前台等待）。其余行为不变：窗口内退出→带 `exit_code`；超窗仍 detach，`exit_code:null`，`wait`/`kill` 照旧。更新 `descriptions/run.md`（说明约 30s 内结束直接返回结果，超出转后台用 wait）。
2. **run 锁**：非只读命令目前在整个调用期持 `ResourceKey::Workspace` 独占锁（`run.rs:168-190` + `tool/executor.rs:601-602`），导致并行 run 串行。改为**只在 spawn 前后持锁，前台等待前释放**，并行 run 真并发。`run.md` 加一句：并行命令可能触及同一文件，需要顺序时用 `await` 串起来。
3. **LSP 冷启动**：`document_symbol` / `workspace_symbol` / `diagnostics` 在服务器索引未就绪时必须 reject `unavailable`（走与 hover/definition 相同的 index-not-ready 路径，`lsp/server.rs:292-352` `request_nav_with_retry` 或等价），**不得**返回空成功（`hub.rs:933,1121-1137`；diagnostics 用 `fresh`/`server_ready` 判定）。
4. **read 越过 EOF**：`read.rs:358-362`（及 `:427`）改为 resolve 空窗口，不抛；`message` 写明文件共 N 行、`start_line` 超出。翻转测试 `read.rs` `test_read_start_line_past_end_errors`；`read.md` 对齐。
5. **glob 文档**：`glob.md` 写明不含 `/` 的 pattern 按 basename 在任意深度匹配（类 .gitignore；见 `workspace/filter/path_glob.rs:20-29`）。**仅文档**。
6. **boundary 路径**：`ptc/boundary.rs:109-115` bigint / symbol / function 错误带路径，复用 `path_at(frames)`（同 `undefined_at`）。
7. **undefined 宽容**：
   - 工具入参：静默剥除值为 undefined 的属性；
   - run_code 返回值：剥除 undefined 属性，数组中 undefined→null，并在 head 加一行 `dropped undefined at a.b, c[2]`（列表封顶，超出写 `+N more`）；
   - 同步修订 `docs/ptc/charter.md` I10（及 `run-code-remediation-plan.md` 对应段）、PRELUDE（`tool/code_manifest.rs`）与 `run_code` schema 中 "use null or omit" 文案。
8. **LSP 路径**：`lsp/format.rs`（`uri_to_path` 处 :98/:193/:394）位于工作区内的返回工作区相对路径，工作区外保持绝对，与 read/grep/glob 一致。

## Out of scope
忽略规则命中提示；read 单行超长截断/按列续读；后台完成通知内容；权限预设（All/Safe）行为。

## 验收标准
1. `run` 一条 ~2s 命令直接返回 `exit_code`（非 null）；>30s 命令返回 null 且可 wait/kill（测试）。
2. 4 个并行 ~1s 非只读命令（Promise.all）总耗时约 1s 而非约 4s（测试，给合理裕量）。
3. 模拟索引未就绪时三工具 reject `unavailable`；就绪后正常返回（测试）。
4. 越界 read resolve 空 `lines` + 含 N 行的 message；原测试翻转为通过。
5. glob.md 含 basename 任意深度说明。
6. bigint/symbol/function 报错含路径（如 `bigint at a.b`）（测试）。
7. 入参 undefined 属性不再报错；返回值 undefined 被剥除/置 null 且 head 出现 dropped 行（含封顶）；charter/PRELUDE/schema 文案一致（测试）。
8. LSP 工作区内结果为相对路径，工作区外为绝对（测试）。
9. SDK fixture（`src/tool/fixtures/run_code_sdk.txt`）与 sha 刷新，sdk_snapshot 测试绿；相关模块测试全绿；cargo 日志入 artifacts。

## 约束
- 分支 `feat/tool-contract-ptc`，push 实验分支，**不合 main**。
- 跑 cargo 前确认空闲内存 ≥ 2GiB。
- 改了任何描述/PRELUDE 必须同步 SDK fixture sha + 快照测试。
- 不改 web/；不顺手改 Out of scope 项。
