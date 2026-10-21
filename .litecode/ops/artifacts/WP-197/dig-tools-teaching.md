# WP-197 Tools 教法 — dig（收口终态）

> **作者：** Tools  
> **时间：** 2026-10-22 Asia/Hong_Kong  
> **分支：** `feat/tool-contract-ptc`（不合 main；**未 commit**）  
> **阶段：** **收口完成** — Runtime 硬删 + 新三工具已落地；Tools 刷 SDK fixture 并测绿。  
> **依据：** `brief.md`、`plan.md`；中层「放 Tools 收口 · 先 197 再 198」；object bag / `run`/`wait`/`kill` / 不写死短等秒数已拍板。

## 0. 总评

| 项 | 状态 |
|---|---|
| 单轨教法 | **只教** `tools.run` / `tools.wait` / `tools.kill` → `Command{id, exit_code, output}` |
| 旧 description | **已删** `run_command.md` / `command_output.md` / `command_kill.md`（Runtime 先行删除；Tools 确认无残留 include_str） |
| 活文案 | `src/tools/descriptions/{run,wait,kill}.md` 对齐 object bag + Command + wait.sec 必填/300 静默钳 |
| SDK / fixture | **已刷** `src/tool/fixtures/run_code_sdk.{txt,sha256}`（含 run/wait/kill **以及** 已 live 的新 edit/apply_edits） |
| cargo | `tool::sdk_snapshot` **2 passed**；日志见下 |

## 1. 交付路径

| 路径 | 说明 |
|---|---|
| `src/tools/descriptions/run.md` | 活教法（object bag；Command；非零仍 resolve；示例 edit→run→wait） |
| `src/tools/descriptions/wait.md` | `id`+必填 `sec`；>300 静默钳；到期不杀 |
| `src/tools/descriptions/kill.md` | 按 id 杀进程组；说明进 output |
| ~~`run_command.md` / `command_output.md` / `command_kill.md`~~ | **已删除** |
| `src/tool/fixtures/run_code_sdk.txt` | 渲染自当前 HEAD code surface |
| `src/tool/fixtures/run_code_sdk.sha256` | `49e251ed2fbb172c6e0d901c6b78a643cf091e20ad5a283f780aa7c2cf8ea102` |
| `.litecode/ops/artifacts/WP-197/cargo-sdk-tools.txt` | UPDATE + VERIFY 日志 |
| `.litecode/ops/artifacts/WP-197/dig-tools-teaching.md` | 本笔记（终态） |

## 2. 教法要点（已锁）

1. **唯一路径：** Code 内 `run` →（可选）`wait` / `kill`；禁止旧名与 Direct。  
2. **`Command`：** 三字段始终在；`output` 可空串但不缺字段。  
3. **`wait.sec`：** 必填；最大 300；超额静默钳制。  
4. **防退化示例：** `edit`/`write` 后 `run`，再按 `exit_code`（必要时先 `wait`）分支。  
5. **入参：** object bag（`tools.run({command})` 等）。

## 3. 测绿

```
CARGO_BUILD_JOBS=2
MemAvailable ≥ 2GiB（实测 ≈9GiB）
cargo test --lib tool::sdk_snapshot -- --nocapture
→ phase0_run_code_sdk_snapshot ok
→ sdk_hash_is_stable_… ok
EXIT 0
```

顺带 smoke：`description_has_no_old_contract_knobs` / `description_mentions_serial_skip` ok（198 活文案已在 fixture 中）。

## 4. 约束 / 未做

- **未** commit / push / 合 main。  
- **未**跑 WP-198 整包 cargo（mid 占槽）。  
- Registry/seed 里对旧名的 **retired 清理断言** 属 Runtime，未改。  
- PRELUDE 旧 edit 双出口文案在 **WP-198 收口** 中一并清（同一次 fixture 刷新已含新 PRELUDE）。
