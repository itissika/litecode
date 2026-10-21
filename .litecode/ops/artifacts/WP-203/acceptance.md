# WP-203 acceptance

**口径：** Andy + GM；`webfetch` / `websearch` → Code-only；名不变；软失败 `{ok:false}` resolve；硬错 ToolError；TUN 放行 `198.18/15`，拒真私网/回环/100.64。  
**分支：** `feat/tool-contract-ptc`（实验；**不合 main**）  
**方案：** `plan.md`（Q1A 软失败 / Q2A fake-ip / Q3 名不变 / Q4A 结构化 data / Q5B 硬错 ToolError）  
**基线：** `b38ebc4`（WP-202 tip）  
**本 tip：** `4f7b755ed3a8e1c3e323e4c41fb99c40db7ebc53`

| # | done_when | 结果 |
|---|-----------|------|
| 1 | 两工具进 Code；软失败可证（测例） | **通过**。`webfetch` / `websearch` → `ToolSurface::Code`；进 `builtin_code_surface_tools`；Direct 清零（`webfetch_websearch_project_to_code_not_direct`）。HTTP/DNS/超时/Exa → `ok_data` `{ok:false}`；非法入参/非 http(s)/真 SSRF/未 warm·未配 → ToolError（`invalid_input` / `permission_denied` / `unavailable`）。出参：fetch `{ok,url,status?,format?,body?,message}`；search `{ok,query,text,message}`（无 `results[]`）。 |
| 2 | TUN/过度防御有 dig 结论或修复 | **通过**。SSRF 放行 `198.18.0.0/15`（`ssrf_allows_hostname_resolved_to_fake_ip` / `ssrf_allows_benchmarking_fake_ip_range`）；仍拒 RFC1918 / loopback / fe80 / ULA / `100.64/10`。契约 `knowledge/contracts/tools/webfetch-ssrf.md` 已同步。 |
| 3 | cargo 绿；artifacts/WP-203；push 实验分支；未合 main；无 FE | **通过**。测数下（本收口**未** cargo 重跑）；artifacts 齐；push 实验分支；**未合 main**；**未改 `web/`**；未改 libs。 |

## Q1–Q5 定稿

| # | 议题 | 定稿 |
|---|------|------|
| Q1 | 软失败形态 | **A** `ok:false` 软失败（resolve，不 reject） |
| Q2 | TUN / SSRF | **A** 放行 `198.18/15`；拒真私网/回环/`100.64` |
| Q3 | 工具名 | 不变：`webfetch` / `websearch` |
| Q4 | 出参 | **A** 结构化 `data`（上列出参形状） |
| Q5 | 硬错边界 | **B** 未 warm·SSRF 真拒·非法入参 → ToolError |

## 测数（artifacts 日志；本收口未整仓重跑）

| 日志 | 结果 |
|---|---|
| `cargo-final-wp203.txt` | **34 passed**（EXIT 0；含 `ssrf_allows_hostname_resolved_to_fake_ip`、`webfetch_websearch_project_to_code_not_direct`） |
| `cargo-sdk-tools.txt` | `tool::sdk_snapshot` **2 passed**（EXIT 0）；fixture sha `b89efddee23d3d398aee0607ccefe5c98ca2a50b10229e074319ca8f571e3c33` |

## 出参形状（钉死）

- webfetch：`{ok,url,status?,format?,body?,message}`；body 上限 50000；成功 message：`Fetched.`  
- websearch：`{ok,query,text,message}`（**无** `results[]`）；成功 message：`Search completed.`  
- 软：HTTP/DNS/超时/Exa → resolve `{ok:false,…,message}`  
- 硬：非法/非 http(s)/真 SSRF/未 warm·未配 → ToolError（无 ok:false）

## Digs

- `dig-runtime-impl.md`（Runtime 终态）  
- `dig-tools-teaching.md`（Tools 教法 / SDK）  
- `dig-survey.md` / `brief.md` / `plan.md` / `draft-webfetch.md` / `draft-websearch.md`

## 已知推迟（非本包 blocker）

- 无（Settings 卡 / Direct 投影均已按方案落地；本包无 stamp 另开项）。

## 非目标 / 未入库

- **不合 main**；**不改 `web/`**；不改 libs；不改 LSP  
- 未 stage ops `board.json` / `events/` / `log.jsonl` / `tool_traces` / `board.md`  
- 未入库 WP-193/195/196 无关 artifacts
