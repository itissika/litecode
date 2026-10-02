```node
node : process-visibility
status : pending
summary : agent 过程默认可见：流式、工具行状态、plan/todo、subagent 按自身会话投影且不混流。
x : 986
y : 1404
```

**纯 agent 可观测：agent 自主行动，人类旁观而不打断。**

- 流式与工具：行状态即渲染依据；工具行有独立 live 状态与内联展示 [@ file="web/src/lib/transcriptProjection.ts" label="transcriptProjection.ts"]。
- 任务注意力：常驻四胶囊 terminal / subagent / plan / todo，plan 可一键执行 [@ file="web/src/components/SessionStatusLine.tsx" label="SessionStatusLine.tsx"]。
- 子代理：挂在父会话 launch item 下，按 child session 实时状态投影；child 的 turn / buffer / permission 事件不转发进父 turn 通道（连接层硬性忽略）[@ file="web/src/components/toolviews/SubagentLaunchToolView.tsx" label="SubagentLaunchToolView.tsx"] [@ file="web/src/stores/connectionStore.ts" label="connectionStore.ts"]。
- 可见性服务于信任；投影纪律见 [@ id="ui-projection-only" label="客户端只做投影"]，子会话语义见 [@ id="subagent-session-parity" label="子 agent 即完整会话"]。


