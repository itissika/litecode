```node
node : knowledge-tool
status : pending
summary : knowledge 工具：读走常驻索引；写落盘后再刷新；不自造第二套解析。
x : 5094
y : 1170
domain : knowledge
```

**准入：** A4。agent 面的 `knowledge` 工具家族（`knowledge.list` / `knowledge.check` / `knowledge.refs` / `knowledge.create`，经 `run_code` `tools.knowledge.*` 调用）必须消费同一索引与同一状态码；其中 `list` 与 `check` 支持全部、单/多文件夹与单/多节点并集过滤，输出扁平标量行适配结构化表格渲染；写操作（`create`）改磁盘后通知刷新，不得在工具内另写一套引用判定。与其余 workspace 工具一致，工具返回的所有 `path`（及 `refs.folder`）统一为相对工作区根路径（经 `root::join` 拼接），入参 `folder` / `folders` 同时兼容工作区相对路径与知识根子目录名。它是知识契约对 agent 的出口，也是 featured-tools 里「关系可验证」的落点。

依据：[KnowledgeListTool](src/tools/knowledge.rs#KnowledgeListTool)、[KnowledgeCheckTool](src/tools/knowledge.rs#KnowledgeCheckTool)、[KnowledgeRefsTool](src/tools/knowledge.rs#KnowledgeRefsTool)、[KnowledgeCreateTool](src/tools/knowledge.rs#KnowledgeCreateTool)、[KnowledgeIndex.notify_paths](src/knowledge/index.rs#KnowledgeIndex.notify_paths)。
上层：[knowledge](knowledge/framework/knowledge.md)。
相关：[featured-tools](knowledge/framework/featured-tools.md)、[knowledge-index](knowledge/contracts/knowledge-index.md)。
