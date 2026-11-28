```node
node : layout-ssot
status : enabled
summary : 面板在不在、在哪，只由 Dockview 说了算；业务 store 只管内容。
x : 5094
y : 1386
domain : client-ux
```

**目标：** 一个面板开没开、停在哪，全应用只有一个地方能回答。那个地方是 Dockview 的工作区实例，只能通过 `workbench` 门面碰。

**必须做到：**

- 布局只有一份真相：Dockview 实例，经 `workbench` 门面暴露打开、移动、弹出。[openPanel](web/src/dockview/workbench/commands.ts#openPanel)、[movePanelToMain](web/src/dockview/workbench/commands.ts#movePanelToMain)、[startWorkbench](web/src/dockview/workbench/lifecycle.ts#startWorkbench)
- 业务 store 只持内容：编辑器、会话、浏览器各管各的数据，布局一律向 `workbench` 问。
- 面板的增删移动都走同一个门面，不直接改 Dockview 实例。

**绝对不允许：**

- 再维护一份「哪些 tab 开着」的清单——两份清单必然不同步。
- 用互斥 flag 双向联动两个 store，互相开关。
- 绕过门面直接操作 Dockview 的 panel 集合。

坏了的样子是一串小毛病：幽灵 tab、丢订阅、popout 回主窗看不见。守卫在 [placement.test.ts](web/src/dockview/workbench/placement.test.ts)。

上层：[ux](knowledge/framework/ux.md)。
