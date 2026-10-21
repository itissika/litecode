# WP-198 diff-summary

**分支：** `feat/tool-contract-ptc`  
**相对起点：** `ab95a80`（与 WP-197 同基线；常与 197 同一次 tip 合入）  
**本包合入后：** tip 含精简 edit + apply_edits（及 WP-197 shell，若同提交）。

## 产品改动（edit）

| 路径 | 变更 |
|---|---|
| `src/tools/edit.rs` + `edit/{engine,result,planner,matcher,tests,python_indent}.rs` | 单替换 → `EditResult`；硬清旧多块/双出口 |
| `src/tools/edit/feedback.rs` | **删除** |
| `src/tools/apply_edits.rs` | **新增** 串行 batch → `{results}` |
| `src/tools/descriptions/{edit,apply_edits}.md` | 活教法（object bag；无旧 knobs） |
| `src/tool/registry.rs` / `mod.rs` | companion 注入 `apply_edits` |
| `src/permission/{engine,floor,presets}.rs` / `authorize.rs` | 别名跟 edit；Fixed 列表 |
| `src/tool/{contract_matrix,code_manifest,...}.rs` | matrix present 空 content；PRELUDE 去旧双出口 |
| `src/tools/{phase2_gate,phase3_gate,baseline/**,code}.rs` | 测/ledger 对齐新契约 |
| `src/tool/fixtures/run_code_sdk.{txt,sha256}` | 含新 edit/apply_edits（与 197 同刷） |

## 测试

- `cargo-edit-apply.txt`：34 + 9 = 43 passed
- dig §9：`present_*` 空 content 修复后 matrix 绿
- SDK：见 WP-197 `cargo-sdk-tools.txt`

## 未做

- 不合 main；未入库脏 ops board
- `write.expected_hash` 保留（非本包）
