> **已折进 live（WP-200 收口）** — 以下留档对照；以 PRELUDE / write.md / coords.rs 为准。

# WP-200 draft — run_code 默认模型面教法（artifacts only，非 live）

> 待命草稿；歧义 **A–F 已拍板**。等 Runtime 硬清 + 中层「放 Tools 收口」后再折进 PRELUDE / `write.md` / `coords.rs`。  
> **不要**复制进 live，直到收口。

## 教 agent 什么（自然语言；标签跟 Runtime）

`run_code` 跑完后，默认只看到三块（有则出现，无则省略；渲染标签跟实现，例如 `logs:` / `result:`，不强迫固定 Head/Logs/Result 词）：

1. **短头（一两行）**  
   - 没跑起来：哪一行（+列若有）什么错。  
   - 跑起来了：已结算工具调用的成功数 / 失败数 / 耗时（单位跟实现，倾向 ms）。**unsettled 不计入**成败数；若有仍在跑的调用，短头内旁注一两行（不另开段）。  
   - 过程失败：同一短头带简短标签（timeout、uncaught ToolError 等），不要找独立 failure 大段。  
   - session-read：若产品要可见，最多短头一句；否则教法不提（可能只在内部通道）。
2. **你自己打印的过程日志** — 没有就不出现；成功的嵌套工具调用不会自动占一行。需要过程就 `console.log` / `return`。  
3. **你 `return` 的值** — 按窗尽量保留；失败也不空抹已有 partial。

## 不要指望什么

- 独立 failure 长文、`diagnostics:` 列表、`calls (N of M shown)` 全量账本、把工具 `message` 再复述一遍的 error_details。  
- 用模型面文本当 `ledger_reads_sessions` 或任何 ledger 探测。  
- LSP 错误自动出现在工具 resolve data，或默认塞进短头 diagnostics；要看就自己 log/return（Runtime 若另写一句 head 摘要再跟）。  
- 外层再包一层与 `{ok,message}` / `ToolError` 同义的说明。

## PRELUDE 意向句（收口时替换相关旧句）

自然语言示意（数字格式 / 标签以 Runtime dig 终态为准，教法不发明）：

```
After a run you normally see a short head (settled tool-call success and failure counts and how long the run took—duration unit follows the host; or a precise line/error if the program never started; unsettled in-flight calls, if any, are noted in that same head and are not counted in those totals), optional logs you printed yourself, and your returned result (kept within the run window; failures keep partial output when possible). Labels follow the host render (for example logs: / result:). Successful nested tool calls are not auto-listed. LSP and other tool-side diagnostics are not in the tool's resolved data—log or return them if you need them visible. Tool contract errors stay on the tool ({ok,message} or ToolError)—do not expect a second copy outside. Return a summary, not a raw bulk of data.
```

删除/改写旧导向：

- ~~LSP errors … outer text~~ → 不进 resolve；需可见自行 log/return；勿默认 Head diagnostics。  
- ~~outer ledger~~（`coords.rs` / edit 字段）→ **transcript or read output with line-number prefixes**（禁 ledger）。

## 示例取向（防退化）

```js
const edited = await tools.edit({ path, old_string, new_string });
if (!edited.ok) return { edited };
const cmd = await tools.run({ command: "cargo test -q" });
// Need process detail? console.log(cmd.exit_code, cmd.output.slice(0, 500));
return { edited, exit_code: cmd.exit_code };
```

短头会概括已结算成败与耗时；不要假设还能翻 ledger 逐行对账。
