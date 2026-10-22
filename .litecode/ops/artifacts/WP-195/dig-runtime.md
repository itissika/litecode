# WP-195 Runtime dig：bash / kill_shell / wait_shell 能否进入 Code

> **作者：** Runtime（Grok Bot 会话自动路由；无 picker）  
> **产出：** 2026-10-21 Asia/Hong_Kong；本会话重核并重写（不用 CloudAgent）  
> **分支 / HEAD：** `feat/tool-contract-ptc` @ `bd71f8c`  
> **对照：** `brief.md`、`dig-mid-notes.md`、章程/整改/改造计划、`src/ptc/**` + tool 宿主；证据行号已对本 HEAD 复核

## 总评

**2/10。倾向：不进。**

三件套保持 Direct。只改 `surface()` 过不了准入；即便绕过准入，今日「成功=纯文本、无 `data`」也会在 Code 路径变成内部错误。权限/审批管线可以复用（I5/D7/I8），但作业寿命、同步 `jobs.wait` 堵 current-thread、以及结果契约，与 `run_code` 的预算、封口、unsettled、64KB 视口对不上。下面的「有条件」是**另一份工具契约**的门槛，不是给现有三件套加开关。

| 项 | 分 | 含义 |
|---|---|---|
| 按现状进入 Code | 2/10 | 无 `data` schema；成功进 Code → 内部错误 |
| 权限 / 审批沿 I5、D7、I8 | 8/10 | 嵌套已走同一条 `execute_tool_call` |
| 与程序预算、封口、unsettled 共存 | 1/10 | 前台同步等待饿死预算 watcher |
| 与文件工具同程序编排 | 收益真实，现状拿不到 | 先要稳定 `data` + 非阻塞等待 |

## 背景

- mid 盘点：`bash` / `kill_shell` / `wait_shell` 皆 Direct（未覆盖 `surface()`，默认 `ToolSurface::Direct`，`src/tool/trait_.rs:49-50`）。工作树 artifacts 有 `WP-195/{brief,dig-mid-notes}.md`（多为未跟踪）；本 dig 覆盖写本文件。
- 章程 D2：首期 code 工具集不含 bash 系列（`docs/ptc/charter.md:22`）；本轮不做 bash/subagent code 化（`:60`）。整改计划范围排除 bash code 化；改造计划 D-11 推荐否、独立项目（`docs/ptc/code-tool-rework-plan.md:615`、`:711`）。
- 结论与文档一致，但依据是**运行时行为**复核，不是复述文档。

## 1. 现状：三件套 + Direct 作业协议

- **`bash`**：单次命令；cwd/环境不跨调用保留。`timeout` 秒 1–600，默认 30；到点**不杀**，改报 `status: running` 留后台作业。`run_in_background` 立即返回 `bash_id`（`src/tools/bash.rs`；说明 `descriptions/bash.md`）。
- **`wait_shell`**：按 `bash_id` 和/或 `sec`（1–600）等；不杀进程。
- **`kill_shell`**：按 `bash_id` 终止；输出文件保留。

投影一面互斥：`ToolCaller::Code` 只能调 `surface() == Code`；Direct 在程序内拒绝，文案 `` `{name}` is a direct tool and is not available inside run_code ``（`src/tool/executor.rs:830-855`）。`project_tools` 把 Code 工具从 direct 列表拿掉，准入失败则两面都拿掉（`src/tool/project.rs:38-60`；组装见 `src/tool/registry.rs` 调 `project_tools`）。D1 禁止双面（`docs/ptc/charter.md:20`）——要进就得三件套一起进，跨回合 wait/kill 也只能再进 `run_code`。

成功结果是纯文本：`ToolCallResult::ok` 固定 `data: None`（`src/types/result.rs:83-86`）。非零退出仍是 `ok`，退出码写在正文。取消走 `error`/`Internal`，不是 `Cancelled`（`bash` / `wait_shell`）。

Code 成功路径要求 `data` 且过 `data_schema`；否则内部错误（`src/tool/executor.rs:884-906`）。准入要求非空 `data` schema（`src/tool/code_manifest.rs:119-131`）。三者皆无 schema →

1. 只改 `surface()`：准入失败，两面消失，SDK 不出现该工具。
2. 若仍 `ok(文本)`：`tools.bash` → `returned success without data`；`bash_id`/退出码在被丢掉的正文里，后台进程可能仍在。

文件工具已是另一契约：成功 resolve `data`，领域失败 reject `ToolError`（`src/tool/outcome.rs` ≈194+）。bash「退出码在散文、非零也算成功」不能原样套。

## 2. 权限 / 审批 / 超时 / 流式 / 工作目录 / 并发

### 权限与审批（够用）

嵌套 `tools.*` → `run_nested` → 同一 `execute_tool_call`（校验、authorize、floor、审批、写锁、审计、取消令牌；`src/tools/code.rs:114-148`）。`run_code` 自身不带权限（D9）。I8：绑定集合里没有则调不到。

`bash` floor 只升格危险命令的 **Deny**；非根 `rm -rf` 的 Ask 不会成 floor（`src/permission/floor.rs:27-36`、`bash_safety`）。SAFE 默认 Deny 只读放行；ALL allow-all。`kill_shell`/`wait_shell` 无预设 → 缺省 Allow。

审批时 `PausingSink` 持 `BudgetPause`（`src/tools/code.rs:152-165`）；拒绝 → 可捕获 `permission_denied`（合 D7）。Ask 在进程启动前。进 Code **不**放宽规则，也**不**顺便补 Direct 上已有的 floor 缝。

### 超时（三套钟，单位不一致）

| 钟 | 到点 |
|---|---|
| `bash.timeout`（秒，默认 30，≤600） | 转 running，不杀（`bash.rs:132-139`） |
| `wait_shell.sec`（1–600） | `status: waited`，进程仍在 |
| `Tool::timeout()` | 三者皆 `None`（bash/wait 本地覆盖；kill 靠 trait 默认 `trait_.rs:119-121`） |
| `run_code.timeout_ms` | 默认 120000，>600000 夹紧；结束整个程序；审批等待不计入（`code.rs:36-38`、`:1070-1080`） |

SDK：工具超时是内部的，程序预算只有 `timeout_ms`（`code_manifest.rs:26`）。把 `timeout`/`sec` 原样进 Code = 第二/三套预算（秒）。D-09 已从文件工具拿掉 per-call 超时。

### 流式与 64KB

PTY → `BoundedTee`：内存头 2KB+尾 4KB；盘上限 8MB（`src/terminal/tee.rs:12-18`）。增量进 tee/`jobs_changed`，**不**进模型 token 流、不新 Item（I9 仍立）。

Code 账本按 D8 **不含结果正文**（`charter.md:35`；摘要约 80 字偏 `command=`，`code.rs:57`、`:769-795`）。stdout 要进模型只能进程序返回值，再经 64KB 渲染（`code/render.rs:17`）。一次头尾窗 ~6KB 装得下；多次整段日志会撞 1MB 输出硬顶 / 16MB JS 堆（不可捕获）。

`.litecode/bash/<id>.output` 可用 `tools.read` 分页；`grep`/`glob` 对 `.litecode` 会 `path_excluded`。

### 工作目录

`spawn_command` workdir 固定 `None`（`bash.rs:81`/`98`）→ 线程局部 LAP（`shell.rs:83-87`），**不看** `ToolExecutionContext.workspace_root`。output 相对路径用执行上下文 root——两条路径。回合 current-thread + `set_runtime_paths` 时碰巧对齐，不是契约。无持久 shell、无 `cwd` 参数。

### 并发

回合批处理：会写的 `bash`（`is_concurrency_safe == false`）单独成批。`tools.*` 每次独立 `execute_tool_call`，不再 `partition_tool_calls`。bash `effects()` 默认空 → 文件写锁挡不住同程序里的 bash。再叠加同步阻塞，实际并行几乎不发生（见下节）。

## 3. 预算 / 封口 / unsettled

JS 在 `ptc-js`；host future 派到**调用方** tokio runtime = 回合 current-thread（`src/ptc/mod.rs:8-9`、`:441-445`；`runtime/mod.rs:483-484`）。

`bash`/`wait_shell` 在 async `execute` 里对 `jobs.wait` **同步**等待：持锁 + 约 50ms condvar 看取消（`agent_jobs.rs:429-496`；调用点 `bash.rs:104-110` 一带）。current-thread 上这段等待占住回合线程 → 预算 watcher 排不进（`engine.rs:480-515`/`803`）。

后果：

1. **用户取消仍有效**（令牌外取消，wait ≤~50ms 可见）；前台 `bash` 取消会 `hub.kill`；`wait_shell` 取消只报错不杀。
2. **程序预算在调用期间不生效**：watcher 发不出 Timeout/`cancel_open`；命令可跑到自身退出或 `timeout`/`sec`（最长 600s）。I7「超时先立刻取消在途嵌套」对阻塞中的 bash/wait **做不到**（`charter.md:49`）。
3. **超时后外层无 stdout / 无 bash_id**（超时失败不保留程序返回值；账本无结果正文）。
4. **若预算取消真打到令牌**：语义与 `bash.timeout` 相反（取消杀前台 vs 到点转后台）。宽限 3s；`hub.kill` 最坏可睡 ~5s 且不看令牌 → 可能 `unsettled` 后进程才死。
5. **在途上限 64 挡不住真并行**：槽先占；第一个阻塞的 bash 不返回，后续 host 不被 poll。

封口：嵌套「已取消」今天常记成 `internal`，不是 `cancelled`。I12 程序级取消不可捕获；嵌套 bash 自己返回的取消看起来像内部错误。

## 4. 与文件工具同程序：收益与风险

**收益真实：** 改文件 → 跑测 → 按退出码再改，今天至少多一次模型往返；同程序可少一程。非零退出应留在成功 `data.exit_code`；若做成未捕获 `ToolError`，会按 I7 取消其余在途（应用 `Promise.allSettled`）。

**风险更大：** 无结构化 `data` 无法编排；同步等待冻住同程序文件工具；改成非阻塞后写锁空洞变真并行；后台作业比程序长命（D1 下 wait/kill 也只能再写 `run_code`）；结果正文 vs 64KB/账本分工未定。

## 5. I5 / I7 / I8 / I12 够不够

| 条款 | 位置 | 对本三件套 |
|---|---|---|
| I5 | `charter.md:47` | 管线够；bash 未声明 `effects`，写锁不替它与 edit 串行 |
| I6 | `:48` | 够；不该为 PTY 改 runtime |
| I7 | `:49` | **不够覆盖现状**：用户取消进得去；程序预算取消进不了阻塞调用；取消≠timeout 转后台 |
| I8 | `:50` | 够；进 Code 不扩大命令权 |
| I9–I11 | `:51-53` | 够（不新 Item；视口非主障；说明静态） |
| I12 | `:54` | 条文够，**返回值不够**：成功必须 `data`；退出码/running/waited 属成功字段 |

产品仍要拍板 I7 对后台作业：程序结束时前台该杀还是留 `bash_id`。

## 6. 风险汇总

1. 只改 `surface()` / 无 schema：两面消失或成功变内部错误，后台可能无 id。  
2. 程序预算挡不住阻塞中的 bash/wait（最长可拖到调用自身 600s）。  
3. 非阻塞化后不补 `effects` → 与 edit/write 抢文件。  
4. 三件套必须同面；只把 bash 进 Code 违 D1。  
5. 非零退出若未捕获 ToolError → 取消同程序写入。  
6. 8MB 日志进 JS → 撞堆/输出上限。  
7. `hub.kill` > 3s 宽限 → unsettled 与「已杀」语义糊在一起。  
8. cwd 依赖 LAP；host 线程一变就与 workspace_root 分家。  
9. floor 缝（非根 `rm -rf` Ask）是既有问题，不该在「进 Code」顺手改。

## 7. 工具设计要不要改

**要进 Code，就要改契约，不是改 `surface()`。按本次结论不进；Direct 只建议一处小改。**

不进时建议的 Direct 小改：`spawn_command` 工作目录改为本次 `workspace_root`，勿再 `workdir: None` 读 LAP（output 路径已用执行上下文 root）。不改模型字段、不改超时/取消语义。

不进时不要做：把 `jobs.wait` 改成真异步（那是为程序预算服务的）；把结果改成 `data`；改「到点转后台 / 取消杀前台」；拆三件套互指说明。

若以后真要程序内跑命令，须**同时**满足（否则另立 WP，勿打开 surface）：

1. 成功 `data` 至少稳定含 `status`、`exit_code`、`bash_id`、`output_file`、有界 head/tail；非零/running/waited 都是成功。  
2. 等待不占 current-thread，使预算能取消在途并在 3s 宽限内见终态；cwd 来自执行上下文。  
3. 明确程序结束：杀前台 vs 留可追踪 `bash_id`；两套语义勿共用一个 `Cancelled`。  
4. 会改文件的命令声明 `effects`。  
5. 返回摘要+日志路径；长日志用 `tools.read` 分页。  
6. 三件套同一面。

更贴 I7 的形态往往是**新的前台工具**（可取消、结构化摘要、结束即杀进程组、不登记后台）——不是把今日 `bash` 标成 Code。章程 D2 / D-11 尚未有该工具。

## 8. 可选后续

维持 Direct；WP-195 不必改三件套。若收 cwd，单开极小修复。若仍要「改完就测」少往返，另立 WP 做前台可取消结构化工具，不碰现有后台作业与 kill/wait。

---

**确认：** 未改 `src/**`、`docs/**`、descriptions、`code_manifest`、fixture、WP-194 文案目标；未 commit / push / 开 PR。  
**模型：** Grok Bot（会话自动路由；无 picker）  
**branch / SHA：** `feat/tool-contract-ptc` / `bd71f8c`
