```node
node : collaboration-cards
status : pending
summary : 人机协作以面板内卡片承载：单次 / 始终 / 拒绝；卡片出现时 loop 阻塞等待回执。
x : 34
y : 1404
```

**agent 需要人类参与时，用一张卡收敛为一个明确选择。**

- 典型：Permission Ask → 卡片呈现 tool、规则摘要、rule_id；选项「单次放行 / 始终允许 / 拒绝」[@ file="web/src/components/PermissionModal.tsx" label="PermissionModal.tsx"]。
- 阻塞语义：卡片出现时 agent loop 阻塞等待（会话相位 `awaiting_permission`，状态灯呼吸）；回执到达前卡片保持打开 [@ file="web/src/stores/turnStore.ts" label="turnStore.ts"]。
- 形态：Agent 面板内的浮动卡（对话流之上、输入框之上），不是全屏模态、不在消息流内 [@ file="web/src/dockview/panels/AgentPanel.tsx" label="AgentPanel.tsx"]。
- 权限语义见 [@ id="permission-asymmetric" label="权限模型"]。


