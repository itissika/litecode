```node
node : tool-output-shaping
status : pending
summary : 公共出口语言：坐标、信号、截断/落盘/媒体与 blob 还原；工具内部也在整形。
x : 2394
y : 486
w : 326
h : 346
domain : tools-permission
```

**主张：** 结果文本有共同语言——坐标、信号、截断与落盘、媒体、出站还原；但工具内部也在整形。

- 信号：Error / Warning / Hint 由 `compose_full` 一次合成，Warning 与 Hint 独立成块；Hint 只留给 LSP 是模块约定（[@ file="src/tool/signal.rs" symbol="fn compose_full"]、[@ file="src/tool/signal.rs" lines="1-8"]）。
- 坐标：文件行 1-based 与命中 offset 0-based 两套坐标不混；标签 `L12` / `L12-15`、`{path}:L12`（[@ file="src/tool/coords.rs" lines="1-52"]）。
- 截断与落盘：只有外层 `run_code` 模型视图做唯一出口的超限落盘（上限 `RUN_CODE_VIEW_MAX_BYTES` = 20 KB，不设行上限）；超限时将未截断全文一次写入工作区 `.litecode/run/run_code_<nonce>.txt`，正文保留按行对齐的 50/50 首尾预览与极简缺口行 `… lines A-B omitted`，顶部 `↳ ` 上浮提醒给出完整路径与精确 1-based 缺口行号范围（[@ file="src/tool/output.rs" symbol="fn spill_full_text"]、[@ file="src/tool/output.rs" symbol="fn cut_and_spill_lines"]、[@ file="src/tools/code/render.rs" symbol="fn render_model_view_in"]）；前台子工具不做独立落盘，`exec_command` 仅返回 `{ exit_code, output }`（前台结束后清理临时 tee 文件，仅后台运行/超时转后台时在 `.litecode/run/<job>.stdout` 保留日志并通过上浮提醒与 `bash_exit` 暴露路径，上限 `FILE_MAX` = 8 MiB）（[@ file="src/tools/exec_command.rs" symbol="struct ExecCommandTool"]、[@ file="src/terminal/tee.rs" symbol="fn read_tail_utf8_clean"]）；`knowledge` 不再做分页或落盘（[@ file="src/tools/knowledge.rs"]）。
- 媒体与历史存储：模型侧仅暴露 `run_code`；嵌套工具产生媒体经 `MediaRegistry` 登记为 `__litecode_media` 句柄，仅当句柄被 `run_code` 程序显式 `inspect` 出沙箱时才收集交付给外层渲染（`不 inspect 不给`），外层出口经 `finalize_direct_tool_call_result` 与 `materialize_media_parts` 物化（LocalFile 读入 blob，图片→InputImage、音视频→InputFile，缺 blob / 未物化即 Error）（[@ file="src/tools/code.rs" symbol="struct MediaRegistry"]、[@ file="src/tools/code.rs" symbol="impl MediaRegistry › fn collect_inspected_parts"]、[@ file="src/tool/output.rs" symbol="fn finalize_direct_tool_call_result"]、[@ file="src/tool/output.rs" symbol="fn materialize_media_parts"]、[@ file="src/tool/executor.rs" symbol="fn input_content_from_artifact"]、[@ file="src/session/media.rs" symbol="fn resolve_media_artifact_url"]）。
- 出站与边界：客户端路径经 `encode_client_item` 与 `resolve_body_refs` 解析遗留持久化消息（[@ file="src/client_protocol/controller/mod.rs" lines="150-153"]、[@ file="src/tool/output.rs" symbol="fn resolve_body_refs"]）；工具自身整形之外，`Tool::call` 只做信号收口、不在调用前做无路径截断，外层 `run_code` 超限统一在执行器出口落盘（[@ file="src/tool/trait_.rs" symbol="trait Tool › fn call"]、[@ file="src/tool/executor.rs"]）。

父节点：[@ key="tool-call-contract"]。
