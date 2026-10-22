# WP-209 measurements — run_code result token cost, before vs after

计数器：`session::estimate::count_text_tokens`（cl100k，tiktoken-rs，仓内现成）。
before = 基线 90654c1 的 result 正文（`serde_json::to_string_pretty`；logs 为基线 `logs:` 全量块）；
after = 新渲染（`render_result_body` 整形 + `budget_result` 8k；成功 logs → `logs omitted (N lines)`；失败 logs 尾部 2k）。
数据：本仓真实内容（read=render.rs 前 200 行；grep=`rg -n "fn " src/tools` 前 50 条；glob=`git ls-files src` 前 200；LSP symbols=lsp.rs 54 个符号；run=全量 cargo 日志尾 120 行，按 RunData 返回）。临时测量测试已删除，未提交。

| 典型调用（返回原 data） | before bytes | before tok | after bytes | after tok | 节省 |
|---|---:|---:|---:|---:|---:|
| read 200 行 | 8,972 | 2,497 | 7,952 | 2,113 | 15% |
| grep 50 hits（hits → 表格） | 6,631 | 1,987 | 3,612 | 1,009 | 49% |
| glob 200 paths | 7,431 | 2,077 | 6,419 | 1,668 | 20% |
| LSP document_symbol 54 个（→ 表格） | 6,272 | 1,886 | 1,913 | 572 | 70% |
| apply_edits 5 条（→ 表格） | 446 | 144 | 143 | 54 | 63% |
| run 输出（~70KB，超限） | 69,516 | 9,525 | 57,999 | 8,038* | 16%（截到 8k） |
| grep 映射为 {path,line} 50 行 | 3,026 | 1,074 | 1,240 | 378 | 65% |
| logs 300 行，成功 | 8,295 | 2,701 | 24 | 6 | ~100% |
| logs 300 行，失败 | 8,295 | 2,701 | 6,203 | 1,997 | 26%（尾部 2k） |
| CJK 字符串 90KB | 90,000 | 20,000** | 36,108 | 8,038* | 60% |

\* 含开头超限提示行与尾部 `… [truncated: X of Y tok]` 标记（约 30–40 tok，不计入 8k 正文预算，Q5）。
\** 基线按字节截到 64KB 视口，实际进上下文约 14k+ tok；新渲染按 token 封顶，修复 CJK 欠保护。

结论：表格形态对同构对象数组（grep hits / symbols / apply_edits / 映射结果）节省 49–70%；纯字符串数组（read lines / glob paths）只省缩进约 15–20%；成功 logs 基本归零；超大与 CJK 结果被硬封在 8k tok。
