# WP-200 Tools 教法 — dig（收口完成）

> **作者：** Tools  
> **时间：** 2026-10-22 Asia/Hong_Kong  
> **分支：** `feat/tool-contract-ptc`（不合 main；**未 commit**）  
> **阶段：** **收口完成** — PRELUDE / write / coords 对齐三块模型面；刷 `run_code_sdk`；sdk_snapshot 绿。  
> **依据：** `brief.md`、`plan.md`；中层拍板 A–F；Runtime `dig-runtime-impl.md`（lib tools::code 18 + run_code_model_view 17）。

## 0. 总评

| 项 | 状态 |
|---|---|
| 目标教法 | **只教** 默认模型面三块（自然语言）：短头 +（自愿）过程日志 + 返回值；标签跟 Runtime（`logs:` / `result:`） |
| 禁止教法 | 独立 failure 大段、`diagnostics:` 层、全量 ledger、`error_details`、**outer ledger / outer text** 观测导向 |
| 歧义 A–F | **已拍板并落地** |
| live / SDK | **已叠**；fixture sha `58cd2f33a5111ed0bcf6b6ed662f4338f8582fbd11314e091ce18a3650a49a6e` |
| cargo | `tool::sdk_snapshot` **2 passed**（phase0 + hash stable） |

## 1. 落地教法（唯一）

`run_code` 是**单次聚合管线**，不是调试观测台：

1. **短头（不变量，一两行）** — 已结算成败数 + **ms** 耗时；未启动则行/列+错因；unsettled 旁注同头不计成败；session-store 可读短注。  
2. **过程日志（自愿）** — 仅脚本自写；无则整段省略。  
3. **返回值** — 按窗保留；失败不空抹 partial。

LSP / 工具侧 diagnostics：**不进** resolve data；需可见自行 log/return；勿默认塞 Head。  
契约错误留在 `{ok,message}` / `ToolError`；勿指望外层同义复述。

## 2. 改动文件

| 路径 | 说明 |
|---|---|
| `src/tool/code_manifest.rs` | PRELUDE：去 outer text；自然语言三块；LSP 自 log/return |
| `src/tools/descriptions/write.md` | 去 outer text；LSP 不在 resolve |
| `src/tool/coords.rs` | `FILE_LINE_PREFIX_HINT`：outer ledger → **transcript or read output** |
| `src/tool/fixtures/run_code_sdk.txt` | 渲染自当前 HEAD |
| `src/tool/fixtures/run_code_sdk.sha256` | `58cd2f33a5111ed0bcf6b6ed662f4338f8582fbd11314e091ce18a3650a49a6e` |
| `.litecode/ops/artifacts/WP-200/cargo-sdk-tools.txt` | UPDATE + VERIFY 日志 |
| `.litecode/ops/artifacts/WP-200/dig-tools-teaching.md` | 本笔记（终态） |
| `.litecode/ops/artifacts/WP-200/draft-run_code-view.md` | 草稿留档（已折进 live） |

## 3. 残留扫描（教法面）

对 `code_manifest` / `coords` / `descriptions/` / `run_code_sdk.txt`：

- **无** `outer ledger` / `outer text`  
- **无** 默认面 `calls (` / `diagnostics:` 段导向  
- `lsp.md` 的 diagnostics **动作**保留（LSP 产品能力 ≠ run_code 模型面段）  
- UPDATE hook **已从** `sdk_snapshot.rs` **撤回**

## 4. cargo

```bash
MemAvailable ≥10 GiB；CARGO_BUILD_JOBS=2

UPDATE_SDK_FIXTURE=1 cargo test --lib tool::sdk_snapshot::phase0_run_code_sdk_snapshot -- --exact
→ EXIT 0；sha=58cd2f33…

cargo test --lib tool::sdk_snapshot -- --test-threads=2
→ 2 passed（phase0 + sdk_hash_is_stable…）
```

tee → `artifacts/WP-200/cargo-sdk-tools.txt`。

## 5. 约束 / 未做

- **未** commit / push / 合 main  
- **未**改 Runtime 渲染或 file/shell 契约（除 coords 用词）  
- Runtime 模型面测（18+17）由 Runtime 已绿，本包只动教法 + SDK fixture

## 6. Blockers

无。可交 mid 收口 commit/deliver。
