```node
node : tool-result-signals
status : enabled
summary : 信号只在出口拼一次：Error / Warning / Hint 各就各位，重复整形不变形。
x : 3024
y : 378
w : 420
h : 330
domain : tools-permission
```

**解决什么：** 工具只返回结构化字段，模型看到的是拼好的文本；前缀语法由出口统一生成，工具自己拼就会一套工具一个样。

**必须做到：** 三种信号各就各位，同一条结果无论经过几次整形，结果都不变。[compose_full](src/tool/signal.rs#compose_full)、[finalize_signals](src/tool/signal.rs#ToolCallResult.finalize_signals)

**实现细节：**

- Error 拼成 `Error: 正文`；Ok 原样给正文，不加前缀；Warning 可以带一条独立状态行，正文照常在前。[signal.rs:30-46](src/tool/signal.rs#L30-L46)
- Hint 永远独立成块，不粘在 Error / Warning 的尾巴上；appendix 跟在 Hint 后面。[signal.rs:48-59](src/tool/signal.rs#L48-L59)
- Hint 只给 LSP 用；其它指引写进正文，不另开第二个信号。[signal.rs:1-8](src/tool/signal.rs#L1-L8)
- 幂等靠 `composed` 标记：第一次拼完置位，再调用直接返回原样。[signal.rs:63-81](src/tool/signal.rs#L63-L81)
- 两条收口路径（`Tool::call` 与执行器）都走同一个 finalize，保证不会一处拼、一处不拼。[trait_.rs:90-94](src/tool/trait_.rs#L90-L94)、[executor.rs:174](src/tool/executor.rs#L174)

**绝对不允许：**

- 工具在正文里手写 `Error:` / `Warning:` / `Hint:`。
- 把非 LSP 的指引塞进 Hint。
- 在出口之外再拼一次信号，长出第二层前缀。

**守它的测试：** 五个单测，全部是精确字符串比对，改一个字符就红：

- 前缀只加一次。[error_and_ok_prefixes_are_not_doubled](src/tool/signal.rs#error_and_ok_prefixes_are_not_doubled)
- Warning 状态行独立成块。[warning_status_is_its_own_block_after_the_body](src/tool/signal.rs#warning_status_is_its_own_block_after_the_body)
- Hint 永远不粘在状态行尾巴上。[hint_is_a_separate_block_never_glued_onto_a_status_line](src/tool/signal.rs#hint_is_a_separate_block_never_glued_onto_a_status_line)
- appendix 跟在 Hint 后面。[appendix_follows_the_hint_block](src/tool/signal.rs#appendix_follows_the_hint_block)
- 重复整形结果不变。[finalizing_twice_leaves_the_text_unchanged](src/tool/signal.rs#finalizing_twice_leaves_the_text_unchanged)

上游：[tool-output-shaping](knowledge/contracts/tools/tool-output-shaping.md)。
