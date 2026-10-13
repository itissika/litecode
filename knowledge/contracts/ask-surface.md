```node
node : ask-surface
status : pending
summary : 人机协作卡统一走 PermissionCard：permission / approval / ask_user 分 kind，回执仍经 agent/permission。
x : 4500
y : 666
w : 470
h : 506
domain : client-ux
```

**准入：** A2 / A4。Ask 是会话级阻塞面，必须一种表面、三种 kind（Allow 系 / Approve 系 / Submit+Cancel 多题），出站仍是同一 RPC；第二套弹层会拆散投影纪律与授权回执配对。坏了会出现多会话全局闸误伤或回执对不上 request_id。

依据：[@ file="web/src/api/types.ts" symbol="interface PermissionRequest"]、[@ file="web/src/components/PermissionModal.tsx" symbol="fn PermissionCard"]、[@ file="web/src/stores/turnStore.ts" symbol="interface PendingPermission"]。
上层：[@ key="ux-interaction"]。
相关：[@ key="client-projection"]、[@ key="tool-pipeline"]、[@ key="connection-ask-registry"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 三 kind 单表面 —— [@ file="web/src/components/PermissionModal.tsx" symbol="fn PermissionCard"]；多题 [@ file="web/src/components/AskUserCard.tsx" symbol="fn AskUserCard"]。
- 回执配对 —— [@ file="web/src/stores/turnStore.ts" symbol="interface PendingPermission"]；出站 `agent/permission`；卡按 session slice。
- 跨 session WS 串行：ISSUE-019（closed，基线前已修）；协议侧见 [@ key="connection-ask-registry"]。
