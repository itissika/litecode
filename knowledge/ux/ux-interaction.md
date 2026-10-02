```node
node : ux-interaction
status : pending
summary : 三层交互模型：纯人类操作 / 人机协作（卡片阻塞循环）/ 纯 agent 可观测（按自身会话投影）。
x : 520
y : 890
w : 416
h : 449
```

**主张：** 同一内核两种交互形态：人类消费工作区基座，agent 消费 Tool；对人而言，交互分三层。

- 三层：纯人类操作（文件树 / 搜索 / Git / 会话 / 设置 / 知识库浏览）；人机协作（权限卡、对话输入）；纯 agent 可观测（流式、工具行状态、任务胶囊）。
- 协作卡：需要人类参与时用一张卡收敛为一次明确选择（单次 / 始终 / 拒绝）；卡片出现时循环阻塞等待回执；卡片是面板内的浮层，不是全屏模态、不在消息流内。
- 过程可见：agent 过程默认可见——流式与工具行状态即渲染依据；常驻任务胶囊（终端 / 子代理 / 计划 / 待办）；子代理按自身会话投影，不混流。
- 投影纪律：客户端只做投影，不发明第二套对话语义（死亡清单测试封禁旧方言）；流式平滑只是显示缓冲。
- 人类握有控制权：可打断、可回退、可授权；交互不止「看」。

上层：[@ id="ux-design" label="ux 设计"]。
相关：[@ id="ux" label="ux"]、[@ id="client-projection" label="客户端投影"]、[@ id="tool-pipeline" label="工具管线"]。
依据：[@ file="web/src/components/PermissionModal.tsx" label="协作卡"]、[@ file="web/src/components/SessionStatusLine.tsx" label="任务胶囊"]、[@ file="web/src/stores/connectionStore.ts" label="子会话事件隔离"]。
