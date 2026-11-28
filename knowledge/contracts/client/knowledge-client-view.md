```node
node : knowledge-client-view
status : enabled
summary : 知识库真相在磁盘、经后端索引；前端只显示，不判断对错。
x : 5832
y : 414
domain : client-ux
```

**目标：** 一条引用对不对、一个节点存不存在，答案只能来自后端索引。前端负责显示和打字时的即时预览，不负责裁决。

**必须做到：**

- 状态按后端快照走：`GET /api/workspace/knowledge` 拉快照，`knowledge/changed` 收增量，版本对不上就整份换。[fetchKnowledgeSnapshot](web/src/api/knowledge.ts#fetchKnowledgeSnapshot)、[acceptsKnowledgeVersion](web/src/lib/knowledge/snapshot.ts#acceptsKnowledgeVersion)、[pullSnapshot](web/src/stores/knowledgeStore.ts#pullSnapshot)
- 写入走统一入口：`@` 菜单只给文件、目录、合法节点，`#` 只给符号定位，产出的都是标准 Markdown 链接。
- 本地解析只服务两件事：打字时的即时预览（≤16ms）和语法级的 `ref_key` 对齐。

**绝对不允许：**

- 前端自己扫 `/tree`、`/file`、`/symbol-refs` 去判断引用真假——那是第二真相。
- 打开面板时写盘。
- 自判磁盘状态，绕过后端快照。

后果两条：引用真假出现两个答案；打开面板的时延被前端扫描拖垮。

上层：[ux](knowledge/framework/ux.md)。相关：[client-projection](knowledge/contracts/client/client-projection.md)、[knowledge-ref-authority](knowledge/contracts/knowledge/knowledge-ref-authority.md)。
