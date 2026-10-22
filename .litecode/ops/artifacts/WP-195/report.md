# WP-195 终稿调研报告：bash 系列是否进 Code 模式

| 项 | 内容 |
|---|---|
| **标题** | bash 系列（`bash` / `kill_shell` / `wait_shell`）能否 / 该不该进入 Code 面 |
| **WP** | WP-195 |
| **模型** | Grok Bot（会话自动路由；无 picker）。说明：原要求顶级 Opus；Grok Bot 无模型 picker，按 Andy 确认用会话自动路由产出（本报告由 mid 侧 knowledgeWork 聚合 dig）。 |
| **分支** | `feat/tool-contract-ptc` @ `bd71f8c` |
| **日期** | 2026-10-21 Asia/Hong_Kong |
| **性质** | 只读调研，未改产品码 |

---

## 1. 一句话结论

**不进。** 今日三件套保持 Direct；只改 `surface()` 会准入失败或成功变内部错误。真要「改完就测」少往返，应另立前台可取消的结构化工具，而不是把现有 bash 作业协议标成 Code。

---

## 2. 大结论散文（给 Andy）

### 能否 / 该不该

**能进的前提不存在；该不该进，答案是否。**

Agent 今天在 Direct 面用 bash 很清楚：一条命令、超时到点转后台留 `bash_id`，再用 `wait_shell` / `kill_shell` 管作业。Code 面要的是另一套事——成功必须带稳定 `data`、嵌套调用不能堵死 `run_code` 预算、程序结束时前台进程语义要干净。这三件现成工具都对不上。

投影规则更硬：`ToolCaller::Code` 只能调 `surface() == Code` 的工具；Direct 在程序里直接拒绝。D1 禁止双面，所以要进就得三件套一起进，跨回合的 wait/kill 也只能再写进 `run_code`。现状三者都无 `data` schema、成功走纯文本 `ok`，与 Code 成功路径和准入检查冲突。

章程 D2 / 改造计划 D-11 写「本轮不做 bash code 化」，与本结论同向；但依据是 Runtime 对行为的复核（结果契约、同步 wait、作业寿命），不是只复述文档。

### 挑战（agent 易理解优先）

| 维度 | 对齐情况 |
|---|---|
| 权限 / 审批 | 可复用。嵌套已走同一条 `execute_tool_call`（I5/D7/I8）；进 Code 不放宽规则。 |
| 超时 | 三套钟拧在一起：`bash.timeout` / `wait_shell.sec`（秒）与 `run_code.timeout_ms`（毫秒）单位与语义都不同；到点转后台 ≠ 程序预算取消杀前台。 |
| 流式 / 视口 | PTY 增量不进模型 token；进程序返回值再撞 64KB 渲染与输出硬顶。长日志应走 `output_file` + `tools.read`，不是整段塞进 JS。 |
| 工作目录 | `spawn_command` 工作目录走线程局部 LAP，不读 `workspace_root`——契约缝，不是「碰巧对齐」就能当设计。 |
| 并发 / 预算 | `bash`/`wait_shell` 在 current-thread 上同步 `jobs.wait`，堵预算 watcher；程序超时在调用期间发不出。与文件工具同程序编排的收益，被这一堵抹掉。 |

### 风险

1. **只改 surface / 不补 schema**：准入失败 → 两面都拿掉；或成功无 `data` → 内部错误，正文里的 `bash_id`/退出码被丢掉，后台进程可能仍在。  
2. **阻塞等待饿死预算**：最长可拖到调用自身 600s；I7「超时先立刻取消在途嵌套」对阻塞中的 bash/wait 做不到。  
3. **非阻塞化后写锁空洞**：bash 默认无 `effects`，文件写锁挡不住同程序里的命令写盘。  
4. **非零退出若做成未捕获 ToolError**：会按 I7 取消同程序其他在途写入——与「测失败再改」的编排意图相反。  
5. **作业比程序长命**：D1 下 wait/kill 也只能再进 `run_code`；取消 vs timeout 转后台两套语义不能共用一个 `Cancelled`。  
6. **错误形态**：嵌套取消今天常记成 `internal`，agent 不易区分「被取消」与「真坏了」。

### 收益

编排密度真实存在：改文件 → 跑测 → 按退出码再改，今天至少多一次模型往返；同程序可少一程。但**现状拿不到**——先要稳定 `data` + 非阻塞等待。收益不构成「把今日 bash 标进 Code」的理由。

### bash 工具设计要不要改

**明确回答 brief 第 5 问：**

- **不进 Code（本包结论）→ 现有三件套契约不要为 Code 重设计。** 不要把 `jobs.wait` 改成真异步、不要把结果改成 `data`、不要改「到点转后台 / 取消杀前台」、不要拆三件套互指说明。  
- **Direct 仅建议一处小修：** `spawn_command` 工作目录改为本次 `workspace_root`，勿再 `workdir: None` 读 LAP（output 相对路径已用执行上下文 root）。不改模型可见字段，不改超时/取消语义。  
- **若以后真要程序内跑命令：** 不是给现有 bash 开 surface，而是另立契约（最好是新的前台可取消结构化工具：结束即杀进程组、不登记后台、成功 `data` 含 `status`/`exit_code`/有界 head-tail/`output_file`）。三件套若整体 code 化，须同时满足 Runtime dig §7 六条门槛，并另立 WP。

---

## 3. 评分表

来源：`dig-runtime.md` 总评表（本报告沿用；未另评分）。

| 项 | 分 | 含义 |
|---|---|---|
| 按现状进入 Code | 2/10 | 无 `data` schema；成功进 Code → 内部错误 |
| 权限 / 审批沿 I5、D7、I8 | 8/10 | 嵌套已走同一条 `execute_tool_call` |
| 与程序预算、封口、unsettled 共存 | 1/10 | 前台同步等待饿死预算 watcher |
| 与文件工具同程序编排 | 收益真实，现状拿不到 | 先要稳定 `data` + 非阻塞等待 |

**总评 2/10 → 不进。**

---

## 4. 关键证据摘要

细节与行号见附录；此处只留裁决用要点。

1. **面与准入**：三者未覆盖 `surface()`，默认 Direct（`trait_.rs:49-50`）。Code 调用 Direct 被拒（`executor.rs:830-855`）。Code 成功要求 `data` + schema（`executor.rs:884-906`；准入 `code_manifest.rs:119-131`）。`ToolCallResult::ok` 固定 `data: None`（`result.rs:83-86`）。  
2. **同步 wait**：`bash`/`wait_shell` 对 `jobs.wait` 同步阻塞（`agent_jobs.rs:429-496`；调用点约 `bash.rs:104-110`）；host future 在回合 current-thread（`ptc/mod.rs:441-445`）→ 预算 watcher 排不进。  
3. **权限可复用**：嵌套 `run_nested` → 同一执行管线（`code.rs:114-148`）；进 Code 不扩大命令权（I8）。  
4. **cwd 缝**：`spawn_command` `workdir: None`（`bash.rs:81`/`98`）→ LAP，不看 `workspace_root`。  
5. **文档同向但非依据**：D2 首期不含 bash（`charter.md:22`）；本轮不做 bash code 化（`:60`）；D-11 推荐否、独立项目（`code-tool-rework-plan.md:615`、`:711`）。行为证据优先。

---

## 5. 可选后续包（本包不实现）

| 包 | 内容 | 优先级建议 |
|---|---|---|
| A. Direct cwd 小修 | `spawn_command` 改用本次 `workspace_root`；单测对齐；不碰 timeout/取消/结果形态 | 可单开极小修复 |
| B. 新前台结构化执行工具 | 可取消、成功 `data`、结束杀进程组、不登记后台；供 `run_code` 内「改完就测」 | 若产品要编排收益，另立 WP |
| C. 现有三件套整体 code 化 | 须同时满足 data 契约、非阻塞 wait、程序结束语义、`effects`、三件同面等；独立大项目 | 不建议在 PTC 本轮插入 |

WP-195 本身：维持 Direct，不必改三件套 surface / schema / 说明。

---

## 6. 附录

### 6.1 材料清单

| 材料 | 状态 |
|---|---|
| `artifacts/WP-195/brief.md` | 已读 |
| `artifacts/WP-195/dig-mid-notes.md` | 已读（工具清单 + charter/计划摘录 + agent 视角 Direct bash） |
| `artifacts/WP-195/dig-runtime.md` | 已读（行为复核；总评 2/10 不进；评分表来源） |
| `artifacts/WP-195/dig-tools.md` | **缺失**。Tools 视角由 mid 清单 + Runtime 契约结论补全；**未假称有独立 Tools dig**。 |
| `docs/ptc/charter.md`（D2 等） | 对照，未整篇复述 |
| `docs/ptc/code-tool-rework-plan.md`（D-11） | 对照 |
| `docs/ptc/run-code-remediation-plan.md` | 对照（范围排除 bash code 化） |

分支 HEAD 复核：`feat/tool-contract-ptc` / `bd71f8c`（`bd71f8c docs(tool): 去掉工具说明里对其他工具的点名调用`）。

### 6.2 重要路径 / 行号摘录

| 主题 | 位置 |
|---|---|
| Direct 默认 surface | `src/tool/trait_.rs:49-50` |
| Code 拒 Direct | `src/tool/executor.rs:830-855` |
| Code 成功要 data | `src/tool/executor.rs:884-906` |
| 准入要 data schema | `src/tool/code_manifest.rs:119-131` |
| ok 无 data | `src/types/result.rs:83-86` |
| 嵌套执行管线 | `src/tools/code.rs:114-148` |
| 同步 jobs.wait | `src/agent_jobs.rs:429-496`；`src/tools/bash.rs` ≈104-110 |
| cwd / LAP | `src/tools/bash.rs:81`、`:98`；shell LAP `shell.rs:83-87` |
| D2 / 不做 bash code 化 | `docs/ptc/charter.md:22`、`:60` |
| D-11 推荐否 | `docs/ptc/code-tool-rework-plan.md:615`、`:711` |

### 6.3 dig 分歧

| 点 | mid | Runtime | 本报告 |
|---|---|---|---|
| 倾向结论 | 盘点为主，未打分 | **2/10 不进** | **采纳 Runtime：不进** |
| 工具清单 | 仅三件套 Direct；辅助模块非工具 | 同意并深挖契约 | 无分歧 |
| 设计是否改 | 未最终裁决 | 不进；Direct 仅 cwd 小修；真要编排另立工具 | **采纳 Runtime** |
| dig-tools | — | 未依赖 | **文件不存在**；不另造 Tools 结论 |

无实质证据冲突；mid 供清单与文档锚点，Runtime 供行为裁决，本终稿聚合二者。

### 6.4 brief 五问对照

| # | 问题 | 答案 |
|---|---|---|
| 1 | 能否 / 该不该进 Code | **不该；按现状也不能安全进** |
| 2 | 挑战 | 权限可复用；卡在同步 wait、作业寿命、纯文本 vs data schema、多套超时 |
| 3 | 风险 | surface  alone 破坏可用性；预算失效；写锁空洞；错误形态变糊 |
| 4 | 收益 | 编排少往返真实，但要新契约才拿得到 |
| 5 | 设计要不要改 | **不进则三件套不大改；Direct 仅建议 cwd→workspace_root；进则必须重设计或另立前台工具** |

---

**确认：** 本报告只写 artifacts；未改 `src/**`、`docs/**`、descriptions、`code_manifest`；未 commit / push / 开 PR。  
**聚合说明：** knowledgeWork 终稿；依据 brief + dig-mid-notes + dig-runtime（dig-tools 缺失已注明）。

---

## 7. Andy 纠正（2026-10-21，GM 追加）

**产品结论覆盖调研「不进」：** bash **最终要进 Code**。难度与风险只能决定**切口与分阶段**，不能得出「不进」。

**动机：** 今日 bash 停在 Direct，模型系统性退化到凡事用 bash——这是要解的问题，不是可接受稳态。

**对 WP-195 的处置：** 证据与门槛（Runtime dig §7）仍有效；§1「不进」与总评「不进」作废为产品裁决。后续实现见 **WP-196**（最低风险切口进 Code + 防退化验收）。

**调研仍可用部分：** 挑战/风险表、禁止只改 surface、六条门槛、可选切口 A/B/C 的技术对比。
