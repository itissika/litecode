```node
node : tool-output-shaping
status : pending
summary : 公共出口语言：坐标、信号、截断/落盘/媒体与 blob 还原；工具内部也在整形。
x : 3726
y : 396
w : 326
h : 346
```

**主张：** 结果文本有共同语言——坐标、信号、截断与落盘、媒体、出站还原；但工具内部也在整形。

- 信号：Error / Warning / Hint 由 `compose_full` 一次合成，Warning 与 Hint 独立成块；Hint 只留给 LSP 是模块约定（[@ file="src/tool/signal.rs" symbol="fn compose_full"]、[@ file="src/tool/signal.rs" lines="1-8"]）。
- 坐标：文件行 1-based 与命中 offset 0-based 两套坐标不混；标签 `L12` / `L12-15`、`{path}:L12`（[@ file="src/tool/coords.rs" lines="1-52"]）。
- 截断：`max_result_size` 是 token 预算，`truncated_tool_result` 以 4 字节/token 近似；emergency cap 与 spill 按字节判断（[@ file="src/session/data/sqlite/session.rs" symbol="impl Session › fn truncated_tool_result" lines="2513-2520"]、[@ file="src/tool/output.rs" symbol="fn apply_emergency_cap"]）。
- 落盘与媒体：超 spill 阈值写 `data_root/blobs`，正文留 `[blob:id]` 与预览；媒体在 spill 前先物化（失败不留孤儿文本），LocalFile 读入 blob，图片→InputImage、音视频→InputFile，缺 blob / 未物化即 Error（[@ file="src/tool/output.rs" symbol="fn finalize_tool_call_result" lines="65-69"]、[@ file="src/tool/output.rs" symbol="fn materialize_media_parts" lines="80-118"]、[@ file="src/tool/executor.rs" symbol="fn input_content_from_artifact" lines="168-204"]、[@ file="src/session/media.rs" symbol="fn resolve_media_artifact_url"]）。
- 出站与边界：客户端路径经 `encode_client_item` 处理，把 `[blob:id]` 还原为当时落盘的正文——已截断、已受紧急上限，不是未截断原文（[@ file="src/client_protocol/controller/mod.rs" lines="150-153"]、[@ file="src/tool/output.rs" symbol="fn resolve_body_refs" lines="245-256"]）；工具自身也整形（如 `read` 行号前缀 [@ file="src/tools/read.rs" lines="507"]），`Tool::call` 辅助路径按同一预算先截断一次（[@ file="src/tool/trait_.rs" symbol="trait Tool › fn call"]）。

父节点：[@ key="tool-call-contract"]。
