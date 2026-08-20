# 08: 围栏补全与全库针 + 编译绿

**What to build:** 在身份已正确的前提下，一次唤醒的 loop 用 `turn`/`step` 围起来，每步请求信封能从日志重建（`request/header` 变化才写）。取消是把已有 seq 标 incomplete 并 `turn/end`，不是丢行。此时死亡清单全仓库绿，才要求 `cargo check` 与 `npx tsc -b` 绿。durable `assistant/chunk` 可选；不做也必须在类型/文档上显式「未实现」，不得静默缺失，更不得因此复活 overlay。

**Blocked by:** 05: 边路只消费 seq / surface; 07: 转写投影与 FoldCard 只看该 seq

**Status:** ready-for-agent

- [ ] loop 写 `turn/start|end`、`step/start|end`；取消补 `turn/end`（interrupted/aborted）且 surface 不另算一套
- [ ] 每步 dispatch 前可重建 `fold_request_header(log) ∪ derive_messages(surface)`
- [ ] seq 连续、replace 范围合法等不变量有伴生检查（加载可拒读未知非 ignorable type）
- [ ] `MENTAL-MODEL` §4 词汇在事件 type 中有对应，或本票显式修订「chunk 未实现」
- [ ] G7：全仓库死亡清单绿；§12 对照表无残留实现（含 store 上已无读者的 `buffer_index` / `load_turn_transcript` 窗口 SQL 等死皮，05 未删的在此删）
- [ ] 构建门禁此刻才必绿：`cargo check`、`npx tsc -b`；本专题相关测试绿（05 点名的 search/revert、协议、面板）。无关的本机 flake（git CRLF、code-search warmup）不挡完成
