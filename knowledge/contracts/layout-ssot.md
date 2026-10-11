```node
node : layout-ssot
status : pending
domain : client-ux
summary : 布局真源只在 Dockview；业务 store 只持内容，经 workbench 单向跟从。
```

**准入：** A1 / A3。面板「在不在、在哪」只能有一份真相——Dockview 实例经 `workbench` 门面暴露；编辑器/会话/浏览器 store 不得再维护平行 tabs 清单或用互斥 flag 双向关断。坏了会出幽灵 tab、丢订阅、popout 回主窗看不见等一串小毛病。

依据：[@ file="web/src/dockview/workbench/commands.ts" symbol="fn openPanel"]、[@ file="web/src/dockview/workbench/lifecycle.ts" symbol="fn startWorkbench"]、[@ file="web/src/dockview/workbench/commands.ts" symbol="fn movePanelToMain"]。
上层：[@ key="ux"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 布局门面 —— [@ file="web/src/dockview/workbench/commands.ts" symbol="fn openPanel"]、[@ file="web/src/dockview/workbench/commands.ts" symbol="fn movePanelToMain"]；测试见 [@ file="web/src/dockview/workbench/placement.test.ts"]。
- 内容 vs 几何：`editorStore.tabs` 只持文件内容/dirty，面板存亡经 workbench/`hasPanel` 跟从 Dockview。
