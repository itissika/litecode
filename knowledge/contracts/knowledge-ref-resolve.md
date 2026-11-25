```node
node : knowledge-ref-resolve
status : pending
summary : 引用解析：路径→可读→符号/范围→越界，一引用一状态；错误码固定。
x : 4500
y : 450
domain : knowledge
```

**准入：** A1/A2。`resolve` / `validate_with` 先对知识根下的裸路径 `.md` 做节点折叠（合法节点→node 关系，坏节点/缺失→`dangling_relation`，无 node 块→普通文件），再按规范顺序校验路径→可读→点分符号锚点/带 `L` 行范围→越界，取第一个失败状态；状态码表由 `cases.json` v3 + `spec_tests` 锁定。解析顺序或状态码漂移会让徽标、提醒与工具 check 对不齐。

依据：[resolve](src/knowledge/validate.rs#resolve)、[validate_with](src/knowledge/validate.rs#validate_with)、[lookup](src/knowledge/fsref.rs#lookup)、[read_text](src/knowledge/fsref.rs#read_text)、[resolution_follows_the_spec](src/knowledge/spec_tests.rs#resolution_follows_the_spec)。
上层：[knowledge](knowledge/framework/knowledge.md)。
相关：[knowledge-ref-authority](knowledge/contracts/knowledge-ref-authority.md)、[knowledge-parse](knowledge/contracts/knowledge-parse.md)。
