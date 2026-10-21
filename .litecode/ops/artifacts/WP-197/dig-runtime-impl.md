# WP-197 Runtime 实现笔记 — shell `run` / `wait` / `kill`（硬清）

> **作者：** Runtime（Grok Bot 执行子代理）  
> **时间：** 2026-10-22 Asia/Hong_Kong  
> **分支 / HEAD 基线：** `feat/tool-contract-ptc` @ `ab95a80`（实现**未** commit；工作树有改动）  
> **依据：** `brief.md`、`plan.md`（Q1–Q4 已批）；Andy 硬清口径  
> **硬约束遵守：** **未跑 cargo test/build**（available≈2.8–2.9Gi）；**未 commit/push**；**未改** `src/tool/fixtures/run_code_sdk.*`

---

## 1. 关闭/替换层（一句话）

**默认绑定层 + registry 注入层：** seed/`core_configurable` 只绑 `run`（SAFE dial 同原 `run_command`）；`wait`/`kill` 不进设置卡，由 registry 在见到 `run` 时注入（Fixed allow-all）。旧 `run_command`/`command_output`/`command_kill` 及 Direct 三件套实现/description/绑定全部删除；`ensure_core_bindings` 的 strip 清残留行；retired 名进 `reserved_command_name_rejection`。



## 1.1 默认 agent 首轮能否调 wait/kill（mid 确认）

**能。** brief「默认 shell 绑定仅三动作」按 **agent 可调用面** 满足，不是三行都进 `agent_tools` 设置卡：

| 层 | 行为 |
|---|---|
| seed / `core_configurable` | 只绑 **`run`**（SAFE dial）；`wait`/`kill` **不进** settings 卡（与旧 `command_output`/`command_kill` companion 同型） |
| `build_tool_list` | 见到 `run` 即注入 `WaitTool`/`KillTool`（Fixed allow-all） |
| Code host 投影 | 三者一并打进 `run_code` whitelist / SDK → 首轮即可 `tools.run` / `tools.wait` / `tools.kill` |
| 关 `run` | companion 一并消失（测：`run` disabled → 无 wait/kill） |

证据：`registry` 测 `core_bindings_produce_full_builtin_list` + code-host 子列表 SDK 含 `["run"]`/`["wait"]`/`["kill"]`。

**不另补 seed 行**：三动作都绑设置卡会与「companion 非 settings」产品不一致；若 mid/GM 强制三行 seed，再改。

---

## 2. 行为（对照 plan §3）

| 工具 | 行为 |
|---|---|
| **`run`** | `PipeDetach::Background`（短窗 ≈500ms）：快命令 → `exit_code` 有值；仍跑 → `exit_code=null`，登记可 `wait`/`kill` 的 `id`（`bg_`+hex）。无 `kind` / `timeout_sec` / `run_in_background` / promote |
| **`wait`** | 入参 `id` + **必填** `sec`（≥1）；`>300` **静默钳到 300**（上限写入 schema/description）；到期未结束仍 resolve（null exit + 现状 output + 说明），不杀进程 |
| **`kill`** | 杀进程组；`output` 含「process was killed」；消费 mailbox notice |
| **`Command`** | 对外 data **仅** `{id, exit_code, output}`；`output` 字段始终有（可空串）；截断/落盘路径/状态说明写进字符串 |

底层复用：`AgentJobRegistry` / `supervise_pipe` / 进程组 kill / tee 截断落盘。落盘目录由 `.litecode/run_command/` **改名为** `.litecode/run/`。

---

## 3. 删除清单（硬清）

| 类 | 已删/已改 |
|---|---|
| 实现文件 | 删 `run_command.rs` / `command_output.rs` / `command_kill.rs`；新增 `run.rs` / `wait.rs` / `kill.rs`；`command_job.rs` 改写为内部 helper（`CommandData` + output 拼装） |
| descriptions | 删 `run_command.md` / `command_output.md` / `command_kill.md`；接线 `run.md` / `wait.md` / `kill.md`（Tools 草稿；Runtime 已 scrub 旧符号教法句） |
| core / preset | `core_configurable`：`run_command`→`run`；`PRESET_TOOL_IDS` / SAFE dial 同步；`wait`/`kill` Fixed |
| seed | explore/general/default 绑 `run`；`strip_direct_shell_bindings` 增删 `run_command`/`command_output`/`command_kill` |
| reserved | `wait`/`kill` 保留给 companion；`run_command`/`command_output`/`command_kill`/`bash`/`wait_shell`/`kill_shell` 退休占位 |
| floor / messages / authorize | 危险命令 floor 与 SAFE 文案目标名 → `run` |
| registry | builtin + companion 注入 + `builtin_code_surface_tools` + 相关测 |
| 路径/文案 | `readonly.md`、`DATA.md` reminder 行、web `SessionStatusLine` / `bashLive` 测路径 |

**故意保留旧符号的位置（非契约暴露）：** reserved 拒绝表、seed strip 列表、负向断言（「不应出现」）、历史回放文档/`final_immutability` 旧 Item 名、web 对旧 Direct 卡回放测。

---

## 4. 改动文件列表（Runtime）

**新增：**  
`src/tools/run.rs`、`src/tools/wait.rs`、`src/tools/kill.rs`  
`.litecode/ops/artifacts/WP-197/dig-runtime-impl.md`（本文件）

**重写/大改：**  
`src/tools/command_job.rs`、`src/tools/mod.rs`  
`src/tool/registry.rs`、`src/config/global_db/tools.rs`、`src/config/global_db/seed.rs`  
`src/permission/{presets,floor,messages,engine}.rs`  
`src/tool/{authorize,availability,executor}.rs`  
`src/terminal/agent_jobs.rs`、`src/terminal/mod.rs`（测路径）  
`src/config/guides/readonly.md`、`src/session/DATA.md`  
`web/src/lib/bashLive.test.ts`、`web/src/components/SessionStatusLine.test.tsx`  
`src/client_protocol/protocol.rs`（注释）  
`src/tools/descriptions/{run,wait,kill}.md`（scrub 旧名教法）

**删除：**  
`src/tools/{run_command,command_output,command_kill}.rs`  
`src/tools/descriptions/{run_command,command_output,command_kill}.md`

---

## 5. 单测草稿（未跑 cargo）

| 模块 | 覆盖 |
|---|---|
| `run` | surface/Code、data 三字段无 kind、快命令 exit_code、长命令 null+job 存活、validate、SAFE dial、cancel |
| `wait` | sec 必填、`>300` 静默钳制、等到退出、到期 null+说明、not_found |
| `kill` | Fixed allow、杀进程+output 说明、not_found |
| registry/seed/presets/floor | 绑定仅三动作、companion 注入、strip 旧行、reserved |

---

## 6. Tools 需对齐（Runtime 未做）

1. **刷新** `src/tool/fixtures/run_code_sdk.txt` + `.sha256`（手渲 `run`/`wait`/`kill` → `Command`；旧符号应消失）  
2. 复核教法与 SDK 示例（object bag：`tools.run({command})` 等，与既有 Code 工具一致）  
3. 扫仓内教法/文档残留（历史回放除外）

---

## 7. grep 自检（Runtime）

- **实现/description：** 无旧三件套文件；`descriptions/*.md` 无 `run_command`/`command_output`/`command_kill`/`wait_shell`/`kill_shell`  
- **活跃契约：** `core_configurable` 与默认绑定为 `run`；companion 为 `wait`/`kill`  
- **仍含旧字符串处：** reserved / strip / 负向测 / 历史回放 — 见 §3「故意保留」

---

## 8. 给 mid 的状态

- 实现 + 接线 + dig **已齐**  
- **未跑 cargo**（内存不足；**请 mid 放行后再测**）  
- **未 commit / 未 push**  
- 名称冲突：无非 shell 工具注册为 `run`/`wait`/`kill`；`subagent_wait` 保留；`run` ≠ `run_code`


## 9. 待命测命令（mid 放行后；并行度≤2）

```bash
cd /workspace/litecode
export LC_MIN_MEM_GIB=2 CARGO_BUILD_JOBS=2
mkdir -p .litecode/ops/artifacts/WP-197
{
  echo "=== WP-197 shell triad ==="
  cargo test --lib -- tools::run:: tools::wait:: tools::kill:: command_job 2>&1
  echo EXIT_tools:$?
  echo "=== seed / registry / sdk ==="
  cargo test --lib -- seed_plants core_bindings_produce binding_filter command_tools_have_no_settings sdk_snapshot phase0_run_code_sdk preset_surface 2>&1
  echo EXIT_seed_reg:$?
} | tee .litecode/ops/artifacts/WP-197/cargo-run-wait-kill.txt
```

放行前 **不开 cargo**。fixture 仍由 Tools 测绿后刷。

## 10. cargo 结果（2026-10-22 Asia/Hong_Kong）

| 批 | 结果 |
|---|---|
| `tools::run/wait/kill` + `command_job` | **EXIT 0** · 15 passed |
| seed / registry / preset / code_gate | **EXIT 0** · 6 passed |
| `phase0_run_code_sdk_snapshot` | **EXIT 101** · fixture 仍旧（`run_command`/`command_*`）；现投影为 `run`/`wait`/`kill` + 新 edit — **Tools 刷 fixture** |

日志：`cargo-run-wait-kill.txt`。未 commit。

## 11. 收口待命

mid：Tools 刷 SDK 中；**Runtime 暂勿重跑 cargo**，等 sdk 绿后 mid deliver。
