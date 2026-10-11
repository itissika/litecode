```node
node : knowledge-client-view
status : pending
summary : 知识库状态以后端快照为唯一权威；前端只投影 + 本地解析即时预览，打开面板不写盘。
x : 5832
y : 414
domain : client-ux
```

**准入：** A3。引用对错、符号漂移的权威在磁盘经后端索引；客户端 `GET /api/workspace/knowledge` + `knowledge/changed` 投影，`markers` 只服务 ≤16ms 打字预览。前端再扫 `/tree`/`/file`/`/symbol-refs` 或打开面板写回旧语法，就会制造第二真相并拖垮打开时延。

依据：[@ file="web/src/api/knowledge.ts" symbol="fn fetchKnowledgeSnapshot"]、[@ file="web/src/lib/knowledge/snapshot.ts" symbol="fn acceptsKnowledgeVersion"]、[@ file="web/src/stores/knowledgeStore.ts" symbol="fn pullSnapshot"]。
上层：[@ key="ux"]。
相关：[@ key="client-projection"]、[@ key="knowledge-ref-authority"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 快照权威 —— [@ file="web/src/api/knowledge.ts" symbol="fn fetchKnowledgeSnapshot"]、[@ file="web/src/stores/knowledgeStore.ts" symbol="fn pullSnapshot"]。
- 打开不写盘：`files.ts` 已删；面板入口 [@ file="web/src/lib/knowledge/panel.ts" symbol="fn openKnowledgeGraphPanel"] 只 `openPanel`。
