```node
node : tool-result-render
status : enabled
summary : 渲染由管线统一决定：能表格化就表格化，否则 compact JSON；提醒只补数据说不出来的部分。
x : 3024
y : 1170
w : 460
h : 420
domain : tools-permission
```

**解决什么：** 同一份数据，写成 JSON 还是表格、要不要带提醒，直接决定 token 和返工；所以渲染规则必须由管线统一决定，不能由每个工具自己发挥。

**必须做到：** 能表格化就表格化，否则 compact JSON；只对数据说不出来的东西给提醒。[render_result_body](src/tools/code/render.rs#L1045-L1085)

**实现细节：**

- 表格条件：所有行都是扁平对象、列数不超过 16、单元格单行且不嵌套，才转成 `key[N]{列,列}:` 表；否则整个值退回 compact JSON。[table_columns](src/tools/code/render.rs#L1214-L1241)、[render_table](src/tools/code/render.rs#L1255-L1268)
- 单元格只在有歧义时加引号：空、逗号或引号或控制符、首尾空白、看起来像数字 / 布尔 / null。[cell_text](src/tools/code/render.rs#L1278-L1291)
- 字符串列表：两条短的留在 JSON 里，三条以上或长的 / 多行的按行给，不转义。[render_string_array](src/tools/code/render.rs#L940-L962)
- 工具专项：read 的 `lines:` 块（空时 `lines: (empty)`）；exec_command 的 `exit_code: N` 加原样 output，后台时是 `(running in background)`。[render_read_lines_object](src/tools/code/render.rs#L964-L983)、[render_exec_output_object](src/tools/code/render.rs#L985-L1017)
- 提醒：`↳ #N tools.x: 文本`，单行上限 120 字符；内容只补数据说不出来的——续页参数、缺口行号、完整文件路径、被夹紧的值、数据可能不完整。[render.rs:91-97](src/tools/code/render.rs#L91-L97)、[reminder_line](src/tools/code/render.rs#L919)
- 克制：正常空结果不给提醒（`no_match` / `no_files` 映射为空）；`clamped` 先从模型可见数据里移走，只留在提醒里，避免同一事实出现两次。[render.rs:541-555](src/tools/code/render.rs#L541-L555)、[result_projection.rs:99-127](src/tool/result_projection.rs#L99-L127)
- 坐标：文件行 1-based，命中 offset 0-based；标签 `L12` / `L12-15` / `{path}:L12`；行号列表最多 8 个，再多补 `(and N more)`；续页用 `More hits: pass offset=N.`；列不进视图。[coords.rs:1-55](src/tool/coords.rs#L1-L55)

**绝对不允许：**

- 工具自己渲染最终文本，或自己决定摘要。
- 数据里已经有的东西，再在提醒里说一遍。
- 为某个工具发明第二套表格或标签语法。

**守它的测试：**

- 表格：整表渲染、列数超限退回 JSON、嵌套值不硬塞、歧义单元格加引号。[uniform_object_array_renders_as_a_table](src/tools/code/render.rs#uniform_object_array_renders_as_a_table)、[too_many_columns_fall_back_to_compact_json](src/tools/code/render.rs#too_many_columns_fall_back_to_compact_json)、[table_nested_in_an_object_renders_as_a_keyed_block](src/tools/code/render.rs#table_nested_in_an_object_renders_as_a_keyed_block)、[table_cells_quote_ambiguous_strings](src/tools/code/render.rs#table_cells_quote_ambiguous_strings)
- 专项渲染：read 的 `lines:` 块、exec 的退出码块、字符串数组按行给。[read_lines_object_renders_verbatim_without_json_escaping](src/tools/code/render.rs#read_lines_object_renders_verbatim_without_json_escaping)、[render_result_body_formats_exec_output_object_cleanly](src/tools/code/render.rs#render_result_body_formats_exec_output_object_cleanly)、[render_result_body_renders_string_arrays_one_per_line](src/tools/code/render.rs#render_result_body_renders_string_arrays_one_per_line)
- 提醒：顺序在流之上、单行限长、克制（`no_match` / `no_files` 不给行）。[call_reminders_sit_above_the_stream_in_call_order](src/tools/code/render.rs#call_reminders_sit_above_the_stream_in_call_order)、[a_reminder_is_one_short_line](src/tools/code/render.rs#a_reminder_is_one_short_line)、[grep_fields_and_their_reminder_lines](src/tools/code/render.rs#grep_fields_and_their_reminder_lines)
- `clamped` 先移出数据、再只从 metadata 进提醒。[read_moves_paging_to_metadata_and_drops_path](src/tool/result_projection.rs#read_moves_paging_to_metadata_and_drops_path)、[exec_reminders_surface_background_handoff_and_clamp_from_metadata](src/tools/code/render.rs#exec_reminders_surface_background_handoff_and_clamp_from_metadata)
- 坐标：标签格式、行号列表上限 8、offset 续页。[labels_use_one_l_prefix_and_collapse_equal_ranges](src/tool/coords.rs#labels_use_one_l_prefix_and_collapse_equal_ranges)、[line_lists_cap_at_eight_and_report_the_rest](src/tool/coords.rs#line_lists_cap_at_eight_and_report_the_rest)、[hit_offsets_are_zero_based_continuation_tokens](src/tool/coords.rs#hit_offsets_are_zero_based_continuation_tokens)

上游：[tool-output-shaping](knowledge/contracts/tools/tool-output-shaping.md)。相关：[tool-result-spill](knowledge/contracts/tools/tool-result-spill.md)。
