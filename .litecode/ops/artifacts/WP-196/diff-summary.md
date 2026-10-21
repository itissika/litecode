# WP-196 diff-summary

**分支：** `feat/tool-contract-ptc`  
**相对起点：** `242c5c9`（WP-194 教法对齐）

## 产品改动

| 路径 | 变更 |
|---|---|
| `src/tools/run_command.rs` | **新增** Code-only 前台可取消结构化执行；稳定 `data`；超时/取消杀进程组；不登记后台 |
| `src/tools/descriptions/run_command.md` | **新增** 单轨教法（edit/write→run_command→exit_code） |
| `src/tools/mod.rs` / `registry.rs` | 注册 `run_command` |
| `src/config/global_db/tools.rs` | `run_command` 入 configurable；`direct_shell_tools` 分离 |
| `src/config/global_db/seed.rs` | SEED_REVISION=16；默认绑定只 `run_command`；strip Direct bash 三件套 |
| `src/permission/{presets,floor,messages}.rs` / `authorize.rs` | 权限对齐新名；逃生仍可覆盖 direct_shell |
| `src/tools/descriptions/{bash,wait_shell,kill_shell}.md` | Deprecated/逃生指向 `run_command` |
| `src/tools/bash.rs` | 描述测试字符串对齐（行为未改） |
| `src/tool/fixtures/run_code_sdk.{txt,sha256}` | SDK 含 `run_command`（Tools 手渲；测绿） |

## 未改

- Direct bash/wait/kill **执行逻辑**（模块保留，默认绑定关闭）
- 不合 main；未 commit 脏 ops board

## 测试

- `cargo-hide-bash.txt`：**EXIT 0**，37 passed（run_command / seed / sdk_snapshot / phase0）
