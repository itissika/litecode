```node
node : websearch
status : pending
summary : websearch：查询经提供商适配返回文本；超时与结果走公共出口。
x : 2394
y : 810
w : 480
h : 408
domain : tools-permission
```

**准入：** A1。websearch 是只读网络检索适配：参数是 query，业务是调搜索后端拿文本；不自造授权、不自造结果方言。与 webfetch 同属网络面，但没有 URL 出站门——风险模型不同，故分节点。

依据：[@ file="src/tools/websearch.rs" symbol="impl Tool for WebSearchTool › fn call_inner"]、[@ file="src/tools/websearch.rs" symbol="impl WebSearchTool › fn search"]。
父节点：[@ key="tool-implementation-contract"]。
相关：[@ key="webfetch-ssrf"]、[@ key="tool-adapters"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b）
- 只读检索适配：query → provider 文本 —— [@ file="src/tools/websearch.rs" symbol="impl Tool for WebSearchTool › fn call_inner"]、[@ file="src/tools/websearch.rs" symbol="impl WebSearchTool › fn search"]。
- 无 URL 出站门（与 webfetch 风险模型不同）—— 与节点承诺一致；超时/结果走公共 Tool 出口。
