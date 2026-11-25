```node
node : knowledge-client-view
status : pending
summary : 知识库状态以后端快照为唯一权威；前端只投影 + 本地解析即时预览，打开面板不写盘。
x : 5832
y : 414
domain : client-ux
```

**准入：** A3。引用对错、知识节点折叠与符号解析的权威在磁盘经后端索引；客户端 `GET /api/workspace/knowledge` + `knowledge/changed` 投影，输入侧通过统一 `@` 菜单（文件/目录/合法节点）+ `#` 符号定位写入标准 Markdown 链接，`markers` 只服务 ≤16ms 打字预览与语法级 `ref_key` 对齐。前端再扫 `/tree`/`/file`/`/symbol-refs` 或自判磁盘真假，就会制造第二真相并拖垮打开时延。

依据：[fetchKnowledgeSnapshot](web/src/api/knowledge.ts#fetchKnowledgeSnapshot)、[acceptsKnowledgeVersion](web/src/lib/knowledge/snapshot.ts#acceptsKnowledgeVersion)、[pullSnapshot](web/src/stores/knowledgeStore.ts#pullSnapshot)、[openKnowledgeGraphPanel](web/src/lib/knowledge/panel.ts#openKnowledgeGraphPanel)。
上层：[ux](knowledge/framework/ux.md)。
相关：[client-projection](knowledge/contracts/client-projection.md)、[knowledge-ref-authority](knowledge/contracts/knowledge-ref-authority.md)。
