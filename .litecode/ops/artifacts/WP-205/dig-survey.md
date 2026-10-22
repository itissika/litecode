# WP-205 dig：session_search 3 个既有失败测试

**结论：三个都是测试过时，不是产品缺陷。** 溢出文件真实写在点名路径；虚拟 read 路由正常返回 Ok，只是结果改为结构化 `data`。

复现（2026-10-22，available 3GiB，`cargo test -j 2 --lib session_search`）：198 passed / 3 failed / 1 ignored。

## 1+2. 溢出文件“读不到”（NotFound）
- 产品：`src/engines/session_search/mod.rs:924` footer 现为 `... are in {location}.`（句号收尾，下一行 `More in:`）。
- 历史：e918090 时为 `are in {location} — read or grep it.`；bd71f8c「去掉工具说明里对其他工具的点名调用」删掉了 ` — read or grep it`。
- 测试：`mod.rs:2103-2107`、`src/tools/session_search.rs:382-390` 仍用 `split(" —")` 截路径 → 拿到 `….txt.\nMore in: …`，`read_to_string` NotFound。
- `spill_slot`（mod.rs:1037）写入 `.litecode/bash/session_search_<nonce>.txt` 并以同一相对路径点名，路径本身正确。

## 3. hit 行与虚拟 read 不一致
- c688b0a（WP-199 slim read 契约）：read 改为 `ok_data`（`src/tool/outcome.rs:178`，content 为空，内容在 `data.lines`），并退役 `end_line`（read.rs:870/877，以 `start_line`+`limit` 取代）。
- 测试 `session_search.rs:568-594` 仍传 `end_line`、断言 `read.content` 含 token → level 是 Ok，content 空，589 处失败（不是“非 Ok”）。

## 修法（最小、通用）
1. 测试抽一个 helper 解析点名路径：取 ` are in ` 之后到首个空白/行尾，去掉末尾 `.`；两处共用。可选（产品侧小加固）：footer 用反引号包路径 `` are in `{location}`. ``，测试按反引号取，避免句号粘连——不要恢复点名其他工具的文案。
2. 虚拟 read 测试改用 `start_line` + `limit = end-start+1`，断言 `read.data["lines"]` 拼接后包含 token，且 `lines.len()` 与 hit 范围一致。
3. 不改 read / spill / transcript_file 产品逻辑。

## 涉及域
Tools（session_search 工具测试 + engines/session_search 视图测试）；与 Session/Runtime 无关。

## 风险
低。仅测试改动（可选 footer 反引号会改 session_search 输出文案，需同步其它断言 `are in .litecode/bash/session_search_` → 若加反引号要相应调整，并检查 sdk snapshot/teaching fixture 是否引用该文案）。根因是契约/文案变更没连带测试，建议后续契约改动时跑 session_search 过滤。
