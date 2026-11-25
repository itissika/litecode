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

依据：[PermissionRequest](web/src/api/types.ts#PermissionRequest)、[PermissionCard](web/src/components/PermissionModal.tsx#PermissionCard)、[PendingPermission](web/src/stores/turnStore.ts#PendingPermission)、[AskUserCard](web/src/components/AskUserCard.tsx#AskUserCard)。
上层：[ux-interaction](knowledge/ux/ux-interaction.md)。
相关：[client-projection](knowledge/contracts/client-projection.md)、[tool-pipeline](knowledge/contracts/tool-pipeline.md)、[connection-ask-registry](knowledge/contracts/client/connection-ask-registry.md)。
