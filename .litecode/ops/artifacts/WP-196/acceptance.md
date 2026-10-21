# WP-196 acceptance

**口径：** 切口 B + Andy（2026-10-21）Direct bash 家族默认关、单轨只推 `run_command`。  
**方案：** `plan.md`（含 §0 补丁）

| # | done_when | 结果 |
|---|-----------|------|
| 1 | Code 可调、稳定 `data`、不堵预算 | **通过**。`run_command`：data 含 status/exit_code/stdout|stderr 窗/可选 output_file；async 等待；取消/超时杀进程组。见 `dig-runtime-impl.md`。 |
| 2 | 取消/超时语义 + 测 | **通过**。相关 lib 测绿；日志 `cargo-hide-bash.txt` EXIT 0（37 passed）。 |
| 3 | 防退化 | **通过**。默认 seed/绑定不暴露 Direct bash/wait/kill（关闭层：agent_tools 种子；`strip_default_direct_shell_bindings`）；教学/SDK 单轨只推 `run_command`；三件套描述标 Deprecated。 |
| 4 | cargo 绿 + artifacts | **通过**。本目录含 plan / dig-runtime-impl / dig-tools-teaching / cargo-* / diff-summary / acceptance。 |
| 5 | push 实验分支、未合 main | **交付时** push `feat/tool-contract-ptc`；不合 main。 |

## Agent 视角最小场景（防退化）

**应走：** 同一 `run_code` 内 `tools.edit`/`write` → `tools.run_command({command})` → 读 `exit_code` 再改。  
**不应走：** 默认会话里直接调 Direct `bash` / `wait_shell` / `kill_shell`（默认绑定已剥，投影不可见）。

## 逃生

实现与 registry 保留；`direct_shell_tools` + 人工绑定/preset 仍可恢复，**默认 off**。
