# WP-022 / WP-028 — IDE 基座：可疑实现 + 测试裁剪清单

- HEAD：`c6b24df`（分支 ops/team-framework），日期：2026-10-12（HKT）
- 执行：ide；审批：backend_mid。本包只审不修，无 commit，未改产品代码和测试。
- 工具链：rustc 1.99.0。测试二进制由 `cargo test --no-run --tests` 编译，15:12 HKT 完成。之后通过 `cargo test --test X` 跑测试时，因为其他域在共用的 target 目录里同时编译，全部超时（rc=124）。所以实测改为直接执行已编译好的二进制：`CARGO_MANIFEST_DIR=$PWD HOME=$PWD/.tmp/home timeout 120 target/debug/deps/<test>-<hash> </dev/null`。
- 禁区：引擎产品形状（ISSUE-023..026）、symlink 复制（ISSUE-010/WP-004），均未分析。
- 工作区现状（只记录，不处置）：
  - `src/workspace/service.rs` 的未提交 diff 只是 `refuse_top_level_symlink_copy` 函数签名的 rustfmt 换行，正好 100 列。按方案保留，本包不动。
  - `src/engines/code_search/embed.rs` 的未提交 diff 只是测试模块约 359 行一个 `assert!` 的 rustfmt 拆行。冻结，只记录。

## 1. 可疑实现表

| # | file:line（符号） | 问题 | 严重度 | 依据 | 建议分级 |
|---|---|---|---|---|---|
| S1 | `src/workspace/service.rs:347-357`（`rename_path` 的 rename 失败分支）；原报的 `:490-512`（`move_via_copy`）已推翻 | **原假设推翻**：dest 已存在时，`refuse_directory_overwrite`（454 行）挡住目录；文件覆盖走 `is_file_replace` → `replace_file`（同样是 rename），所以 `move_via_copy` 只会在 dest 不存在时进入，cleanup 删的只是半成品，**不会删掉原有目标**。**探针却实测出另一个问题**：跨设备 + `overwrite=true` + 目标是已有文件时，`replace_file` 再次失败（EXDEV），因为 cross-device 分支排在 file-replace 分支之后，永远走不到 copy 回退。结果是移动直接报 `Io(CrossesDevices)` 失败，原目标和源都完好，不丢数据但功能不可用。 | 低（不丢数据） | **实测**：`tests/wp028_probe_s1_xdev_move.rs` 输出 `rename_path result=Err(Io(Os { code: 18, kind: CrossesDevices ... })) dst=Some("ORIGINAL") src=Some("NEW")`。场景构造：workspace 根设为 `/`，源在 `/dev/shm`（tmpfs），目标在仓库 `.tmp`（ext4） | bug → **ISSUE-038**（修复另开包） |
| S2 | `src/workspace/service.rs:270-287`（`create_file`）；`:404-433`（`write_file_bytes`，`overwrite=false`） | 先 `exists()` 检查、再 `atomic_write`（rename 覆盖）。同一路径的并发 create 会同时返回成功，后写者静默覆盖先写者，违反“已存在则 AlreadyExists”的承诺。 | 中 | **实测**：`tests/wp028_probe_s2_create_race.rs`，每轮 8 线程 × 300 轮。`create_file`：300/300 轮出现多个成功；`write_file_bytes(overwrite=false)`：300/300 轮出现多个成功 | **bug → ISSUE-037**（修复另开包） |
| S3 | `src/lsp/server.rs:771-776`（`LspServer::kill_child`）；被 `src/lsp/hub.rs:959-985`（`Drop for LspHub`，runtime 内路径）调用 | `child.try_lock()` 拿不到锁时静默跳过 kill，之后也不 wait。runtime 是 `ManuallyDrop`（泄漏），进程可能残留。 | 中 | 读码推理。**探针无法构造**：`LspServer` 的 `child` 字段、`kill_child`、`is_process_alive`、`spawn` 都是 `pub(crate)`；公共 API（`LspHub`）也无法在 drop 的瞬间稳定持有 child 锁。不改产品代码、不用 `#[cfg(test)]` 就触不到。要复现，需要在 crate 内加单元测试，或者给 hub 加一个测试钩子（本包只审不修，未做） | suspect |
| S4 | `src/lsp/server.rs:137-170`（`LspServer::spawn`，没有 `process_group`/setsid） | 只 kill 直接子进程。rust-analyzer 的 `proc-macro-srv` 等孙进程只能靠 stdin EOF 自行退出，卡住时会成为孤儿。与 S5 叠加时，Linux 上可能清扫不到。 | 低 | 读码推理 | suspect |
| S5 | `src/proc_lifetime.rs:244-265`（`sweep_once`，Linux） | 注释说孤儿会被重挂到 “init（或 subreaper）”，但代码只认 `ppid == 1`。在 systemd --user、容器 init、tini 等 subreaper 下，孤儿的 ppid ≠ 1，永远清扫不到。注释和实现不一致。 | 低 | 读码推理 | drift（注释/契约与实现不符） |
| S6 | `src/engines/code_search_ipc/client.rs:226-230`（`Drop for CodeSearchWorkerClient`） | Drop 只 `kill()` 不 `wait()`。`std::process::Child` 被丢弃后没人回收，直到父进程退出都会留下僵尸。显式的 `kill()`（213 行）会 wait，但 Drop 路径不会。这一条只看子进程生命周期，不涉及引擎产品形状。 | 低 | 读码推理 | suspect |
| S7 | `src/workspace/service.rs:550-554`（`copy_recursive` 的覆盖分支 tmp 名） | tmp 名只用 `pid` 区分。同一进程内两个并发复制到同一个 dst 时会共用 tmp，可能交叉写入或误删。注：这里是普通文件覆盖，不是 symlink 复制。 | 低 | 读码推理 | suspect |

已排除（核实后不报）：
- terminal kill 后只做一次 `try_reap`（`src/terminal/mod.rs:195/451/778`）：每个会话都有 `start_reaper` 线程（`mod.rs:244`），持有 `Arc<PtySession>` 并循环回收；最后 `Drop for PtySession`（`pty.rs:278`）里的 `wait_join` 会兜底。进程组先 kill 的顺序（`pty.rs:30-50`）也正确。不报。
- `lsp/deps.rs:466` 的 `wait_with_timeout`：超时后先 kill 再 wait，正确。

## 2. 测试清单（只列不删）

| # | 测试 | 文件 | 类别 | 理由 / 实测 | 建议动作 |
|---|---|---|---|---|---|
| T1 | `death_list_forbidden_hot_switch_symbols` | tests/workspace_hotswitch_death_list.rs | 可放宽 | **实测 FAILED**：命中 8 处，都是子串误报，不是根热切换回归。命中分布：`reload_workspace` 6 处（filter/workspace_excludes、watcher、runtime、config/gate、code_search_ipc/server）、`post_open` 1 处（workspace/routes.rs）、`api/workspace/open` 1 处（web/src/api/workspace.ts）。这就是 ISSUE-007 的问题，而且范围比 ISSUE-007 记录的更大（又多了 config/gate 和 ipc/server）。清单本身的意图（禁止根热切换）仍然成立。 | 把 FORBIDDEN 改成词边界或精确符号匹配，`post_open` 和 `api/workspace/open` 改成精确路由匹配；已在 ISSUE-007 追加 8 处证据（WP-028 第二轮）。**不删** |
| T2 | `death_list_src_canonicalize_whitelist_is_only_path_rs` | tests/workspace_path_death_list.rs | 重复（弱） | 实测 ok。它只断言 path.rs 里还有 canonicalize 调用，属于自检，`no_bare_canonicalize` 已经覆盖主承诺。但它能防止“正则失效导致清单空转”，有保留价值。 | 保留；可以合并成 `no_bare_canonicalize` 里的一条前置断言 |
| T3 | `workspace_open_http_endpoint_is_gone` | tests/workspace_process_contract.rs | 重复 | 实测 ok。它和 T1 都在守“`/api/workspace/open` 已删除”：T1 用子串扫描，T3 用 HTTP 行为验证。T3 更准。 | T1 去掉 `api/workspace/open` 这一项，交给 T3 守；T3 保留 |
| T4 | 三个 node-pty 清单（`death_list_no_node_pty_*`、`death_list_src_pty_symbols_only_in_terminal_module`） | tests/terminal_bypass_death_list.rs | 保留 | 实测 3/3 ok。承诺仍然有效（PTY 只在 terminal 模块里），没有发现过时依据。 | 不动 |
| T5 | `workspace_path_consumer.rs` 和 `workspace_path_lap.rs` 中的 `rel_path_under_*` 用例（`rel_path_under_accepts_raw_canonicalize_form` / `rel_path_under_uses_lap_compare` / `rel_path_ctx_matches_rel_path_under_and_rejects_outside`） | 两个文件 | 重复（疑） | 三条都围绕 `rel_path_under` 的 LAP 比较；`windows_*` 用例在 Linux 上走的分支有限。未实测（cargo 被锁）。 | 核对断言后合并到 workspace_path_lap.rs |
| T6 | `workspace_file_crud` / `workspace_mkdir_rename_copy_blob` | tests/workspace_api.rs | 保留（补缺） | 与 `service.rs` 单元测试 `mkdir_rename_copy_and_reject_self_copy`、`refuses_directory_overwrite_and_renames_directories` 部分重叠，但 API 层另外覆盖了路由和状态码。两边都没有覆盖 S1/S2 的失败路径和并发。 | 保留；建议另起 WP 补 S1/S2 的测试，本包不加 |

## 1b. S1 原假设「会删掉原有目标」已推翻（第三轮裁定）

推翻依据（读码 + 探针）：
1. `rename_path`（service.rs:316-360）在 rename 之前先调用 `refuse_directory_overwrite`（454-474 行）。只要 dest 已存在：任一侧是目录就直接 `InvalidMove`；文件对文件且 `overwrite=false` 时返回 `AlreadyExists`。
2. 文件对文件且 `overwrite=true` 时，rename 失败后先匹配 `dest_existed && overwrite && is_file_replace` 分支（351 行），走 `replace_file`（Unix 上仍是 `rename`），**不会进入** `move_via_copy`。
3. 所以 `move_via_copy` 只可能在 dest 原本不存在时被调用，它的 `cleanup_incomplete_dest` 删掉的只能是自己复制出的半成品。
4. 探针实测（跨盘、覆盖已有文件）：结果是 `Err(Io(CrossesDevices))`，`dst=Some("ORIGINAL")`，原目标完好。由此另立 ISSUE-038。

新 issue：**ISSUE-037**（S2 并发 create 覆盖，bug，中）、**ISSUE-038**（S1 跨盘覆盖移动失败，bug，低），均写明“修复另开包”。两个探针都已加 `#[ignore = "WP-028 probe: ISSUE-0xx"]`。S1 探针已改为断言“跨盘覆盖应成功”。S3 保持 suspect，不加测试钩子。

### 第三轮实跑（探针，加 ignore 后，16:0x HKT）
编译：`PATH=/home/box/.cargo/bin:$PATH HOME=… timeout 3000 cargo test --no-run --test wp028_probe_s1_xdev_move --test wp028_probe_s2_create_race`，期间排队等 target 锁，EXIT 0。运行时直接执行二进制。
- 默认运行：s1 → `ok. 0 passed; 1 ignored`（ignored, WP-028 probe: ISSUE-038）；s2 → `ok. 0 passed; 2 ignored`（ISSUE-037）。
- `--ignored`：s1 FAILED，输出 `rename_path result=Err(Io(Os { code: 18, kind: CrossesDevices ...})) dst=Some("ORIGINAL") src=Some("NEW")`；s2 FAILED 2/2，两个 API 都是 `rounds_with_multiple_successes=300`（300 轮）。

## 2b. 补跑四个套件（WP-028 第二轮，timeout 600）

编译：`PATH=/home/box/.cargo/bin:$PATH HOME=/workspace/litecode/.tmp/home timeout 3600 cargo test --no-run --test optional_lsp --test workspace_api --test workspace_path_consumer --test workspace_path_lap --test wp028_probe_s1_xdev_move --test wp028_probe_s2_create_race </dev/null`。编译在后台跑，期间排队等共享 target 锁（`Blocking waiting for file lock`），没有删锁。约 15:47 HKT 产出二进制，EXIT 0。
运行：`CARGO_MANIFEST_DIR=$PWD HOME=$PWD/.tmp/home timeout 600 target/debug/deps/<bin> --nocapture </dev/null`，约 16:00 HKT。

**源码状态（编译时）**：HEAD `c6b24df`，工作区有 90 行 `git status`，均为其他域或以前留下的未提交改动。src/tests 下的改动包括：cli、client_protocol/*、config/{manager,schema,settings_writer}、context_pipeline/compact、engines/code_search/embed（fmt）、knowledge/*、optional/mod、permission/*、reminder/*、runtime/{exec,mod,stream_projection}、session/*、tool/*、tools/{ask_user,bash_jobs_contract,custom,custom_hub,knowledge,litecode_workspace,mod,plan,webfetch,websearch}、types/death_list_gate、workspace/service（fmt）、tests/{client_protocol_*,common/*,compact_failure_calibers,permission_pipeline,stage_a_ctx_consistency,manual_compact_e2e}、新增 tests/wp028_probe_*。快照见 `.tmp/wp028_srcstate_build.txt`。
**编译后、运行前又被其他域改动**（未进入本批二进制）：`src/agent/core.rs`（15:44，mtime 早于二进制，可能已编进去）、`src/tools/lsp.rs`（15:51，**未编进去**）、`tests/agent_loop.rs`、ISSUE-036 新增。快照见 `.tmp/wp028_srcstate_run.txt`。

| 套件 | 结果 | 备注 |
|---|---|---|
| optional_lsp | **FAILED**：23 passed / 1 failed / 1 ignored（22.9s） | 失败用例 `agent_lsp_tool_line_text_scenarios`（tests/optional_lsp.rs:1062）：断言 refs 输出要含 ``` 或 `\|`，实际是“编号行”格式。输出由 `src/tools/lsp.rs`（Tools 域）生成，而编译时该文件处于 HEAD 状态，所以是 HEAD 上的断言或输出格式漂移，不是 IDE 基座的问题。Tools 正在改 `src/tools/lsp.rs`，等它就绪后需要重跑确认，交 Tools 判定 |
| workspace_api | ok 12/12（1.25s） | |
| workspace_path_consumer | ok 4/4 | |
| workspace_path_lap | ok 5/5 | |

**15:12 那批二进制的结果（另行标注）**：它来自第一轮 `cargo test --no-run --tests`，编译时的源码状态没有留快照，只知道 HEAD 是 `c6b24df`、工作区当时已有其他域的未提交改动。结果：workspace_hotswitch_death_list FAILED（8 处误报，见 T1）；terminal_bypass_death_list ok 3/3；workspace_path_death_list ok 4/4；workspace_process_contract ok 7/7。这四个本轮没有重编。

## 3. 待实测 / 未完成
- 四个套件已在 2b 节补跑。optional_lsp 的那一条失败要等 Tools 改完 `src/tools/lsp.rs` 后重跑。
- S3 无法用公共 API 构造探针，仍为 suspect（原因见表）。S4–S7 按裁定只保留读码证据。
- 探针（只加测试，没改产品）：`tests/wp028_probe_s1_xdev_move.rs`（ISSUE-038）、`tests/wp028_probe_s2_create_race.rs`（ISSUE-037），均带 `#[ignore]`；默认运行显示 ignored，加 `-- --ignored` 时复现失败（见 1b 和第三轮实跑记录）。

## 统计与优先级
- 可疑项共 7 条（第二、三轮修订；S2=ISSUE-037，S1=ISSUE-038）：中 2 条（S2、S3），低 5 条（S1、S4、S5、S6、S7）；高 0 条。分级：bug 2 条（S2，以及修订后的 S1：跨设备覆盖移动失败），suspect 4 条（S3、S4、S6、S7），drift 1 条（S5）。S1 的“删掉原目标”假设被读码和探针共同推翻。
- 测试清单 6 条：可放宽 1、重复 3（T2、T3、T5）、保留 2（T4、T6）。实测：4 个二进制共 15 条用例，1 条 FAIL（T1，即 ISSUE-007）。
- 建议优先级：S2（并发 create 静默覆盖，已复现）> T1/ISSUE-007（门禁持续红，已补证）> S3（要做 crate 内复现）> S1（跨设备覆盖移动失败）> S5/S4 > S6/S7。

## 互审（ide → Tools：进程生命周期线索）
1. LSP 关闭：`kill_child` 的 try_lock 会静默跳过，而且不 wait（S3）；没有进程组，孙进程可能成为孤儿（S4）。请 Tools 看工具调用路径上 Hub 被 drop 的时机是否会落在持锁期间。
2. Linux 孤儿清扫只认 ppid==1（S5）。
3. code_search worker 的 Drop 不 wait（S6）。
4. Broken pipe：`code_search_engine.rs:504` 已经把 BrokenPipe 视为 worker 死亡；`tools/custom.rs:364-376` 写 stdin 时吞掉 EPIPE。LSP `send_notification`（shutdown/exit 时）和 PTY writer 对 EPIPE 怎么处理还没核实。Rust 运行时默认忽略 SIGPIPE，所以进程不会因此退出，但需要确认错误是否被吞掉或误报。请 Tools 核对。

## 交叉审（Tools 填写）

（Tools，2026-10-12 16:1x HKT，只读核对）
- **workspace/service.rs**：本次 diff 只有 `refuse_top_level_symlink_copy` 签名 rustfmt 合并成一行，无语义改动，不涉及进程生命周期。**同意**。
- **terminal/pty.rs:30-50 kill_process_group**：unix `kill(-pid)`，Windows `taskkill /T /F`，再用 `child.kill()` 兜底；PTY 子进程是会话首进程，组杀有效。**同意**。
- **pty.rs:248-276 wait_join**：kill 后最多轮询 reap 100×20ms，reader join 最多 150×20ms，超时就 detach 并 warn。有界，不会挂住。**同意**。
- **pty.rs:278 Drop → wait_join**：所有关闭路径都会 reap。**同意**；附注：Drop 最多阻塞约 5s，如果最后一个 Arc 在 tokio worker 上释放，会占住 worker，建议确认释放点在 spawn_blocking 或专用线程上。
- **terminal/mod.rs:195-196、:451-453（原疑问：是否有僵尸进程）**：核对后结论是**不会留僵尸，但退出码会丢**。`kill()` 发出 SIGKILL 是异步的，紧接着调用一次 `try_reap()` 多半还拿不到状态，所以 :453 的 `exit_code()` 大概率是 None，exit 事件不带退出码。进程本身由 entry 释放时 PtySession::Drop→wait_join 回收（有界轮询），前提是没有别的地方持有这个 Arc。等级：低。建议 :196/:452 改成有界 reap（复用 wait_join 的轮询），或接受 None 并在协议里注明。
- **对 ide 互审第 4 条（Broken pipe）的回应**：custom.rs 吞 EPIPE 是有意的（stdin 没读完由退出码判定，见 WP-022-tools.md A 节和测试 child_ignoring_stdin_is_not_a_broken_pipe_failure）。pty.rs:190 `write_all` 把错误返回给调用方，不吞，合理；pty.rs:70 的 DSR 应答 `let _ =` 忽略，可以接受。LSP `send_notification`（lsp/server.rs:433）直接透传 `io.notify` 的错误，shutdown/exit 时如果服务器已退出会报 EPIPE 错误，建议在 shutdown 路径把 BrokenPipe 视为已退出，属 IDE 域（低）。
- 第 1–3 条（LSP kill_child try_lock、ppid==1 清扫、code_search Drop 不 wait）属 IDE 域，Tools 无异议；Hub 被 drop 的时机：工具调用路径上 Tools 不持有 LSP Hub 锁跨 await，未见冲突（未实跑）。
