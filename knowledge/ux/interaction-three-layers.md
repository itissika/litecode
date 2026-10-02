```node
node : interaction-three-layers
status : pending
summary : 三层交互模型：纯人类操作 / 人机协作（卡片阻塞循环）/ 纯 agent 可观测（按自身会话投影）。
x : 510
y : 1404
```

| 层 | 含义 | 现状承载 |
|----|------|----------|
| **纯人类** | 人类独立操作，agent 不参与 | 文件树 / 搜索 / Git / 会话列表 / 设置 / 知识库浏览 |
| **人机协作** | 双方面向同一目标互动 | 权限确认卡（loop 阻塞）、对话输入 |
| **纯 agent 可观测** | agent 自主行动，人类旁观 | 流式输出、tool 行状态、terminal / subagent / plan / todo 胶囊、subagent 面板 |

- 协作态用一张卡收敛为一次明确选择 [@ id="collaboration-cards" label="协作卡"]。
- 可观测态按日志行与自身会话投影，不制造第二语义 [@ id="process-visibility" label="过程可见"]。

**来源：** 布局 [@ file="web/src/dockview/config/layout.ts" label="layout.ts"]、状态胶囊 [@ file="web/src/components/SessionStatusLine.tsx" label="SessionStatusLine.tsx"]、权限卡 [@ file="web/src/components/PermissionModal.tsx" label="PermissionModal.tsx"]。


