# WP-194 diff summary — PTC 教法文案对齐裸 resolve data

**分支：** `feat/tool-contract-ptc`  
**范围：** 只改描述 / 注释 / PRELUDE / schema description 字符串 / SDK fixture；**未改**工具执行、权限、render、spill。

## 文件

| 路径 | 改动 |
|------|------|
| `src/tools/descriptions/read.md` | `data.file`/`data.lines`/`data.page`/`data.file.hash` → 裸 `file`/`lines`/`page`/`file.hash` |
| `src/tools/descriptions/glob.md` | `data.base`/`data.paths`/`data.page`/`data.reason` → 裸字段；`page.next`→`cursor` |
| `src/tools/descriptions/grep.md` | 同上；去掉成功路径 `data.*` 信封腔 |
| `src/tools/descriptions/edit.md` | `data.file`/`data.blocks`/`data.applied`/`data.failed` → 裸字段 |
| `src/tools/descriptions/write.md` | `data.file`/`data.created` → 裸字段 |
| `src/tool/code_manifest.rs` | PRELUDE 增续页方言：read 用 `start_line`，glob/grep 用 `cursor` |
| `src/tool/coords.rs` | `FILE_LINE_PREFIX_HINT`：由「read's line-number prefix」改为「line-number prefixes pasted from outer ledger or transcript text」；更新 rustdoc |
| `src/tools/grep.rs` | 模块 rustdoc 去 `data.`；cursor schema description：`data.page.next` → `page.next` |
| `src/tools/read.rs` | `lines[].text` schema description：`Bare line text (no line-number prefix).` |
| `src/tool/fixtures/run_code_sdk.txt` | 与渲染 SDK 同步（PRELUDE / 绑定注释 / old_string 提示） |
| `src/tool/fixtures/run_code_sdk.sha256` | 随 fixture 重算 |

## 刻意未改

- edit matcher / `read_line_prefix` 拒绝逻辑与运行时错误文案（执行语义）
- Promise.all / `.status` 硬拦截、edit 半成功强制查 `failed`、人类 UI
- 无关 ops board / WP-193 / WP-195 调研产物

## 测试

- `cargo test --lib sdk_snapshot` → `phase0_run_code_sdk_snapshot` + `sdk_hash_is_stable_…` **ok**
