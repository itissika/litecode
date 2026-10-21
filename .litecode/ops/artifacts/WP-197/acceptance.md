# WP-197 acceptance

**口径：** Andy + GM 硬清；Code shell 仅 `run` / `wait` / `kill` → `Command{id, exit_code, output}`。  
**分支：** `feat/tool-contract-ptc`（实验；不合 main）  
**方案：** `plan.md`（Q1–Q4 全认）

| # | done_when | 结果 |
|---|-----------|------|
| 1 | Code 面仅三动作；默认 shell 绑定只有它们 | **通过**。seed/`core_configurable` 绑 `run`（SAFE）；`wait`/`kill` 由 registry companion 注入（Fixed）；关 `run` 则三者皆无。见 `dig-runtime-impl.md` §1 / §1.1。 |
| 2 | 仓库无旧 shell 契约符号/文案残留（grep） | **通过（活跃面）**。`src/tools/descriptions/` 与 `src/tool/fixtures/run_code_sdk.*` **干净**（无 `run_command`/`command_output`/`command_kill`）。实现文件已删。旧名仅留 reserved 拒绝表、seed strip、负向断言、历史回放文案（brief/dig 允许）。 |
| 3 | 相关 cargo 绿 + artifacts | **通过**。`cargo-run-wait-kill.txt`：tools/job **15** + seed/registry/preset/code_gate **6** = **21 passed**（`EXIT_tools:0` / `EXIT_seed_reg:0`）。初跑 `EXIT_sdk:101` 为 fixture 漂移；Tools 刷后 `cargo-sdk-tools.txt`：`tool::sdk_snapshot` **2 passed**（`EXIT_VERIFY:0`）。fixture sha256=`49e251ed2fbb172c6e0d901c6b78a643cf091e20ad5a283f780aa7c2cf8ea102`。 |
| 4 | push 实验分支；未擅合 main | **交付时** push `feat/tool-contract-ptc`；不合 main。 |

## 契约要点（已落地）

- `tools.run({command})` / `tools.wait({id, sec})` / `tools.kill({id})` → `Command{id, exit_code, output}`
- `wait.sec` 必填；`>300` **静默钳到 300**（说明可见）
- `output` 始终有字段；截断/落盘路径/被杀说明写进文本；落盘目录 `.litecode/run/`

## Agent 视角最小场景

**应走：** 同程序 `tools.edit`/`write` → `tools.run({command})` → 若 `exit_code===null` 则 `tools.wait({id, sec})` → 按 `exit_code`/`output` 分支。  
**不应走：** `run_command` / `command_output` / `command_kill`；默认 Direct `bash`/`wait_shell`/`kill_shell`。

## 同分支附注

工作树同提交含 **WP-198**（`edit`/`apply_edits` 精简）实现与教法；其 cargo 见 `artifacts/WP-198/`，acceptance 以 WP-198 正文为准。本包验收不依赖 198。

## 逃生

无双轨。旧名 reserved，不可被 custom/MCP 占用；strip 清残留绑定行。
