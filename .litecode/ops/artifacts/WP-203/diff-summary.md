# WP-203 diff-summary

**分支：** `feat/tool-contract-ptc`  
**相对起点：** `b38ebc4`（WP-202 tip）  
**本包：** webfetch/websearch → Code-only；软失败 `{ok:false}`；SSRF 放行 198.18/15；教法与 SDK fixture 对齐。

## 产品改动

| 路径 | 变更 |
|---|---|
| `src/tools/webfetch.rs` | surface=Code；软失败 `ok_data`；硬错 ToolError；SSRF 放行 198.18/15；出参 `{ok,url,status?,format?,body?,message}`；相关测 |
| `src/tools/websearch.rs` | surface=Code；软失败 `ok_data`；硬错 ToolError；出参 `{ok,query,text,message}`（无 results[]）；相关测 |
| `src/tool/registry.rs` | 两工具投影 Code；Direct 清零断言（`webfetch_websearch_project_to_code_not_direct`） |
| `src/tool/availability.rs` | 对齐 Code 可用性 / Fixed surface；builtin_code 候选含两名 |
| `src/tool/code_manifest.rs` | 进 `builtin_code_surface_tools`；PRELUDE T5 点名 webfetch/websearch 软 vs 硬 |
| `src/tools/descriptions/webfetch.md` | 整份替换：Code；软/硬；body 50000；成功 message `Fetched.` |
| `src/tools/descriptions/websearch.md` | 整份替换：Code；软/硬；无 results[]；成功 message `Search completed.` |
| `src/tool/fixtures/run_code_sdk.{txt,sha256}` | SDK fixture 刷两名教法；sha `b89efddee23d3d398aee0607ccefe5c98ca2a50b10229e074319ca8f571e3c33` |
| `knowledge/contracts/tools/webfetch-ssrf.md` | TUN 放行 198.18/15；仍拒 RFC1918/loopback/fe80/ULA/100.64 |

## 未改

- **`web/`**（本包禁止）  
- **libs** / LSP  
- ops `board.json` / `events/` / `log.jsonl` / `tool_traces` / `board.md`

## 测数（不重跑；以 artifacts 为准）

- `cargo-final-wp203.txt`：**34 passed**（含 `ssrf_allows_hostname_resolved_to_fake_ip`、`webfetch_websearch_project_to_code_not_direct`）  
- `cargo-sdk-tools.txt`：`tool::sdk_snapshot` **2 passed**；fixture sha `b89efd…`  
- dig：`dig-runtime-impl.md` / `dig-tools-teaching.md`

## Artifacts

- `acceptance.md` / `diff-summary.md`  
- `brief.md` / `plan.md` / `dig-survey.md` / `dig-runtime-impl.md` / `dig-tools-teaching.md`  
- `draft-webfetch.md` / `draft-websearch.md`  
- `cargo-final-wp203.txt` / `cargo-sdk-tools.txt` / `cargo-webfetch.txt` / `cargo-websearch-registry.txt` / `cargo-project-registry-avail.txt`

## 未做 / 不入库

- 不合 main；不改 `web/` / libs  
- 未入库脏 ops `board.json` / `events/` / `log.jsonl` / `tool_traces` / `board.md`  
- 未入库 WP-193/195/196 无关 artifacts
