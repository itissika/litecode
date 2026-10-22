# WP-206 dig-survey（只读，基线 0981ac7）

| # | 定位 | 根因 | 最小修法 | 域 |
|---|---|---|---|---|
| 1 run 前台窗 | `src/terminal/agent_jobs.rs:789` 常量；`:1038` select 分支；`src/tools/run.rs:98` 用 `PipeDetach::Background` | 窗口 500ms 是为 Windows shell 启动设的，太短，~2s 命令都 detach 成 `exit_code:null` | 常量改 30s（建议改名 `RUN_FOREGROUND_WINDOW`，注释同步）；其余 detach/wait/kill 逻辑不动；`descriptions/run.md` 写"约 30s 内结束直接返回 exit_code，超出转后台用 wait" | Runtime |
| 2 run 锁 | `run.rs:168-190` 非只读命令 `effects()=write(Workspace)`；`tool/executor.rs:601` `process_resource_gate().acquire` 得 `_effect_guard`，`:614-626` `write_lock.try_acquire` 得 `_lock_guard`，两者都活到 `:681` `tool.execute(...)` future 完成 → 整个前台等待期独占 | 锁按"调用期"持有；窗口一旦 30s，Promise.all 的 4 个 run 会被串成 ~4×耗时 | 通用机制：`ToolExecutionContext` 加 `spawn_guard: Option<SpawnGuard>`（包住 EffectGuard+WriteLockGuard，`release()` 幂等，Drop 兜底）；`Tool` trait 加 `fn releases_lock_after_spawn(&self)->bool {false}`，run 返回 true，executor 对这类工具把两个 guard move 进 context 而非局部变量；run 在 `supervise_pipe` 报告 `cmd.spawn()`（agent_jobs.rs:885）成功后 release（PipeRequest 加 `on_spawned: oneshot::Sender<()>`，run.rs 收到后 release；spawn 失败/取消同样 release）。=类似 VS Code task 只在启动时串行。**必须与 #1 同包同提交**。 | Runtime |
| 3 LSP 冷启动 | 导航走 `lsp/server.rs:292-352` `request_nav_with_retry`（空结果+`index_ready_for_empty_miss()` 为假时重试，最终 Err "index not ready"）；documentSymbol `hub.rs:1121-1128`、workspaceSymbol `hub.rs:1130-1137` 与 `hub.rs:904-944` `workspace_symbol_outcome` 直接 `send_request_outcome`；diagnostics `hub.rs:787-801` 服务器未跑返回 `diagnostics_silence_snapshot()`（`fresh:false,server_ready:false`，`:1239`）被工具当空成功 | 这三条绕过 index-not-ready 判定 | documentSymbol/workspaceSymbol 改走 `request_nav_with_retry`（`nav_empty` 增加：数组为空即空）；diagnostics 在工具层（`src/tools/lsp.rs`）若 `server_ready==false` 或（`fresh==false` 且 `!index_ready_for_empty_miss`）→ reject `unavailable`；错误码统一映射 `unavailable`（与 hover/definition 同） | IDE(LSP) |
| 4 read 越 EOF | `src/tools/read.rs:358-362`（文件路径）与 `:427`（`render_text_page`，虚拟会话）抛 `invalid_input` | 契约选择，非 bug | 改为 `finish_read(path,start_line,vec![],total,total,msg,false)`，msg=`File has N lines; start_line S is past the end.`；空文件 N=0 同理；翻转 `read.rs:749` 测试；`read.md` 对齐 | Tools |
| 5 glob 文档 | `workspace/filter/path_glob.rs:20-29` 全路径不中再按 basename 匹配 | 文档没写 | `descriptions/glob.md` 加：不含 `/` 的 pattern 按文件名在任意深度匹配（类 .gitignore），如 `*.rs`；仅文档 | Tools |
| 6 boundary 路径 | `src/ptc/boundary.rs:109-117` `illegal("bigint"/"symbol"/"function")` 无路径；`path_at` 在 `:237` | 在 `open()` 时当前值路径=`path_at(frames)`（同 `undefined_at`，`:276`） | 抽 `illegal_at(kind, frames)` → `"{kind} at {path}"`；三处改用；根值为 `$` | Runtime(PTC) |
| 7 undefined 宽容 | 拒绝点：`boundary.rs:96`（根）、`:211`（数组元素）、`:225`（对象属性）；入口：`engine.rs:769/783`（工具入参）、`engine.rs:976` `accept_value`（返回值） | `from_js` 一刀切判非法 | `from_js` 加 `UndefinedPolicy { Reject, Strip, StripOrNull }` + 收集 `dropped: Vec<String>`：对象属性 undefined→跳过（记路径），数组元素→`null`（记路径），根 undefined→`null`；入参用 Strip（不报告），返回值用 StripOrNull 并在 render head 加 `dropped undefined at a.b, c[2]`（封顶 10 条，超出 `+N more`）；`throw`/details（`engine.rs:1136`）保持现状；改 `code_manifest.rs:28` PRELUDE、run_code schema、`charter.md:52` I10、`run-code-remediation-plan.md:248,361` 同步；更新 `code_manifest.rs:820` 断言 | Runtime(PTC) |
| 8 LSP 路径 | `src/lsp/format.rs:98/193/394` `uri_to_path(...).to_string_lossy()` 绝对路径 | format 无 workspace_root | `format_action_result` 等加 `workspace_root: &Path` 参数，复用 `workspace::filter::cheap_rel_under`（session_search spill 同款）：工作区内→相对（`/` 分隔），外→绝对；调用点 `hub.rs:940`、`tools/lsp.rs` | IDE(LSP) |

## 锁（硬约束 #2）
- 现状：`executor.rs:601` EffectGuard（进程级 resource gate，同进程互斥）+ `:614` WriteLockGuard（跨 session）均为 `run_tool` 局部变量，作用域覆盖 `tool_fut` 及 timeout 等待 → 持有 = spawn + 全部前台等待（现 ≤500ms，改后 ≤30s）。
- 改法：见 #2，锁持有缩到「acquire → cmd.spawn() 返回」；前台等待、detach、输出收集都不持锁。只读命令（`is_readonly_command`）本来不加锁，不变。
- 同包同提交：#1 与 #2 一个 commit；验收测 4×`sleep 1 && touch f$i` Promise.all ≤ ~2s。

## 文件冲突 / 顺序
- 共享热点：`src/tool/fixtures/run_code_sdk.txt` + `.sha256`（run.md/read.md/glob.md/PRELUDE/schema 都进 SDK 快照）。
- 不重叠：A(Runtime #1#2：agent_jobs.rs, run.rs, executor.rs, resource_gate.rs, tool trait, run.md)；B(Tools #4#5：read.rs, read.md, glob.md)；C(IDE #3#8：lsp/server.rs, hub.rs, format.rs, tools/lsp.rs)；D(PTC #6#7：boundary.rs, engine.rs, code_manifest.rs, run_code schema, docs/ptc)。
- 风险：#2 改 executor 公共路径，影响所有写工具；Drop 兜底保证不漏锁；30s 窗口让单个 run 调用变长（Code 模式 timeout 需 ≥30s，需核对 run_code 总预算）。#7 放宽 I10，charter 需 Andy 已拍板（brief 已写）。

## Q（带推荐）
- Q1 根值 `return undefined`：推荐 → `null` + head 记 `dropped undefined at $`。备选：保持报错。
- Q2 入参剥除是否也报告？推荐不报告（brief 写"静默"）。
- Q3 dropped 封顶条数：推荐 10。
- Q4 run_code 自身超时 < 30s 时，run 窗口取 min(30s, 剩余预算)？推荐是，避免 run_code 因前台等待超时。
- Q5 diagnostics 服务器"已跑但未 publish"（fresh=false, server_ready=true）：推荐等至 INDEX_GRACE 后仍无→返回空成功（真无诊断），之前 reject unavailable。
- Q6 read 越界对空文件（N=0）：推荐同样 resolve 空 lines + message。
- Q7 LSP 工作区外路径是否也经 symlink canonicalize？推荐不做，保持 uri_to_path 原值。
