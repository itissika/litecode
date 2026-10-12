## WP-027 Tools 方案（tools，direct）
### 0 现状快照（WP-018 作废时，未回退任何东西）
- WP-018-tools.md 不存在；WP-018 无 Tools 产物。
- custom.rs 已有改动（含中层半成品）：run_sync_with_lifecycle stdin write_all（约 :295-305）**已真正**放过 ErrorKind::BrokenPipe，其余写错误仍 kill+error；新增测试 child_ignoring_stdin_is_not_a_broken_pipe_failure（unix，1MiB 输入+`exec 0<&-`，未跑）；其余为 rustfmt 格式化。
- cli.rs：只有 AskAnswer / ask_cli_ask_user 返回类型的 rustfmt 换行，无语义改动。
- 本机 box rustc 1.85.1 < 依赖要求 1.88（ort/time），cargo test 无法编译，未能实跑。
### 1 Broken pipe 根因与修复方向
根因（待实跑确认）：background_real_process_begin_job_then_mailbox_notice 的子进程 `sh -c "echo custom-bg-marker"` 不读 stdin；父进程 spawn 后再 write_all(input_json)。子进程先退出（关闭读端）→ write 得 EPIPE → 旧代码判 "stdin write failed: Broken pipe"。批量跑负载高时 sh 退出更早于写入，故偶发；单跑写入抢先进 pipe buffer，故 2/2 过。纯竞态，非逻辑错。
修复方向：EPIPE 不算工具失败，以退出码/输出为准（即现半成品做法）；同时核对后台路径（custom_hub begin_job）是否复用同一写入函数，若各自写 stdin 则统一走一个 helper。用 1MiB 输入的确定性测试固化，再把原 flake 测试批量跑 ≥20 次验证。
### 2 custom.rs 半成品：采用（小改）
理由：EPIPE 放过逻辑正确且最小；测试设计可确定性复现。小改：补注释说明非 EPIPE 仍致命；检查 Windows 对应错误码（ERROR_NO_DATA→BrokenPipe 映射）；测试实跑通过后保留。格式化 hunk 保留。不回退。
### 3 可疑实现排查范围
src/{tool,tools,permission,mcp,hook,knowledge,optional}，四类：并发/竞态（custom_hub mailbox 与 seam、ask 串行、MCP 连接池）、回滚（权限规则写入、plan 审批失败）、半写入（knowledge index/文件写、settings 写）、失败路径（wait panic 兜底 custom.rs wait_child_cancellable 的 panic!、子进程 kill 树、webfetch DNS 失败/代理残留）。逐条 file:line + 严重度(arch/drift/bug/suspect)；小问题域内修+补测，大问题只登记报中层。
### 4 测试裁剪清单（只列不删）
按 可删（重复覆盖、已被集成测替代）/ 可放宽（依赖时序 sleep、断言过严字符串）/ 重复（tests/permission_pipeline 与 client_protocol_permission 重叠门禁）三类，逐条给测试名+理由+替代覆盖。
### 5 未提交文件处置（逐文件，结论在产物表中落实）
- tool/{availability,registry}.rs, tools/{ask_user,bash_jobs_contract,custom_hub,knowledge,litecode_workspace,mod,plan,webfetch,websearch}.rs, permission/{mod,sinks}.rs：逐个看 diff，区分 功能(ISSUE-012/021/030 等已交付) / 纯格式化 / 测试；保留并 cargo check+test 本域通过后标「待提交」。
- tools/custom.rs：见 §2，保留。
- knowledge/{corpus,fsref,index,index_tests,mentions,spec_tests,validate}.rs：同上逐文件核对；疑为纯格式化者标注。
- 关联 tests/{client_protocol_permission,common/permission,permission_pipeline}.rs 属本域测试，一并收（WP-022 未列，报中层确认归属）。
- src/cli/cli.rs：仅 AskAnswer 格式化 hunk，保留，标「纯格式化」。
- 不 commit，由 GM 统一提交。
### 6 与 IDE 基座互审
交 IDE：custom.rs 进程 spawn/stdin/wait/kill_child_tree、custom_hub 后台 job 生命周期、EPIPE 修复；审 IDE 的 workspace/service.rs 及终端进程生命周期。双方在对方产物末尾附「交叉审」段。
### 7 工具需求
需要可编译本仓库的 rustc ≥1.88（box 当前 1.85.1，无法 cargo test）。请中层转工具集管家，或指定实跑环境。

---
## 执行记录（WP-027 已批，start 2026-10-12 15:0x HKT；box rustc 1.85.1 无法编译，以下改动均 **未编译/未实跑**）

### A. Broken pipe 修复（src/tools/custom.rs，未验证）
- 结论：后台路径 start_background(:192) 复用 run_sync_with_lifecycle，**不存在第二处 stdin 写入**，无需另抽统一入口；新增 `spawn_stdin_writer`（唯一写 stdin 处）。
- 改动：
  1. stdin 改为独立线程写入（`spawn_stdin_writer`），BrokenPipe→Ok，其他错误在 Finished 分支报 `stdin write failed`。同时修掉原同步写的**死锁**：子进程先写满 stdout(>64KiB) 再读 stdin 时，旧代码卡在 write_all，超时/取消循环永远进不去（bug，中）。
  2. Cancelled/TimedOut 分支不 join writer（kill 后自然 EPIPE 退出），避免 kill 前 join 死锁。
  3. Windows：std 把 ERROR_BROKEN_PIPE(109)/ERROR_NO_DATA(232) 映射为 ErrorKind::BrokenPipe，同一判断覆盖，已在注释说明。
  4. 中层半成品：**采用**（EPIPE 放过语义保留，迁入 helper；测试 child_ignoring_stdin_is_not_a_broken_pipe_failure 保留，待实跑）。
- 顺带小修（同文件，未验证）：
  5. wait_child_cancellable stdout/stderr 原为同线程顺序读 → stderr 写满 64KiB 时子进程卡住直到超时（bug，中）。改为 stderr 独立线程。
  6. start_background 结果分类原用 `content.contains("timed out"/"cancelled")`，子进程正常输出含这些词即被误判 Error/Cancelled（bug，中）。改为仅看 job_cancel + level。
  7. unix kill_child_tree 原只 kill(pid)，名不副实，`sh -c "a|b"` 孙进程泄漏并可能持有管道使读线程挂住（bug，中）。改为 spawn 时 `process_group(0)` + `kill(-pid)`，与 terminal/pty.rs 组杀一致。副作用：子进程不再收到终端 Ctrl-C（由我们统一 cancel，可接受）。

### B. 可疑实现清单（file:line 为改后行号近似；hook/ 目录不存在）
| # | 位置 | 问题 | 严重度 | 处置 |
|---|---|---|---|---|
|1|tools/custom.rs run_sync stdin 写|同步写 stdin 与 stdout 读互相等待死锁，超时失效|bug 中|已修(A1) 未验证|
|2|tools/custom.rs wait_child_cancellable 读线程|stdout/stderr 顺序读|bug 中|已修(A5) 未验证|
|3|tools/custom.rs start_background 分类|按输出字符串判 timeout/cancel|bug 中|已修(A6) 未验证|
|4|tools/custom.rs kill_child_tree unix|只杀单 pid|bug 中|已修(A7) 未验证|
|5|tools/custom.rs ~:430 wait 错误分支|try_wait 失败兜底 `panic!` 在 spawn_blocking/后台线程内：后台线程 panic → 永不 push_notice，job 挂起|suspect 中|登记；建议改为 Finished(合成失败状态)/返回 Error，需编译验证|
|6|tools/custom.rs start_background `let _ = thread::Builder…spawn`|线程创建失败被吞，已返回"started"但永无 notice，hub 中 job 不释放|suspect 低|登记；建议失败时 cancel_job+返回 error|
|7|tools/custom.rs 读线程 `rx.recv()`|子进程退出但孙进程继承 stdout 时 recv 永久阻塞（超时不覆盖 Finished 后阶段）|suspect 中|A7 组杀缓解；彻底修需 Finished 后带超时 recv，登记|
|8|mcp/client.rs:55-70 send_request|read_line 无超时；按"下一行即响应"匹配，服务器先发 notification/日志行即错配|arch 高|登记，报中层（协议层改造，非小修）|
|9|mcp/pool.rs:104-109|hub 线程/runtime 创建用 expect，失败直接 panic 进程|suspect 低|登记|
|10|mcp/client.rs:40-41|stdin/stdout expect：piped 下不会触发|— |无需处理|
|11|tool/output.rs:211 spill_content_if_needed|fs::write 非原子，崩溃留半写 blob；引用前未校验|half-write 低|登记；建议 tmp+rename|
|12|optional/mod.rs:80-246|RwLock `.expect("lock")`：任一持锁 panic 后整个引擎状态永久中毒连环 panic|suspect 低|登记；可改 `unwrap_or_else(PoisonError::into_inner)`|
|13|permission/grants.rs:352|`lock().unwrap()`（在测试附近，需确认是否测试代码）|suspect 低|待编译后确认|
|14|tools/webfetch.rs ISSUE-021 残留|系统代理自行解析 DNS，pin 失效|已知残留|保持登记|

### C. 测试裁剪清单（只列不删）
| 测试 | 建议 | 理由 |
|---|---|---|
|tools/custom.rs background_real_process_begin_job_then_mailbox_notice|放宽|EPIPE 修后应稳定；mailbox 等待超时偏短可放宽到 10s，失败信息已含 marker|
|tools/custom.rs child_ignoring_stdin…（新）|保留|确定性覆盖 EPIPE|
|tests/permission_pipeline.rs authorize 门禁 vs tests/client_protocol_permission.rs ask_user/approval|重复（部分）|两处都断言 Ask 往返/拒绝路径；保留 WS 端到端，pipeline 中纯 sink 往返的可删|
|tools/ask_user.rs 多问题格式化断言（selected_ids/free_text 字符串）|放宽|字符串逐字断言脆，可改结构化 answers map 断言|
|tools/plan.rs 拒绝文案断言 "No plan file was created"|放宽|文案改动即挂，改断言"无文件生成"|
|knowledge/spec_tests.rs 错误文案 contains（10MiB/UTF-8/directory/Lines outside）|重复|与 index_tests 的 ref_status reason 断言覆盖同一分支；保留 reason 断言，文案只留一条|
|tools/webfetch.rs whatwg 解析测试（已改名）|可删候选|已不作 SSRF 门禁，只测第三方 url crate 行为|
|tools/websearch.rs 无引擎构造测试|保留|轻量|

### D. 未提交文件逐文件处置（基于去空白比对 HEAD）
- **纯格式化（rustfmt）/import 顺序**，保留、随批提交：src/cli/cli.rs（仅 AskAnswer 与 ask_cli_ask_user 返回类型换行）、knowledge/{corpus,fsref,index,index_tests,mentions,spec_tests,validate}.rs、permission/{mod,sinks}.rs、tool/{availability,registry}.rs（registry 仅 ask_user import 位置）、tools/{ask_user,bash_jobs_contract,custom_hub,knowledge,litecode_workspace,mod,plan,webfetch,websearch}.rs、tests/{client_protocol_permission,common/permission,permission_pipeline}.rs。
  - 注：比对显示上述均无语义差异，WP-012/015/021 的功能改动已在 HEAD。
- **语义改动**：tools/custom.rs（A 节全部 + 中层测试）——实跑通过后提交；不过则回退 A5–A7 只保留 A1–A3。
- workspace/service.rs 属 IDE，未动。
- 不 commit，等 GM。

### E. 交叉审（Tools → IDE 基座：进程生命周期）
- terminal/pty.rs:30-50 kill_process_group：unix `kill(-pid)` + Windows taskkill /T + child.kill 兜底，**同意**；PTY 子进程为会话首进程，组杀有效。
- pty.rs:237-260 先 try_wait 再 kill，reader join 有界：**同意**。
- pty.rs:278 Drop 中 kill：**同意**；建议确认 Drop 不在 tokio worker 上阻塞 join。
- terminal/mod.rs:195/451 kill 后未见 wait 回收（需确认 Drop 路径是否 reap，否则 zombie）：**疑问**，请 IDE 给证据。
- workspace/service.rs：本次 diff 仅删 4 行（+1/-4），未涉进程生命周期，无异议。
### F. 请 IDE 审 custom.rs
1. spawn_stdin_writer 线程与 Cancelled/TimedOut 分离 detach 是否会泄漏线程（kill 后 EPIPE 应退出；若孙进程持有 stdin 则不退）。
2. process_group(0)+kill(-pid) 与 pty.rs 组杀语义一致性；Windows 仍为 taskkill /T。
3. wait 错误分支 panic!（B5）与读线程 recv 无界阻塞（B7）。
4. 后台线程 spawn 失败吞错（B6）。

### G. rustc 升级后待办（阻塞于编译）
1. `cargo check` / `cargo clippy` 本域（custom.rs 全部改动从未编译；关注 io::Error::other、CommandExt::process_group 导入）。
2. 新测试 child_ignoring_stdin_is_not_a_broken_pipe_failure 实跑。
3. background_real_process_begin_job_then_mailbox_notice 全量批量环境下循环 20 次 0 失败。
4. 本域测试：`cargo test --lib tools:: tool:: permission:: knowledge:: mcp:: optional::` + `--test client_protocol_permission --test permission_pipeline`。
5. 补测（编译后写）：大 stdout 先写后读 stdin 不死锁；stdout 含 "cancelled" 不误判；sh -c 孙进程被组杀。
6. B5/B6/B12 若中层批准再修。

---
## 实跑记录（rustc 1.99.0，2026-10-12 15:10–16:20 HKT，共享 target）
### 1. check / clippy
- `cargo check --lib --tests`：通过，custom.rs 改动无需修编译错误。
- `cargo clippy --lib --tests`：custom.rs / custom_hub.rs / optional/mod.rs **0 条**。本域其余约 30 条为既有告警（tools/read,grep,edit,plan,knowledge…、mcp/client.rs:165、mcp/pool.rs:267 等），本包不处理，登记。
### 2. 新测试（全部通过）
- child_ignoring_stdin_is_not_a_broken_pipe_failure ✅
- large_output_before_reading_stdin_does_not_deadlock ✅（stdout+stderr 各 1MiB 后再读 1MiB stdin，<15s）
- background_output_mentioning_cancelled_is_ok ✅（输出含 cancelled/timed out → Ok）
- timeout_kills_grandchild_process ✅（`(sleep 3; touch M) & wait`，超时 1s 后 M 不出现）
### 3. B5 / B6 / B12 已修（各一测，均通过）
- B5 custom.rs wait_child_cancellable：try_wait 出错后 wait 也失败 → 杀进程组并返回新变体 `WaitEnd::WaitFailed`，结果为 Error（不再 panic!）。测：wait_failure_returns_error_instead_of_panicking（先 waitpid 抢收造成 ECHILD）✅
- B6 custom.rs start_background：抽 `start_background_with(…, spawner)`；线程创建失败 → `hub.abandon_job(job_id)`（custom_hub.rs 新增）+ 返回 Error，不再宣称已启动。测：background_spawn_failure_is_reported_and_job_released ✅
- B12 optional/mod.rs：全部 `RwLock .expect("engine … lock")` → `unwrap_or_else(PoisonError::into_inner)`。测：poisoned_state_lock_does_not_cascade_panics ✅
### 4. MCP（B8）→ ISSUE-034（只登记不改代码）
- 证据：src/mcp/client.rs:57-94 send_request（:72-73 写+flush，:75 read_line 无超时，:82 直接解析、不比对 id）；src/mcp/pool.rs:285-289 持 client Mutex 跨整个 initialize/call_tool 且无超时（仅 :402 握手有 START_TIMEOUT）。
- 复现：①假 server 在响应前先发一行 notification → 返回 Null/错结果，真响应残留，下次调用拿到（错号）；②server 不回复 → 永久挂起，同 server 后续调用全阻塞于 pool.rs:285 锁；③响应延迟期间取消回合 → 下一请求读到上一请求响应。
- 建议：按 id 循环读、丢弃 notification；每请求 timeout；取消/超时后标脏重建连接。
- ⚠ 过程事故：首次误写为 ISSUE-031.json，覆盖了他域 15:40 新建的同名未跟踪文件（原内容不可恢复，未进 git）。已改存 ISSUE-034，ISSUE-031 文件已移除，需原作者（疑为 WP-026 上下文域，同时间段建了 032/033）重建。
### 5. Broken pipe 20 连跑
- 方式：同一测试二进制连续 20 次跑全部 `tools::`（393 测，16 线程，模拟批量负载）。
- background_real_process_begin_job_then_mailbox_notice：**20/20 通过，0 失败**。
- 同批次中非本包改动的失败（登记，不在本包修）：tools::lsp::resolve_strips_windows_verbatim_after_canonicalize_shape 20/20 失败（Windows 路径形态测试在 Linux 必挂，疑需 cfg(windows)）；tools::bash::live_small_echo_exit_code_first_file_on_disk_not_in_view 4/20、live_large_output_windows_and_keeps_full_log 2/20、bash_jobs_contract::short_command_uses_completed_view_not_running_status 2/20（并发负载下偶发，新 flake 候选，归入裁剪/放宽清单待查）。
### 6. 本域测试
- lib：tool:: 77 ✅、permission:: 20 ✅、knowledge:: 46 ✅、mcp:: 4 ✅、optional:: 7 ✅、tools::custom* 24 ✅（tools:: 全量见上，除上述既有失败外全过）。
- 集成：client_protocol_permission 8 ✅、permission_pipeline 16 ✅。
### 7. 仍未完成
- B7（读线程 recv 无界）、B9、B11 未修，保持登记；IDE 交叉审未回。

---
## 追加（2026-10-12 15:5x HKT）
- **ISSUE-031 更正**：据后端中层，原 ISSUE-031 内容已以 **ISSUE-035** 留存，无需重建。教训：建 issue 前先确认编号空闲（`ls issues` + 写前存在性检查）。MCP 问题仍为 ISSUE-034。
- **lsp 测试**：`tools::lsp::tests::resolve_strips_windows_verbatim_after_canonicalize_shape` 位于 src/tools/lsp.rs:1086（Tools 域），已加 `#[cfg(windows)]` + 说明注释。Linux 下 `cargo test --lib tools::lsp` 编译通过，26 passed 0 failed，该测试被跳过、无新告警。
- **测试裁剪/放宽清单补充**（只列不修）：
| 测试 | 建议 | 理由 |
|---|---|---|
|tools::bash::tests::live_small_echo_exit_code_first_file_on_disk_not_in_view|放宽|tools:: 16 线程连跑 20 次挂 4 次；依赖实时 PTY 输出落盘时机，建议放宽等待或改为轮询直到文件出现|
|tools::bash::tests::live_large_output_windows_and_keeps_full_log|放宽|连跑 20 次挂 2 次；大输出写日志时序敏感，同上|
|tools::bash_jobs_contract::short_command_uses_completed_view_not_running_status|放宽|连跑 20 次挂 2 次；负载下短命令完成前就取视图，测试假设"短命令必已完成"，应等待完成或接受 running→completed|
- B7 / B9 / B11 维持仅登记，不修。WP-027 等 IDE 交叉审后再 deliver。

---
## 交叉审（IDE 基座，2026-10-12 HKT，只读）

对象：`src/tools/custom.rs`（Tools 已改完的工作区版本，HEAD c6b24df + 未提交 diff +308/-31）。全部为读码推理，未实测，未改代码。行号对应当前工作区。

| # | file:line（符号） | 发现 | 严重度 | 与 Tools 登记的关系 |
|---|---|---|---|---|
| X1 | `src/tools/custom.rs:449-457`（`wait_child_cancellable`，`Ok(Some(status))` 分支） | 直接子进程退出后，`rx.recv()` 会一直等，直到 stdout/stderr 两个管道都到 EOF。**这一步不受 deadline 和 cancel 约束**。如果孙进程继承了 stdout 并在后台继续运行（例如脚本 `long_task & echo done`），同步调用会挂住整个 turn，后台 job 也一直不 settle，超时形同虚设。 | 中（偏高） | 与 B7 同源。B7 登记的是“读线程 recv 无界”；这里补充触发条件是“孙进程持有管道 + 父进程先退出”，并且确认超时和取消都救不回来 |
| X2 | `src/tools/custom.rs:334-366`（`run_sync_with_lifecycle` 的 `WaitEnd::Finished`） | 正常结束路径**不对进程组做 kill**。同组的孙进程会在父进程退出后变成孤儿继续运行，没有任何回收。只有 Cancelled/TimedOut 路径（374-380 行）才会 `kill_child_tree`。 | 中 | 新增。与 X1 叠加：孙进程既挡住读取又不被清理。建议 Finished 后也对组发一次 kill（父进程已退出，组内剩下的都是遗留进程），或者对剩余进程有界等待后再 kill |
| X3 | `src/tools/custom.rs:339-346`（Finished 分支 join stdin writer） | stdin writer 由 `join()` 无界等待。如果孙进程继承了 stdin 但不读，并且输入大于管道缓冲（Linux 上 64 KiB），`write_all` 会一直阻塞，join 也跟着挂住。EPIPE 只在所有读端都关闭时才会出现。 | 低 | 新增，与 X1 同族，罕见（需要大输入） |
| X4 | `src/tools/custom.rs:306-312`、`:314`（spawn） | 子进程用 `process_group(0)` 单独成组，但**没有调用 `crate::proc_lifetime::bind_child_to_parent`**（LSP 和 code_search worker 都调用了）。litecode 崩溃或被强杀时：Windows 上不在 Job 里，不会级联退出；Unix 上进程独立成组，也不会收到终端 SIGHUP。后台 job 会运行到自己结束，没有人负责超时。 | 中 | 新增。Unix 上 `bind_child_to_parent` 本身是 no-op，这一项主要影响 Windows。Unix 上能靠 stdin 管道 EOF 让子进程退出，前提是工具会读 stdin |
| X5 | `src/tools/custom.rs:463-467`（`WaitEnd::WaitFailed`）+ `:485-503`（`kill_child_tree`） | `wait()` 失败（例如 ECHILD，子进程已在别处被回收）时仍然执行 `killpg(-pid, SIGKILL)`。子进程已被回收，进程组号可能已被系统复用，存在误杀无关进程组的理论风险。 | 低 | 与 B5（wait 失败不 panic）相关，属于边界补充 |
| X6 | `src/tools/custom.rs:395-406`（`spawn_stdin_writer`）；SIGPIPE | EPIPE 被视为 Ok，正确。Rust 运行时默认忽略 SIGPIPE，所以写入不会因 SIGPIPE 让 litecode 退出。测试 `child_ignoring_stdin_is_not_a_broken_pipe_failure` 覆盖了这一点。**没问题** | — | 认可 Tools 的修复 |
| X7 | `src/tools/custom.rs:372-381`（Cancelled/TimedOut） | 先对组 SIGKILL，再 `child.wait()` 回收，顺序正确，不会留僵尸。读线程和 writer 线程被分离，正常情况下会在组被杀后拿到 EOF/EPIPE 退出。如果有组外进程（自己 setsid 的）持有管道，这些线程会泄漏，与 X1 同源 | 低 | 认可，附注 |

后台 job 生命周期：`start_background_with`（210-270 行）在 spawn 失败时 `abandon_job`，正确。job 的 cancel 来自 `hub.begin_job`，turn-cancel 能传到 worker，正确。没有 Drop 钩子，进程退出时依赖 X4。

结论：发现 7 条，其中问题 5 条（中 3：X1、X2、X4；低 2：X3、X5），另有 2 条（X6、X7）为认可项。最严重的是 X1+X2：孙进程持有管道时，超时和取消都失效，进程也不会被清理。**与 Tools 无分歧**：X1 是对 B7 的补充，其余为新增。建议 X1、X2 合并修（父进程退出后对组 kill + 读取有界等待），X4 接入 `bind_child_to_parent`。建议的复现命令（未跑）：自定义工具 `sh -c 'sleep 30 & echo done'`，timeout 设为 2，观察调用耗时约 30s，而不是约 0s 或 2s。

—— IDE 基座

---
## Tools 对 IDE 交叉审（X1–X7）的回应（2026-10-12 16:1x HKT，不改代码）
| # | 结论 | 理由 / 去向 |
|---|---|---|
|X1|**接受**，登记 ISSUE-039（并入 B7）|属实：Ok(Some) 后 rx.recv() 不受 deadline/cancel 约束，孙进程持 stdout 即挂住；B7 原登记只说"无界"，IDE 补了触发条件与超时失效。修复方向：recv_timeout 受剩余 deadline 约束，超时后组 kill 再短等|
|X2|**接受**，并入 ISSUE-039|Finished 路径确实不 kill 组；直接子进程已退出后组内只剩遗留进程，对组 SIGKILL 安全（组号仍由仍存活的组员占用，不会复用）。与 X1 一起修|
|X3|**接受（低）**，并入 ISSUE-039|writer join 无界属实，需大输入+孙进程继承 stdin 不读才触发；与 X1 同一修法（Finished 后组 kill 使写端 EPIPE，再有界 join）|
|X4|**接受**，登记 ISSUE-040（drift）|LSP/code_search 已用 bind_child_to_parent，custom 未接入，Windows 宿主崩溃不级联。注意 Unix 下 process_group(0) 会脱离终端 SIGHUP，但该函数 Unix 为 no-op，故主要影响 Windows|
|X5|**部分反驳，不登记**|WaitFailed 仅在 try_wait 与 wait 都失败时出现（ECHILD = 已被他处回收）；此时 pid 复用为新组长需在极短窗口内 pid 回绕，实际风险极低。但认可"已回收后不应 killpg"更干净：ISSUE-039 修复时顺手把 WaitFailed 分支改为不 kill，不单独开 issue|
|X6|**确认**|与本包设计一致，无需动作|
|X7|**确认**|组外（setsid）进程持管道导致线程泄漏归 ISSUE-039 一并处理|
后台 job 生命周期段（abandon_job/turn-cancel 传播）：同意。

## lsp 集成测试 agent_lsp_tool_line_text_scenarios 结论
- 位置 tests/optional_lsp.rs（Tools 域：测 src/tools/lsp.rs 的 LSP 工具输出）。
- 判定：**测试过时，非实现回归**。证据：commit b4f04e7（2026-09-04 13:08 HKT，"refactor(output): streamline line formatting…"）把 lsp.rs numbered_line_window 从 `{n:>4}|line` 改为统一的 `tool::format_file_line`（src/tool/coords.rs:17，`{n:6}: text`），属有意统一各工具行号格式；测试仍断言 "```" 或 "|"。
- 改动：tests/optional_lsp.rs findReferences(single) 断言改为"存在 `<数字>: ` 行号上下文行"并注明来源 commit。其余断言未动。
- 复跑：agent_lsp_tool_line_text_scenarios 1 passed（结果见下行）。
- 复跑结果（16:10 HKT）：test result: ok. 1 passed; 0 failed。
