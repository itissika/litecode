# WP-212 plan — run_code 收尾瑕疵
基线 b5d9e51（feat/tool-contract-ptc，不合 main）。不碰 desktop/build/icon.ico、tool_traces.jsonl。范围仅 brief 五刀，**不实现定时器**，不扩 run 形态。

## Andy 已定（原样）
1. **kill 假成功**：仅确认进程已死后 `ok:true`；等退出超时/仍活着 → **`ok:false`**；Windows 走 `taskkill /T`；`wait` 明确区分「被 kill」与普通 `exit:1`（结构化字段或可解析 message，见 Q2）。
2. **未处理 Promise rejection**：最低交付为 settle 时打一条 **`[warn]`**，勿再静默；本包**不做 abort**（Andy 已确认先 warn）。
3. **语法**：`1 host call was still running`（单数）；复数再加 s。
4. **文档**：写清未知参数（unknown parameter）**整调用失败**，不是多余字段静默丢弃；可选一句沙箱暂无 `setTimeout`/`setInterval`（本包不实现）。
5. **可选**：栈多一帧越界 `<eval>`——便宜则压；否则 acceptance 注明跳过。

## 定位（只读 @b5d9e51）与分域
| 刀 | 位置 | 域 |
|---|---|---|
| ① kill 假成功 | `src/tools/kill.rs:21-97`：`REAP_WAIT=2s`，`observed_exit` 超时后仍可 `None`，`present`→`present_command` **一律成功**（仅挂 note `process was killed`）；`kill_registered` 已返回 `KilledJob.alive`（`src/terminal/agent_jobs.rs:186-191,679-706`）但 kill 工具未据此判失败。Windows 树杀已在 `kill_process_tree`（`:67-84`）走 `taskkill /PID … /T /F`。呈现：`src/tools/command_job.rs:241-266` `present_command`（除 pathconv 外无 `ok:false`）。文档：`descriptions/kill.md` | **Tools** |
| ① wait 区分 kill | `src/tools/wait.rs:99-105`：`WaitOutcome::Exited(notice)` 只传 `exit_code`，**不读** `ExitNotice.user_killed`（`agent_jobs.rs:95`）。`tools.kill` 故意 `kill_registered(..., false)`（`kill.rs:88-91`）以免人机提醒；`take_notice` 吃掉 mailbox。需在「工具杀」路径留下可观测标记且不破坏「不发 natural-exit reminder」 | **Tools** |
| ② unhandled rejection | 程序包成 `(async () => { source })()` 后 `promise.into_future`（`src/ptc/engine.rs:997-1044`）；浮动 `Promise.reject` 不驳外层。文档仍写 ends the program（`code_manifest.rs:24`）。rquickjs 有 `AsyncRuntime::set_host_promise_rejection_tracker`（RejectionTracker: `Fn(Ctx, Value, Value, bool)`，`is_handled`）。settle/收 logs 点：`engine.rs:331-356` | **Runtime** |
| ③ host-call 语法 | `src/ptc/engine.rs:333-336` 文案固定复数 `host calls were`；断言：`src/ptc/tests.rs:646,772` 含 `"1 host calls were still running"` | **Runtime** |
| ④ 文档 | PRELUDE `src/tool/code_manifest.rs:20` 已写「extra argument … throws TypeError」，但未点名 **unknown parameter 整调用失败 / 不静默丢**；L24 rejection 文案与实现不符，本包随 ② 改正。Code 面校验：`src/tools/code.rs` / schema_validate `unknown_parameter`；Chat 面 `executor.rs:213-218` 仍有「ignored unknown parameter(s)」——**本包不改 Chat 面**，只澄清 Code/SDK。无 timer 可选注进 PRELUDE。fixture：`src/tool/fixtures/run_code_sdk.txt` | **Tools** |
| ⑤ `<eval>` 帧 | 模型视图 `short_stack`（`src/tools/code/render.rs:920-943`）已丢 `program.js` **line≤1**（覆盖常见 `at <eval> (program.js:1:1)`，单测 `failed_run_keeps_…`）。若仍见越界帧：再按名丢 `at <eval>`（便宜）。不改 wrapper、不加定时器 | **Tools**（render；与 WP-209/210 同文件） |

分域：Tools ①④⑤；Runtime ②③。包 **kind=direct**，我统筹；两域可阶段 1 并行（文件不重叠）。

## 串并行
- **阶段 1a Tools**：① kill 确认死后才 ok；wait 区分被杀；kill/wait 文档；⑤ short_stack 按名丢 `<eval>`（若 Q7 做）。
- **阶段 1b Runtime**（可并行）：② rejection tracker → settle `[warn]`；③ 单复数文案 + 改 ptc 测试断言。
- **阶段 2 Tools**：PRELUDE/④ 文案（含 rejection 与实现对齐、unknown param、可选 no-timer）+ 刷 `run_code_sdk.txt`/`.sha256`。
- **阶段 3**：我写 acceptance（protected 断言变更逐条报备）、cargo-full、提交推送。

## 实现要点（推荐口径，待 Q 批）
### ① kill
- `run` 在 `observed_exit` 后：有 `exit_code` **或** `!pid_alive(pid)` → 确认死，`ok:true` + 现有 note；否则 **`ok:false`**、`exit_code: null`、`reason` 推荐 `still_alive`（见 Q1），message 说明 reap 超时进程仍活；**禁止**假成功。
- 呈现：沿用 WP-211 pathconv 模式——`CommandData.ok=Some(false)` + reason，走 `ToolOutcome::ok` 以便保留 id/output/stderr/shell（见 Q1）。
- Windows：确认仍只经 `kill_process_tree`→`taskkill /T /F`；不另开杀路径。真机由 Andy 本地验（CI 无 Windows）。
### ① wait
- 推荐：工具杀路径在 registry 记 **`tool_killed`**（或复用/扩展 notice 标记），**不**设 `user_killed`（保持不发 human reminder + 现有 `take_notice`）。
- wait 在 `Exited` 且标记为杀时：输出可解析 note **`[process was killed]`**（与 kill 成功 note 对齐），并带结构化可选字段 `killed: true`（见 Q2 二选一；推荐 **两者都给**：字段为主、note 兼容）。
- 自然 `exit:1`：无 `killed`、无该 note。
### ② rejection
- runtime 安装 `set_host_promise_rejection_tracker`：`is_handled=false` 记账，`true` 撤销。
- 程序 settle 收 logs 前：若仍有未处理，push **一条** `[warn]`（文案见 Q3）；**不**改 `ProgramEnd`、不 abort。
- PRELUDE L24 改为与实现一致（warn，不结束程序）。
### ③ 语法
- `pending==1` → `1 host call was still running when the program returned and have been cancelled`（后半是否改 `has` 见 Q4）；`pending!=1` 保持复数 were。
### ④ 文档
- PRELUDE：明确 **unknown parameter / extra argument → 整调用 TypeError 失败，字段绝不静默丢弃**（Code 面）；可选一句无 `setTimeout`/`setInterval`。
- kill.md：写明未确认死亡时 `ok:false`；wait.md：写明被 kill 的区分方式。
### ⑤ `<eval>`
- 在 `short_stack` 增加：帧文本含 `at <eval>` 则跳过（与 line≤1 叠加）。若审代码认为已足够，可跳过改动并在 acceptance 写「line≤1 已压」。

## cargo
一次只一个重任务，`-j 2`，每次前 `free` 确认 available ≥2GiB。顺序：
1. `cargo test -j 2 --lib tools::kill tools::wait tools::command_job`
2. `cargo test -j 2 --lib ptc::`
3. `cargo test -j 2 --lib tools::code::render tool::code_manifest`
4. `cargo test -j 2 --lib sdk_snapshot`（若有）
5. `cargo test -j 2 --tests --no-run`
6. tip 全量 `cargo test -j 2` → `artifacts/WP-212/cargo-full.txt`

## 测试（仅核心，入 protected.txt）
约 4–6 条，不堆量：
1. ① 杀短命 sleep：确认死后 `ok:true` + note（改/补 `kill_stops_process_and_notes_output`）。
2. ① 假死/超时夹具：mock 或难杀场景 → `ok:false` + `exit_code` null（见 Q5；Linux 可用卡住的子进程或注入 alive 探测失败路径）。
3. ① wait：工具杀后的 Exited 带 `killed`/note；自然 `exit 1` 不带。
4. ② `Promise.reject('x')` 无 await：程序仍 Success，logs 含 `[warn]` 且文案点出 unhandled rejection。
5. ③ pending=1 日志为单数 `1 host call was`（改现有两条断言）。
6. ⑤（若改 short_stack）夹具含 `<eval>` 帧 → 模型视图 stack 不含 `<eval>`。

SDK 快照随 PRELUDE 变绿。protected 断言变更逐条报备。

## 验收
brief 五条 + cargo 全绿 + PRELUDE/SDK/kill·wait 文档一致 + protected 报备。push 实验分支，不合 main。Windows taskkill / 真杀由 Andy 本地点验。

## Q 列表（附推荐）
Q1 kill 未确认死亡：`ok:false` + `reason:"still_alive"` + `exit_code:null`，仍返回 id/output/stderr/shell（pathconv 同款显式 ok）？推荐：**是**。
Q2 wait 区分被杀：结构化 `killed:true` **且** output note `[process was killed]`，自然退出两者皆无？推荐：**是（字段为主）**。若只留一个：留字段。
Q3 warn 文案：`unhandled promise rejection (N): <first message>`（N≥1，message 截断）？推荐：**是**；单条 warn，不刷屏。
Q4 单数句后半：`… and have been cancelled` 是否改 `has been cancelled`？推荐：**改 has**（语法一并修）。
Q5 「仍活着」单测：不依赖 Windows；用可控夹具（延长 REAP 探测失败 / 注册假 pid 或 stub `pid_alive` 测呈现分支）？推荐：**是**（呈现/分支单测 + 一条真实 sleep 杀成功）。
Q6 工具杀标记：新增 `tool_killed`（不设 `user_killed`），避免 mailbox 人机提醒回潮？推荐：**是**。
Q7 `<eval>`：在 `short_stack` 按名再丢一刀？推荐：**是**（便宜；line≤1 已有）。
Q8 PRELUDE 是否改 L24「ends the program」→ warn 语义？推荐：**是**（文档必须跟实现；abort 另包）。
Q9 无定时器：PRELUDE 加一句「沙箱无 `setTimeout`/`setInterval`」？推荐：**是**（brief 可选；一行）。
Q10 Chat 面 `ignored unknown parameter(s)` 本包不动？推荐：**不动**（范围外；只钉 Code/SDK）。

## 风险
- ① 语义面：依赖 kill 假成功 / 仅看 exit_code 的调用方会变；全量 + protected 报备。
- ① Windows 真机与 taskkill /T 仍靠 Andy；Linux CI 只能验 Unix 树杀 + 呈现分支。
- ② rejection tracker 与 AsyncRuntime 生命周期/线程：须挂在跑 program 的 runtime 上，避免跨 await 丢计数；误计已 handled 的 rejection → 单测钉。
- ③/④ 文案与 SDK fixture 必刷；勿碰 Chat 面 unknown-param 警告。
- ⑤ 若只改 short_stack，原始 `failure.stack` 仍可能含 `<eval>`（仅模型视图变干净）——acceptance 写明。
- 工作区 `tool_traces.jsonl` 有本地脏行，提交不碰。
