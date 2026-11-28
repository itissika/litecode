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

依据：[WebSearchTool.Tool.call_inner](src/tools/websearch.rs#WebSearchTool.Tool.call_inner)、[WebSearchTool.search_outcome](src/tools/websearch.rs#WebSearchTool.search_outcome)。
父节点：[tool-implementation-contract](knowledge/contracts/tools/tool-implementation-contract.md)。
相关：[webfetch-ssrf](knowledge/contracts/tools/webfetch-ssrf.md)、[tool-adapters](knowledge/contracts/tools/tool-adapters.md)。
