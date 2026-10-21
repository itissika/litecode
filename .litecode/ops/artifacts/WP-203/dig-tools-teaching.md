# WP-203 Tools 教法 — dig（收口完成）

> **作者：** Tools  
> **时间：** 2026-10-22 Asia/Hong_Kong  
> **分支：** `feat/tool-contract-ptc`（不合 main；**未 commit**）  
> **阶段：** **收口完成** — live `webfetch`/`websearch` 整份替换；PRELUDE T5；刷 `run_code_sdk`；sdk_snapshot 绿。  
> **依据：** `brief.md`、`plan.md`、`dig-runtime-impl.md`；mid T1–T7。

## 0. 总评

| 项 | 状态 |
|---|---|
| 单轨 | **只教** Code `tools.webfetch` / `tools.websearch`；禁 Direct 顶层调用 |
| 软失败 | HTTP/DNS/超时/Exa 业务 → resolve `{ok:false,…,message}` |
| 硬错 | 非法入参 / SSRF 真拒 / 未 warm·未配 Exa → `ToolError`（`invalid_input` / `permission_denied` / `unavailable`） |
| TUN | 198.18/15 **可过门**；RFC1918/loopback/link-local/ULA/`100.64/10` **仍拒** |
| T1 | **无** `results[]`；只教 `text` |
| T2 | body 截断 **50000** |
| T4 | 成功 message 跟 Runtime 终态：`Fetched.` / `Search completed.`（短固定词；非 T4 示例小写） |
| live / SDK | **已叠**；sdk_snapshot **2 passed**；sha 见下 |
| web/ | **未改** |

## 1. 交付路径

| 路径 | 说明 |
|---|---|
| `src/tools/descriptions/webfetch.md` | live 整份替换（Code；软/硬；50000；`Fetched.`） |
| `src/tools/descriptions/websearch.md` | live 整份替换（Code；只教 `text`；`Search completed.`） |
| `src/tool/code_manifest.rs` PRELUDE | **T5** 点名 webfetch/websearch 软 vs 硬句 |
| `src/tool/fixtures/run_code_sdk.txt` | 渲染自当前 HEAD（含两名 + PRELUDE） |
| `src/tool/fixtures/run_code_sdk.sha256` | `b89efddee23d3d398aee0607ccefe5c98ca2a50b10229e074319ca8f571e3c33` |
| `.litecode/ops/artifacts/WP-203/dig-tools-teaching.md` | 本笔记（终态） |
| `.litecode/ops/artifacts/WP-203/cargo-sdk-tools.txt` | UPDATE + VERIFY tee |
| `draft-*.md` | 草稿保留对照；live 已覆盖（成功 message 已跟 Runtime） |

## 2. 契约摘要（与 Runtime dig / T1–T7 对齐）

```ts
tools.webfetch({ url, format? /* text|markdown|html，默认 markdown */ })
  -> { ok: true,  url, status, format, body, message: "Fetched." }
  |  { ok: false, url?, status?, format?, message }  // 无 body；status 仅有 HTTP 响应时

tools.websearch({ query })
  -> { ok: true,  query, text, message: "Search completed." }  // 无 results[]
  |  { ok: false, query?, message }
```

- **Code-only**；入参 **object bag**；两名已在 `builtin_code_surface_tools()`（Runtime）。  
- **T5 PRELUDE：** 点名两工具——网络/Exa 软失败 `ok:false` 仍 resolve；非法/真 SSRF/未 warm 仍 ToolError。

## 3. cargo

```bash
MemAvailable ≥2 GiB；CARGO_BUILD_JOBS=2

UPDATE_SDK_FIXTURE=1 cargo test --lib tool::sdk_snapshot::phase0_run_code_sdk_snapshot -- --exact
→ UPDATED sha=b89efdde…；EXIT 0

cargo test --lib tool::sdk_snapshot -- --test-threads=2
→ 2 passed（phase0 + sdk_hash_is_stable…）；EXIT 0
```

tee → `artifacts/WP-203/cargo-sdk-tools.txt`。UPDATE hook **已从** `sdk_snapshot.rs` **撤回**。

首轮 VERIFY 因示例字段名 `preview` 误触 `!contains("view")` 失败；已改为 `snippet` 后重刷通过。

## 4. 残留扫描

- live descriptions：无 Direct 顶层口吻残留（明确「no Direct webfetch/websearch」）  
- 无 `results[]` 教法  
- SDK fixture 含 `webfetch` / `websearch` bindings + 新 PRELUDE 句  
- **未改** `web/` / Runtime 执行 / SSRF 分类器

## 5. 约束 / 未做

- **未** commit / push / 合 main  
- **未**改 Runtime 业务代码（仅加深 description + PRELUDE + fixture）  
- 成功 message 字符串以 Runtime 实现为准（`Fetched.` / `Search completed.`），非 mid T4 举例小写

## 6. Blockers

无。可交 mid 收口 commit/deliver。
