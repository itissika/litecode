# WP-197 diff-summary

**分支：** `feat/tool-contract-ptc`  
**相对起点：** `ab95a80`（WP-196：retire Direct bash / run_command through run_code）  
**本包合入后：** HEAD 变为含 shell 三动作 +（同提交）WP-198 edit 精简 的 tip（见 commit log）。

## 产品改动（shell）

| 路径 | 变更 |
|---|---|
| `src/tools/{run,wait,kill}.rs` | **新增** Code `run`/`wait`/`kill` → `Command{id,exit_code,output}` |
| `src/tools/command_job.rs` | 重写为内部 helper（拼装 Command / 落盘 `.litecode/run/`） |
| `src/tools/{run_command,command_output,command_kill}.rs` | **删除** |
| `src/tools/descriptions/{run,wait,kill}.md` | **新增** 教法（object bag；wait 钳 300） |
| `src/tools/descriptions/{run_command,command_output,command_kill}.md` | **删除** |
| `src/tool/registry.rs` / `src/tools/mod.rs` | 注册 + companion 注入 wait/kill |
| `src/config/global_db/{seed,tools}.rs` | 绑 `run`；strip 旧三件套；reserved 旧名 |
| `src/permission/{presets,floor,messages,engine}.rs` / `authorize.rs` | 权限/文案对齐 `run`；wait/kill Fixed |
| `src/terminal/agent_jobs.rs` 等 | 路径/测对齐 |
| `web/.../bashLive.test.ts` / `SessionStatusLine.test.tsx` | 测路径对齐 |
| `src/tool/fixtures/run_code_sdk.{txt,sha256}` | SDK 含 `run`/`wait`/`kill`（及同刷的新 edit） |

## 同提交另含（WP-198，另见其 diff-summary）

`edit`/`apply_edits` 精简、`apply_edits.rs`、edit 引擎重写、baseline fixtures、`code_manifest` PRELUDE 硬清旧双出口等。

## 未做

- 不合 main；未入库脏 ops board / inbox / log
- 交互终端 / 跨会话长作业另立

## 测试

- `cargo-run-wait-kill.txt`：21 passed（tools 15 + seed_reg 6）；初跑 sdk 漂移后由 Tools 刷 fixture
- `cargo-sdk-tools.txt`：`tool::sdk_snapshot` 2 passed
