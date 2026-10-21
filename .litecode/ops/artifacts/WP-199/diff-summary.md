# WP-199 diff-summary

**分支：** `feat/tool-contract-ptc`  
**相对起点：** `25a15e1`（WP-197/198 tip）  
**本包合入后：** tip 含精简 grep/glob/read（及既有 shell + edit）。

## 产品改动

| 路径 | 变更 |
|---|---|
| `src/tools/grep.rs` | 出参 `{hits,message}`；硬清 cursor/page/searched/base/reason/include_ignored |
| `src/tools/glob.rs` | 出参 `{paths,message}`；paths 工作区相对；硬清 base/page/cursor/reason |
| `src/tools/read.rs` | 出参 `{path,start_line,lines,message}`；保留 start_line；砍 end_line/file.hash/page/reason；缺文件/非文本 → ToolError |
| `src/tools/descriptions/{grep,glob,read}.md` | 活教法；limit 写死；禁方言 |
| `src/tool/code_manifest.rs` | PRELUDE 去 reason/分页方言，改 message + start_line 续读 |
| `src/tool/contract_matrix.rs` / `phase2_gate.rs` / `baseline/**` | 测与 ledger 对齐新契约 |
| `src/tool/fixtures/run_code_sdk.{txt,sha256}` | 已刷；sha `12f6745f42c15531a0006de44975608df70c4590cd5850685baa734ddc62b08b` |

## 测试（不重跑；以 artifacts 为准）

- `cargo-grep-glob-read.txt`：61 + 20 = **81 passed**
- `cargo-sdk-tools.txt`：`tool::sdk_snapshot` **2 passed**
- digs：`dig-runtime-impl.md`、`dig-tools-teaching.md`

## 未做 / 不入库

- 不合 main；未 `wp deliver`（中层）
- 未入库脏 ops board / events / WP-193·195·196 无关 artifacts
- `write.expected_hash` 保留（非本包）
- PRELUDE 全局 `ToolError` 码表仍列 `stale_cursor` 字符串（非本三工具契约字段；未本包删枚举）
