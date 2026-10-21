# WP-196 Tools 教法 — dig

> **口径变更（Andy / 2026-10-21，文首）：** 作废「短测走 Code、长作业走 Direct」双轨。**单轨只教 Code `run_command`。** Direct `bash` / `wait_shell` / `kill_shell` 教法改为废弃/逃生；默认对 agent 关闭由 Runtime 负责（本包不改 Direct 行为代码）。

> **作者：** Tools  
> **时间：** 2026-10-21 Asia/Hong_Kong  
> **分支：** `feat/tool-contract-ptc`（未合 main；未 commit）  
> **依据：** 更新后 `plan.md` §0/§3；`brief.md` Andy 改口径；`dig-runtime-impl.md` 契约字段

## 1. 做了什么（重做后）

| 项 | 结果 |
|---|---|
| `run_command` description | **单轨**：唯一命令路径；去掉一切 Direct bash / 长短作业分流；保留 edit/write→`run_command`+`exit_code` 示例 |
| Direct `bash.md` | **Deprecated escape hatch**；指向 Code `run_command`；不教后台三件套工作流 |
| `wait_shell.md` / `kill_shell.md` | 同标废弃/逃生（仅显式重开 Direct bash 时的残留作业） |
| SDK fixture | 随 `run_command.md` 重渲；禁止双轨措辞进入 fixture |
| Direct 行为代码 | **未改**（实现/schema/enabled 留给 Runtime 默认关） |

## 2. 与 Runtime「默认隐藏」对齐情况

| 现状 | 说明 |
|---|---|
| 计划口径 | `plan.md` §0：三件套默认 off、对 agent 不可见 |
| 工作树实现 | seed 仍绑定 `bash`/`wait_shell`/`kill_shell` 为 enabled（Runtime 默认关尚未落地） |
| Tools 对齐 | 文案先标 **Deprecated / escape / 非默认路径**；不引导当默认。隐藏/enabled=false 等行为属 Runtime，本包不动 |

## 3. 改动路径

| 路径 | 说明 |
|---|---|
| `src/tools/descriptions/run_command.md` | 单轨完整教法 |
| `src/tools/descriptions/bash.md` | 废弃/逃生 |
| `src/tools/descriptions/wait_shell.md` | 废弃/逃生 |
| `src/tools/descriptions/kill_shell.md` | 废弃/逃生 |
| `src/tools/bash.rs` | description 单测对齐废弃文案（断言 Deprecated/escape/`run_command`） |
| `src/tool/fixtures/run_code_sdk.txt` | 手渲插入 `RunCommand*` + 单轨注释 |
| `src/tool/fixtures/run_code_sdk.sha256` | 见下方 |
| `.litecode/ops/artifacts/WP-196/dig-tools-teaching.md` | 本笔记（口径变更置顶） |

## 4. fixture sha256

`334bee4dd1e1635cebfb3e0e2bedc6ce02fb3fadea3ea44424edff1a484bcd3e`

## 5. 关键措辞变化

| 前（已作废） | 后 |
|---|---|
| 短测/构建优先 Code；Direct 仅长作业/后台 | **只**教 `run_command` 为命令路径 |
| bash.md 教 timeout→后台→wait/kill | bash.md：Deprecated escape；显式重开才用 |
| run_command.md 点名 Direct bash 分流 | 不再点名 Direct 三件套 |

## 6. 约束

- 未跑 cargo；fixture 手渲，放行后跑 `phase0_run_code_sdk_snapshot` 核对。  
- 未 commit / 未 push。  
- 未改 Direct 行为代码、权限 floor、seed enabled。
