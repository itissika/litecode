# WP-199 Tools 教法 — dig（收口终态）

> **作者：** Tools  
> **时间：** 2026-10-22 Asia/Hong_Kong  
> **分支：** `feat/tool-contract-ptc`（不合 main；**未 commit**）  
> **阶段：** **收口完成** — draft 叠 live；PRELUDE 清旧方言；刷 `run_code_sdk`；sdk_snapshot 绿。  
> **依据：** `brief.md`、`plan.md`；中层「放 Tools 收口 · WP-199」；歧义 1–7；Runtime `dig-runtime-impl.md` limit 数字。

## 0. 总评

| 项 | 状态 |
|---|---|
| 单轨教法 | **只教** `tools.grep` / `tools.glob` / `tools.read` → `{hits\|paths\|lines, message}` |
| 行号 | **全工具 1-based**；read 保留 **`start_line`**（禁止 offset / cursor） |
| limit（写死） | grep **默认 20 / 上限 100**；glob **默认 1000 / 上限 5000**；read **默认 2000 / 上限 5000** |
| 方言禁令 | live + PRELUDE **不教** `cursor`/`page.next`/`reason` 枚举/`include_ignored`/`matches`/`base`/`end_line`/`line_truncated`/`file.hash` |
| 活文案 | `src/tools/descriptions/{grep,glob,read}.md` 已叠 |
| PRELUDE | `src/tool/code_manifest.rs` 已去掉 reason/分页方言三句，改为 message + `start_line` 续读 |
| SDK / fixture | **已刷** sha `12f6745f42c15531a0006de44975608df70c4590cd5850685baa734ddc62b08b` |
| cargo | `tool::sdk_snapshot`（见 cargo-sdk-tools.txt） |

## 1. 交付路径

| 路径 | 说明 |
|---|---|
| `src/tools/descriptions/grep.md` | object bag；hits；limit 20/100；硬 ToolError / 软空搜 |
| `src/tools/descriptions/glob.md` | paths 工作区相对；limit 1000/5000 |
| `src/tools/descriptions/read.md` | start_line；lines:string[]；limit 2000/5000；缺文件/非文本 → ToolError（见 §3.1） |
| `src/tool/code_manifest.rs` | PRELUDE 清 grep/glob/read 旧方言教法 |
| `src/tool/fixtures/run_code_sdk.txt` | 渲染自当前 HEAD |
| `src/tool/fixtures/run_code_sdk.sha256` | `12f6745f42c15531a0006de44975608df70c4590cd5850685baa734ddc62b08b` |
| `.litecode/ops/artifacts/WP-199/cargo-sdk-tools.txt` | UPDATE + VERIFY 日志 |
| `.litecode/ops/artifacts/WP-199/dig-tools-teaching.md` | 本笔记（终态） |
| artifacts `draft-*.md` | 收口前草稿保留；live 已对齐并写死数字 |

## 2. 教法要点（已锁）

1. **入参：** object bag（与 WP-197/198 一致）。  
2. **出参唯一形态：** grep=`hits+message`；glob=`paths+message`；read=`path+start_line+lines+message`。  
3. **有界一次：** limit 数字已写死（§0）。  
4. **续读：** 仅增大 `start_line`；禁止 cursor / `page.next`。  
5. **截断：** 单行缩短落在 `text` / `lines[]`；整次触顶只进 `message`。  
6. **链式示例：** grep→edit；glob→read；read 分页→edit。  
7. **write `expected_hash`：** 出本包（教法声明不教）。

## 3. 歧义对照 + Runtime 落地差

| # | 拍板 | 收口落地 |
|---|---|---|
| 1 | limit 对齐 Runtime | ✅ 20/100 · 1000/5000 · 2000/5000 |
| 2 | read 缺文件/非文本软失败 | ⚠️ **Runtime 仍 ToolError**（`not_found` / `unsupported`）；教法跟 Runtime，避免教错。软失败仅保留：空窗 / 触顶 / session 已在上下文 → resolve + `message` |
| 3 | 非法入参/regex → ToolError；搜空/触顶 → resolve | ✅ |
| 4 | glob paths 工作区相对 | ✅ |
| 5 | include_ignored 硬砍 | ✅ |
| 6 | 行截断落字符串；无 truncated 字段 | ✅ |
| 7 | write expected_hash 出本包 | ✅ |

## 4. 约束 / 未做

- **未** commit / push / 合 main。  
- **未**改 Runtime 行为码（缺文件软失败未强改实现）。  
- Registry / baseline / phase2 属 Runtime 已绿面，本包只动教法 + SDK fixture + PRELUDE。

## 5. 测绿

```
MemAvailable ≈ 8.82 GiB；CARGO_BUILD_JOBS=2
UPDATE_SDK_FIXTURE=1 cargo test --lib tool::sdk_snapshot::phase0_run_code_sdk_snapshot -- --exact
→ updated run_code_sdk.txt + .sha256；EXIT 0
cargo test --lib tool::sdk_snapshot
→ phase0_run_code_sdk_snapshot ok
→ sdk_hash_is_stable_… ok
EXIT 0
tee: .litecode/ops/artifacts/WP-199/cargo-sdk-tools.txt
```

UPDATE hook 已从 `sdk_snapshot.rs` 撤回（不留临时补丁）。
