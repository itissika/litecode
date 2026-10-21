# WP-198 Tools 教法 — dig（收口终态）

> **作者：** Tools  
> **时间：** 2026-10-22 Asia/Hong_Kong  
> **分支：** `feat/tool-contract-ptc`（不合 main；**未 commit**）  
> **阶段：** **收口完成** — 草稿已叠 live；PRELUDE 旧 edit 教法已清；fixture 与 WP-197 同次刷新（含 apply_edits）。  
> **依据：** `brief.md`、`plan.md`；中层拍板 A–E；「198 整包 cargo 仍等 mid 放槽」。

## 0. 总评

| 项 | 状态 |
|---|---|
| 单轨教法 | **只教** `tools.edit` / `tools.apply_edits` → `EditResult{ok,path,message}`（apply 外包 `{results}`） |
| 活文案 | `edit.md` / `apply_edits.md` **与** `artifacts/WP-198/draft-*.md` **byte-identical**（Runtime 已先叠；Tools 复核） |
| 旧教法硬清 | description 无 `replace_all`/`expected_hash`/blocks 出参教法；**PRELUDE** 去掉 `edit_failed` + blocks/failed 双出口段，改为 EditResult 指引 |
| SDK / fixture | **同 WP-197 一次刷新**已含新 edit/apply_edits；198 **未**单独再刷 |
| 整包 cargo | **未跑**（mid 占槽）；仅 sdk_snapshot + description smoke |

## 1. 交付路径

| 路径 | 说明 |
|---|---|
| `src/tools/descriptions/edit.md` | = `draft-edit.md`（A–E） |
| `src/tools/descriptions/apply_edits.md` | = `draft-apply_edits.md`（真串行、全失败仍 resolve） |
| `src/tool/code_manifest.rs` | PRELUDE：删 `edit_failed` 码表项与旧 blocks 段；改 EditResult 指引 |
| `src/tool/fixtures/run_code_sdk.*` | 见 WP-197；sha `49e251ed2fbb172c6e0d901c6b78a643cf091e20ad5a283f780aa7c2cf8ea102` |
| `.litecode/ops/artifacts/WP-198/dig-tools-teaching.md` | 本笔记（终态） |

## 2. 拍板 A–E（已写入活文案）

| # | 结论 |
|---|---|
| A | object bag |
| B | 同 path **真串行可见** |
| C | not_found/权限等 → `ok:false` resolve，**不** ToolError（全失败也不 throw） |
| D | 成功 message 短确认 `replaced` / `applied` |
| E | write/read `expected_hash` **出本包** — 未改 `write.md`/`read.md` |

## 3. 约束 / 未做

- **未** commit / push / 合 main。  
- **未**跑 WP-198 整包 cargo。  
- write/read expected_hash 清理属另包。  
- contract_matrix 里 retired `edit_failed + blocks` 臂属 Runtime 保留负向，未动。
