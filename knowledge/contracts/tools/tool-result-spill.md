```node
node : tool-result-spill
status : enabled
summary : 超限只在唯一出口截一次、落一次盘：缺口给精确行号，完整事实留在盘上。
x : 3024
y : 756
w : 440
h : 380
domain : tools-permission
```

**解决什么：** 输出会超长，超长不能靠每个工具自己截、自己想办法；必须由主管线按固定预算截一次，并把完整事实留在可再取的地方。

**必须做到：** 工具先在自己的粒度上截断，剩下的由主管线统一落盘；缺口必须给精确行号，模型视图严格不超预算。[cut_and_spill_lines](src/tool/output.rs#cut_and_spill_lines)、[spill_full_text](src/tool/output.rs#spill_full_text)

**实现细节：**

- 工具侧先截：read 默认 2000 行、最多 5000 行、单行 1500 字符；grep 默认 40 条、最多 100 条、单行 500 字符、另有 8K token 数据护栏；glob 最多 5000 条。[read.rs:11-14](src/tools/read.rs#L11-L14)、[grep.rs:29-36](src/tools/grep.rs#L29-L36)、[glob.rs:15](src/tools/glob.rs#L15)
- 管线预算：`RUN_CODE_VIEW_MAX_BYTES = 20_480`；先扣 header 与 manifest，再给上浮行留 220 字节，保证最终文本不超预算。[render.rs:30](src/tools/code/render.rs#L30)、[render.rs:141-190](src/tools/code/render.rs#L141-L190)
- 切法：50/50 首尾、按行对齐，中段只留一行 `… lines A-B omitted`。[output.rs:62-73](src/tool/output.rs#L62-L73)、[output.rs:110-112](src/tool/output.rs#L110-L112)
- 落盘：一次写入 `.litecode/run/run_code_<nonce>.txt`；上浮行给出路径和精确的缺口行号范围；写不了盘就只给缺口，不假装完整。[output.rs:27-38](src/tool/output.rs#L27-L38)、[output.rs:84-94](src/tool/output.rs#L84-L94)
- 全仓只有一个落盘点；执行器只对超预算打日志，不二次落盘。[render.rs:185](src/tools/code/render.rs#L185)、[executor.rs:789-800](src/tool/executor.rs#L789-L800)
- 子工具不落盘：`exec_command` 前台只回 `{exit_code, output}`；后台日志留在 `.litecode/run/<job>.stdout`，上限 8 MiB，退出时用 `bash_exit` 通知。[exec_command.rs:13-15](src/tools/exec_command.rs#L13-L15)、[tee.rs:12](src/terminal/tee.rs#L12)

**绝对不允许：**

- 工具自己截断后落盘，或自己发明省略号。
- 在唯一出口之外再写一份全文。
- 超限了还假装输出完整：没有缺口行号，也没有可再取的路径。

**守它的测试：**

- 静态清单：`src/` 里出现自造落盘字符串即失败。[death_list_no_bespoke_spill_or_bypass_in_src](tests/unified_output_spill_invariants.rs#death_list_no_bespoke_spill_or_bypass_in_src)
- 2 MB 流只从 run_code 视图落一次盘。[run_code_2mb_stream_succeeds_and_spills_full_output_to_litecode_run](tests/unified_output_spill_invariants.rs#run_code_2mb_stream_succeeds_and_spills_full_output_to_litecode_run)
- 直接 inspect 也只在 run_code 出口落盘。[exec_command_direct_inspect_spills_only_through_run_code_view](tests/unified_output_spill_invariants.rs#exec_command_direct_inspect_spills_only_through_run_code_view)
- knowledge 工具永不落盘。[knowledge_tool_never_spills_to_disk](tests/unified_output_spill_invariants.rs#knowledge_tool_never_spills_to_disk)
- 全仓只有一个落盘点：`spill_full_text` / `cut_and_spill_lines` 只允许出现在 `output.rs` 与 `render.rs`，且渲染侧恰好一处调用。[only_the_model_view_spills_tool_output](tests/unified_output_spill_invariants.rs#only_the_model_view_spills_tool_output)

上游：[tool-output-shaping](knowledge/contracts/tools/tool-output-shaping.md)。相关：[tool-result-render](knowledge/contracts/tools/tool-result-render.md)。
