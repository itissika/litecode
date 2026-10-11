```node
node : tool-control-boundary
status : enabled
domain : tools-permission
summary : 控制接缝：工具声明、政策决定、管线求值；入口、上下文、取消都不等于其效果。
x : 1494
y : 360
w : 326
h : 416
```

**主张：** 公共规则管“允许什么”，工具执行管“做了什么”，接缝处要区分几组不同概念。

- 工具声明自己的能力、参数与资源；系统与用户决定政策；管线负责求值并执行政策的结果。
- 统一入口不等于统一沙箱：入口一致，不等于工具在受控边界内运行。
- 提供上下文不等于工具消费了上下文。
- 发取消不等于底层已停止，更不等于撤销已完成的效果。

后三点是应当区分的概念，不是问题清单；本文描述理解模型，不宣称每一项当前都已实现完备。

上层：[@ key="tools"]。
相关：[@ key="tool-call-contract"]、[@ key="tool-implementation-contract"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b）
- 理解模型节点：入口≠沙箱、提供上下文≠消费、取消≠已停 —— 与 authorization / execution-context / lifecycle 子节点一致；非完备性清单。
