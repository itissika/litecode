```node
node : ui-projection-only
status : pending
summary : 客户端只做投影：行生命周期是日志自己的字段；死亡清单测试封禁第二套对话语义。
x : 1224
y : 1404
```

**内核为真源，客户端只做投影。**

- 行的生命周期是日志自己的字段：wire 上随行携带 log state；行缺 state 即判非法，绝不猜「已定稿」[@ file="web/src/api/adapter.ts" label="adapter.ts"] [@ file="web/src/api/types.ts" label="types.ts"]。
- 「死亡清单」测试封禁第二套对话语义：`ToolStart` / `ToolEnd` / `liveTools`、`partialText` / `partialReasoning`、`revert_messages` 等旧方言不得出现在生产代码 [@ file="web/src/api/r4-death-list.test.ts" label="r4-death-list.test.ts"] [@ file="web/src/api/r5-death-list.test.ts" label="r5-death-list.test.ts"]。
- 流式平滑只是显示缓冲，不制造第二真源 [@ file="web/src/lib/streamingBuffer.ts" label="streamingBuffer.ts"]。
- 设置面同构：以服务器应答为准，持久化失败回滚 [@ file="web/src/lib/settingsPersist.ts" label="settingsPersist.ts"]。
- 投影的源头是会话日志 [@ id="session-log-truth" label="日志为真源"]。


